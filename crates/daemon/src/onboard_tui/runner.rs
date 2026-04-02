// RatatuiOnboardRunner: core TUI runner that implements GuidedOnboardFlowStepRunner.
#![allow(clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use loongclaw_app as mvp;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use serde_json::Value;

use super::event_source::{CrosstermEventSource, OnboardEventSource};
use super::layout;
use super::theme::OnboardPalette;
use super::widgets::*;
use crate::CliResult;
use crate::migration::ProviderSelectionPlan;
use crate::onboard_finalize::{OnboardingAction, OnboardingActionKind, OnboardingSuccessSummary};
use crate::onboard_flow::{GuidedOnboardFlowStepRunner, OnboardFlowStepAction};
use crate::onboard_state::{OnboardDraft, OnboardOutcome, OnboardWizardStep};
use crate::provider_credential_policy;

const HERO_SLOGAN_LINES: [&str; 2] = ["Originated from the East,", "here to benefit the world."];
const HERO_WORDMARK_LINES: [&str; 6] = [
    "██╗      ██████╗  ██████╗ ███╗   ██╗ ██████╗  ██████╗██╗      █████╗ ██╗    ██╗",
    "██║     ██╔═══██╗██╔═══██╗████╗  ██║██╔════╝ ██╔════╝██║     ██╔══██╗██║    ██║",
    "██║     ██║   ██║██║   ██║██╔██╗ ██║██║  ███╗██║     ██║     ███████║██║ █╗ ██║",
    "██║     ██║   ██║██║   ██║██║╚██╗██║██║   ██║██║     ██║     ██╔══██║██║███╗██║",
    "███████╗╚██████╔╝╚██████╔╝██║ ╚████║╚██████╔╝╚██████╗███████╗██║  ██║╚███╔███╔╝",
    "╚══════╝ ╚═════╝  ╚═════╝ ╚═╝  ╚═══╝ ╚═════╝  ╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝",
];
const HERO_STACKED_WORDMARK_LINES: [&str; 13] = [
    "██╗      ██████╗  ██████╗ ███╗   ██╗ ██████╗ ",
    "██║     ██╔═══██╗██╔═══██╗████╗  ██║██╔════╝ ",
    "██║     ██║   ██║██║   ██║██╔██╗ ██║██║  ███╗",
    "██║     ██║   ██║██║   ██║██║╚██╗██║██║   ██║",
    "███████╗╚██████╔╝╚██████╔╝██║ ╚████║╚██████╔╝",
    "╚══════╝ ╚═════╝  ╚═════╝ ╚═╝  ╚═══╝ ╚═════╝ ",
    "",
    " ██████╗██╗      █████╗ ██╗    ██╗           ",
    "██╔════╝██║     ██╔══██╗██║    ██║           ",
    "██║     ██║     ███████║██║ █╗ ██║           ",
    "██║     ██║     ██╔══██║██║███╗██║           ",
    "╚██████╗███████╗██║  ██║╚███╔███╔╝           ",
    " ╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝            ",
];
const HERO_INTRO_LINES: [&str; 2] = [
    "We found a reusable setup on this machine.",
    "Choose the fastest way to first success.",
];
const MIN_FULL_SCREEN_WIDTH: u16 = 72;
const MIN_FULL_SCREEN_HEIGHT: u16 = 22;
const INLINE_LOGO_MIN_WIDTH: u16 = 14;
const INLINE_LOGO_MIN_HEIGHT: u16 = 6;
#[allow(dead_code)]
const INLINE_ICON_MIN_WIDTH: u16 = 8;
#[allow(dead_code)]
const INLINE_ICON_MIN_HEIGHT: u16 = 4;
// ---------------------------------------------------------------------------
// Loop result types
// ---------------------------------------------------------------------------

enum SelectionLoopResult {
    Selected(usize),
    Back,
}

enum MultiSelectionLoopResult {
    Submitted(Vec<usize>),
    Back,
}

enum InputLoopResult {
    Submitted(String),
    Back,
}

enum ProviderConfigurationLoopResult {
    Configured(Box<mvp::config::ProviderConfig>),
    Back,
}

enum StandaloneSelectionResult {
    Selected(usize),
    Cancel,
}

type OpenaiCodexOauthStartFn =
    fn() -> CliResult<Box<dyn crate::openai_codex_oauth::OpenaiCodexOauthFlow>>;

enum OpenaiCodexOauthLoopResult {
    Authorized(crate::openai_codex_oauth::OpenaiCodexOauthGrant),
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShowcaseStageVariant {
    EntryPath,
    DetectedStartingPoint,
    ShortcutChoice,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LaunchDeckResult {
    pub focused_action: Option<usize>,
    pub open_chat: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineLogoProtocol {
    Kitty,
    Iterm2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineLogoTerminal {
    Kitty,
    Iterm2,
    WezTerm,
    Ghostty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InlineLogoSupport {
    terminal: InlineLogoTerminal,
    protocol: InlineLogoProtocol,
    tmux_passthrough: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct InlineLogoEnvironment {
    term: String,
    term_program: Option<String>,
    inside_tmux: bool,
    tmux_passthrough_allowed: bool,
    kitty_window_id_present: bool,
    wezterm_executable_present: bool,
    ghostty_resources_dir_present: bool,
    inline_logo_disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelpOverlayTopic {
    Welcome,
    Selection,
    Input,
    Showcase,
    Confirm,
    Info,
    VerifyWrite,
    Launch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BrandMarkTag {
    label: &'static str,
    color: Color,
}

#[derive(Clone, Debug)]
struct ProviderPickerOption {
    profile_id: String,
    kind: mvp::config::ProviderKind,
    item: SelectionItem,
    preview: mvp::config::ProviderConfig,
}

#[derive(Clone, Debug)]
struct WebSearchPickerOption {
    id: &'static str,
    item: SelectionItem,
    requires_api_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChannelPairingPrompt {
    field_key: &'static str,
    label: String,
    default_value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnboardAcpBackendOption {
    id: String,
    summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BrandStageMetrics {
    column_width: u16,
    wordmark_height: u16,
    primary_height: u16,
    support_height: u16,
    gap_before_logo: u16,
    gap_after_logo: u16,
    gap_before_support: u16,
    desired_height: u16,
}

// ---------------------------------------------------------------------------
// RatatuiOnboardRunner
// ---------------------------------------------------------------------------

pub(crate) struct RatatuiOnboardRunner<E: OnboardEventSource = CrosstermEventSource> {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    event_source: E,
    owns_tty: bool,
    provider_selection_plan: ProviderSelectionPlan,
    openai_codex_oauth_start: OpenaiCodexOauthStartFn,
    skip_next_guided_welcome: bool,
}

impl RatatuiOnboardRunner<CrosstermEventSource> {
    /// Create a new runner that takes over the terminal for the full
    /// onboarding session.
    pub fn new() -> io::Result<Self> {
        // Install a panic hook that restores the terminal before printing.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = Self::restore_tty();
            original_hook(info);
        }));

        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        terminal::enable_raw_mode()?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        Ok(Self {
            terminal,
            event_source: CrosstermEventSource,
            owns_tty: true,
            provider_selection_plan: ProviderSelectionPlan::default(),
            openai_codex_oauth_start: crate::openai_codex_oauth::start_openai_codex_oauth_flow,
            skip_next_guided_welcome: false,
        })
    }
}

impl<E: OnboardEventSource> RatatuiOnboardRunner<E> {
    fn restore_tty() -> io::Result<()> {
        terminal::disable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, Show, LeaveAlternateScreen)?;
        stdout.flush()
    }

    /// Create a runner without touching the real terminal.
    ///
    /// Used in tests where raw-mode is unavailable.
    #[cfg(test)]
    fn headless(event_source: E) -> io::Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 80, 24)),
            },
        )?;
        Ok(Self {
            terminal,
            event_source,
            owns_tty: false,
            provider_selection_plan: ProviderSelectionPlan::default(),
            openai_codex_oauth_start: crate::openai_codex_oauth::start_openai_codex_oauth_flow,
            skip_next_guided_welcome: false,
        })
    }

    pub(crate) fn set_provider_selection_plan(&mut self, plan: ProviderSelectionPlan) {
        self.provider_selection_plan = plan;
    }

    pub(crate) fn run_opening_screen(&mut self) -> CliResult<()> {
        let result = self.run_welcome_step()?;
        match result {
            OnboardFlowStepAction::Next | OnboardFlowStepAction::Skip => Ok(()),
            OnboardFlowStepAction::Back => {
                Err("welcome screen unexpectedly returned back".to_owned())
            }
        }
    }

    pub(crate) fn skip_next_guided_welcome_once(&mut self) {
        self.skip_next_guided_welcome = true;
    }

    #[cfg(test)]
    fn set_openai_codex_oauth_start(&mut self, start: OpenaiCodexOauthStartFn) {
        self.openai_codex_oauth_start = start;
    }

    // -----------------------------------------------------------------------
    // Terminal size guard
    // -----------------------------------------------------------------------

    /// Returns `true` when the terminal area is too small to render dialog
    /// boxes without garbled output.
    fn is_terminal_too_small(area: Rect) -> bool {
        area.width < MIN_FULL_SCREEN_WIDTH || area.height < MIN_FULL_SCREEN_HEIGHT
    }

    /// Render a minimal "resize your terminal" fallback message.
    fn render_too_small_fallback(frame: &mut ratatui::Frame<'_>) {
        let msg = Paragraph::new(format!(
            "Terminal too small.\nResize to at least {}x{}.",
            MIN_FULL_SCREEN_WIDTH, MIN_FULL_SCREEN_HEIGHT
        ))
        .alignment(Alignment::Center);
        frame.render_widget(msg, frame.area());
    }

    fn brand_wordmark_lines(width: u16) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let widest = HERO_WORDMARK_LINES
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        if widest > usize::from(width) {
            return Vec::new();
        }

        HERO_WORDMARK_LINES
            .iter()
            .map(|line| {
                if line.is_empty() {
                    Line::from("")
                } else {
                    Line::from(Span::styled(
                        (*line).to_owned(),
                        Style::default()
                            .fg(palette.brand)
                            .add_modifier(Modifier::BOLD),
                    ))
                }
            })
            .collect()
    }

    fn welcome_hero_wordmark_lines(width: u16) -> Vec<Line<'static>> {
        Self::brand_ascii_logo_lines(width, u16::MAX)
    }

    fn brand_stacked_wordmark_lines(width: u16) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let widest = HERO_STACKED_WORDMARK_LINES
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        if widest > usize::from(width) {
            return Vec::new();
        }

        HERO_STACKED_WORDMARK_LINES
            .iter()
            .map(|line| {
                if line.is_empty() {
                    return Line::from("");
                }

                Line::from(Span::styled(
                    (*line).to_owned(),
                    Style::default()
                        .fg(palette.brand)
                        .add_modifier(Modifier::BOLD),
                ))
            })
            .collect()
    }

    fn brand_stage_column_width(content_width: u16, minimum_width: u16) -> u16 {
        let full_wordmark_width = u16::try_from(
            HERO_WORDMARK_LINES
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0),
        )
        .ok()
        .unwrap_or(0);
        let stage_width = content_width.min(full_wordmark_width);
        let minimum_stage_width = minimum_width.min(content_width.max(1));

        stage_width.max(minimum_stage_width)
    }

    fn preferred_welcome_logo_height(width: u16) -> u16 {
        Self::preferred_brand_logo_height(width)
    }

    fn guided_brand_stage_metrics(
        content_width: u16,
        primary_lines: &[Line<'static>],
        support_lines: &[Line<'static>],
    ) -> BrandStageMetrics {
        let column_width = Self::brand_stage_column_width(content_width, 44);
        let wordmark_height = Self::preferred_brand_logo_height(column_width);
        let primary_height = u16::try_from(primary_lines.len()).ok().unwrap_or(0);
        let support_height = u16::try_from(support_lines.len()).ok().unwrap_or(0);
        let gap_before_logo = 1;
        let gap_after_logo = if primary_height > 0 { 1 } else { 0 };
        let gap_before_support = if support_height > 0 { 1 } else { 0 };
        let minimum_height = wordmark_height
            .saturating_add(gap_before_logo)
            .saturating_add(primary_height)
            .saturating_add(support_height);
        let desired_height = minimum_height
            .saturating_add(gap_after_logo)
            .saturating_add(gap_before_support);

        BrandStageMetrics {
            column_width,
            wordmark_height,
            primary_height,
            support_height,
            gap_before_logo,
            gap_after_logo,
            gap_before_support,
            desired_height,
        }
    }

    fn guided_brand_stage_desired_height(
        content_width: u16,
        primary_lines: &[Line<'static>],
        support_lines: &[Line<'static>],
    ) -> u16 {
        let metrics = Self::guided_brand_stage_metrics(content_width, primary_lines, support_lines);
        metrics.desired_height
    }

    fn focus_selection_panel_height(available_height: u16, item_count: usize) -> u16 {
        let lines_per_item = 3_u16;
        let max_visible_items = available_height / lines_per_item;
        let max_visible_items = max_visible_items.max(1);
        let max_visible_items = usize::from(max_visible_items);
        let visible_items = item_count.clamp(1, max_visible_items);

        let visible_items_u16 = u16::try_from(visible_items).ok().unwrap_or(1);
        visible_items_u16.saturating_mul(lines_per_item)
    }

    fn selection_shell_sections(
        content_area: Rect,
        stage_lines: &[Line<'static>],
        item_count: usize,
    ) -> std::rc::Rc<[Rect]> {
        let desired_stage_height =
            Self::guided_brand_stage_desired_height(content_area.width, stage_lines, &[]);
        let desired_selection_height =
            Self::focus_selection_panel_height(content_area.height, item_count);
        let minimum_selection_height = Self::focus_selection_panel_height(3, item_count);
        let total_desired_height = desired_stage_height.saturating_add(desired_selection_height);

        if total_desired_height <= content_area.height {
            return Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(desired_stage_height),
                    Constraint::Length(desired_selection_height),
                    Constraint::Min(0),
                ])
                .split(content_area);
        }

        let remaining_selection_height = content_area.height.saturating_sub(desired_stage_height);
        let capped_selection_height = remaining_selection_height.max(minimum_selection_height);
        let max_selection_height = content_area.height.saturating_sub(1).max(1);
        let selection_height = capped_selection_height.min(max_selection_height);
        let stage_height = content_area.height.saturating_sub(selection_height).max(1);

        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(stage_height),
                Constraint::Length(selection_height),
                Constraint::Min(0),
            ])
            .split(content_area)
    }

    fn preferred_brand_logo_height(width: u16) -> u16 {
        let full = Self::brand_wordmark_lines(width);
        if !full.is_empty() {
            return u16::try_from(full.len()).ok().unwrap_or(6);
        }

        let stacked = Self::brand_stacked_wordmark_lines(width);
        if !stacked.is_empty() {
            return u16::try_from(stacked.len()).ok().unwrap_or(13);
        }

        1
    }

    fn brand_ascii_logo_lines(width: u16, _height: u16) -> Vec<Line<'static>> {
        let full = Self::brand_wordmark_lines(width);
        if !full.is_empty() {
            return full;
        }

        let stacked = Self::brand_stacked_wordmark_lines(width);
        if !stacked.is_empty() {
            return stacked;
        }

        let palette = Self::palette();
        vec![Line::from(Span::styled(
            "LOONGCLAW",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ))]
    }

    fn screen_header_line(title: &str, hint: &str) -> Line<'static> {
        let palette = Self::palette();
        Line::from(vec![
            Span::styled(
                " LOONGCLAW ",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {title} "), Style::default().fg(palette.text)),
            Span::raw(" "),
            Span::styled(hint.to_owned(), Style::default().fg(palette.muted_text)),
        ])
    }

    fn guided_footer_line(step_number: usize, footer_hint: &str, width: u16) -> Line<'static> {
        let palette = Self::palette();
        let hint = format!(" {footer_hint} ");
        let progress = format!(" {step_number}/{} ", total_step_count());
        let spacer_width =
            usize::from(width).saturating_sub(hint.chars().count() + progress.chars().count());
        Line::from(vec![
            Span::styled(hint, Style::default().fg(palette.secondary_text)),
            Span::raw(" ".repeat(spacer_width)),
            Span::styled(progress, Style::default().fg(palette.muted_text)),
        ])
    }

    fn shell_footer_line(footer_hint: &str) -> Line<'static> {
        let palette = Self::palette();
        Line::from(vec![Span::styled(
            format!(" {footer_hint} "),
            Style::default().fg(palette.secondary_text),
        )])
    }

    fn badge_span(label: impl Into<String>, fg: Color, bg: Color) -> Span<'static> {
        Span::styled(
            format!(" {} ", label.into()),
            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        )
    }

    fn badge_text_color(bg: Color) -> Color {
        match bg {
            Color::Reset => Color::Reset,
            _ if Self::color_is_light(bg) => Color::Black,
            Color::Black
            | Color::Red
            | Color::Green
            | Color::Yellow
            | Color::Blue
            | Color::Magenta
            | Color::Cyan
            | Color::Gray
            | Color::DarkGray
            | Color::LightRed
            | Color::LightGreen
            | Color::LightYellow
            | Color::LightBlue
            | Color::LightMagenta
            | Color::LightCyan
            | Color::White
            | Color::Rgb(_, _, _)
            | Color::Indexed(_) => Color::White,
        }
    }

    fn filled_badge_span(label: impl Into<String>, bg: Color) -> Span<'static> {
        Self::badge_span(label, Self::badge_text_color(bg), bg)
    }

    fn badge_lines<I>(badges: I, max_width: u16) -> Vec<Line<'static>>
    where
        I: IntoIterator<Item = Span<'static>>,
    {
        let max_width = usize::from(max_width.max(12));
        let mut lines = Vec::new();
        let mut current = Vec::new();
        let mut current_width = 0usize;

        for badge in badges {
            let badge_width = badge.content.chars().count();
            let separator_width = if current.is_empty() { 0 } else { 1 };
            if !current.is_empty() && current_width + separator_width + badge_width > max_width {
                lines.push(Line::from(current));
                current = Vec::new();
                current_width = 0;
            }
            if !current.is_empty() {
                current.push(Span::raw(" "));
                current_width += 1;
            }
            current_width += badge_width;
            current.push(badge);
        }

        if !current.is_empty() {
            lines.push(Line::from(current));
        }

        lines
    }

    fn palette() -> OnboardPalette {
        OnboardPalette::current()
    }

    fn color_is_light(color: Color) -> bool {
        let (red, green, blue) = match color {
            Color::Reset => return false,
            Color::Black => (0, 0, 0),
            Color::Red => (205, 49, 49),
            Color::Green => (13, 188, 121),
            Color::Yellow => (229, 229, 16),
            Color::Blue => (36, 114, 200),
            Color::Magenta => (188, 63, 188),
            Color::Cyan => (17, 168, 205),
            Color::Gray => (229, 229, 229),
            Color::DarkGray => (102, 102, 102),
            Color::LightRed => (241, 76, 76),
            Color::LightGreen => (35, 209, 139),
            Color::LightYellow => (245, 245, 67),
            Color::LightBlue => (59, 142, 234),
            Color::LightMagenta => (214, 112, 214),
            Color::LightCyan => (41, 184, 219),
            Color::White => (255, 255, 255),
            Color::Rgb(red, green, blue) => (red, green, blue),
            Color::Indexed(index) => match index {
                0..=15 => match index {
                    0 => (0, 0, 0),
                    1 => (205, 49, 49),
                    2 => (13, 188, 121),
                    3 => (229, 229, 16),
                    4 => (36, 114, 200),
                    5 => (188, 63, 188),
                    6 => (17, 168, 205),
                    7 => (229, 229, 229),
                    8 => (102, 102, 102),
                    9 => (241, 76, 76),
                    10 => (35, 209, 139),
                    11 => (245, 245, 67),
                    12 => (59, 142, 234),
                    13 => (214, 112, 214),
                    14 => (41, 184, 219),
                    _ => (255, 255, 255),
                },
                16..=231 => {
                    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
                    let adjusted = index - 16;
                    let red_index = usize::from(adjusted / 36);
                    let green_index = usize::from((adjusted % 36) / 6);
                    let blue_index = usize::from(adjusted % 6);
                    let red = LEVELS.get(red_index).copied().unwrap_or(0);
                    let green = LEVELS.get(green_index).copied().unwrap_or(0);
                    let blue = LEVELS.get(blue_index).copied().unwrap_or(0);
                    (red, green, blue)
                }
                232..=255 => {
                    let gray = 8 + (index - 232) * 10;
                    (gray, gray, gray)
                }
            },
        };
        (u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114) / 1000 >= 150
    }

    fn stable_selection_theme() -> SelectionCardTheme {
        let palette = Self::palette();
        SelectionCardTheme::new(palette.brand, palette.brand, palette.surface_emphasis)
    }

    fn panel_fill_style(border_color: Color) -> Style {
        let palette = Self::palette();
        let background = if border_color == palette.brand {
            palette.surface_emphasis
        } else if border_color == palette.info {
            palette.info_surface
        } else if border_color == palette.success {
            palette.success_surface
        } else if border_color == palette.warning {
            palette.warning_surface
        } else if border_color == palette.error {
            palette.error_surface
        } else {
            palette.surface
        };
        Style::default().bg(background)
    }

    fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
        let width = width.min(area.width).max(1);
        let height = height.min(area.height).max(1);
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        Rect::new(x, y, width, height)
    }

    fn inset_rect(area: Rect, horizontal: u16, vertical: u16) -> Rect {
        let doubled_horizontal = horizontal.saturating_mul(2);
        let doubled_vertical = vertical.saturating_mul(2);
        let inset_width = area.width.saturating_sub(doubled_horizontal).max(1);
        let inset_height = area.height.saturating_sub(doubled_vertical).max(1);
        let inset_x = area.x.saturating_add(horizontal);
        let inset_y = area.y.saturating_add(vertical);

        Rect::new(inset_x, inset_y, inset_width, inset_height)
    }

    fn shell_content_area(content_area: Rect) -> Rect {
        if content_area.height <= 1 {
            return content_area;
        }

        let shell = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(content_area);
        shell[1]
    }

    fn showcase_shell_sections(area: Rect) -> std::rc::Rc<[Rect]> {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(17),
                Constraint::Length(2),
            ])
            .split(area)
    }

    fn rounded_panel(title: &str, border_color: Color) -> Block<'static> {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color));
        if title.trim().is_empty() {
            block
        } else {
            block.title(Span::styled(
                format!(" {title} "),
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::BOLD),
            ))
        }
    }

    fn render_panel_lines_with_fill(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        title: &str,
        border_color: Color,
        lines: Vec<Line<'static>>,
        alignment: Alignment,
        fill_style: Option<Style>,
    ) {
        let block = if let Some(style) = fill_style {
            Self::rounded_panel(title, border_color).style(style)
        } else {
            Self::rounded_panel(title, border_color)
        };
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let paragraph = if let Some(style) = fill_style {
            Paragraph::new(lines)
                .alignment(alignment)
                .wrap(Wrap { trim: false })
                .style(style)
        } else {
            Paragraph::new(lines)
                .alignment(alignment)
                .wrap(Wrap { trim: false })
        };
        frame.render_widget(paragraph, inner);
    }

    #[allow(dead_code)] // retained for incremental panel migrations
    fn render_panel_lines(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        title: &str,
        border_color: Color,
        lines: Vec<Line<'static>>,
        alignment: Alignment,
    ) {
        Self::render_panel_lines_with_fill(
            frame,
            area,
            title,
            border_color,
            lines,
            alignment,
            None,
        );
    }

    fn render_scrollable_panel(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        title: &str,
        accent_color: Color,
        active: bool,
        lines: &[Line<'static>],
        scroll_offset: u16,
        alignment: Alignment,
    ) -> u16 {
        let palette = Self::palette();
        let border_color = if active { accent_color } else { palette.border };
        let fill_style = active.then(|| Self::panel_fill_style(accent_color));
        let block = if let Some(style) = fill_style {
            Self::rounded_panel(title, border_color).style(style)
        } else {
            Self::rounded_panel(title, border_color)
        };
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let visible_height = inner.height;
        let visible_lines = lines
            .iter()
            .skip(usize::from(scroll_offset))
            .take(usize::from(visible_height))
            .cloned()
            .collect::<Vec<_>>();
        let paragraph = if let Some(style) = fill_style {
            Paragraph::new(visible_lines)
                .alignment(alignment)
                .wrap(Wrap { trim: false })
                .style(style)
        } else {
            Paragraph::new(visible_lines)
                .alignment(alignment)
                .wrap(Wrap { trim: false })
        };
        frame.render_widget(paragraph, inner);

        visible_height
    }

    fn scroll_forward(offset: &mut u16, visible_height: u16, total_lines: usize) {
        if usize::from(*offset) + usize::from(visible_height) < total_lines {
            *offset += 1;
        }
    }

    fn scroll_backward(offset: &mut u16) {
        *offset = offset.saturating_sub(1);
    }

    fn max_scroll_offset(visible_height: u16, total_lines: usize) -> u16 {
        total_lines.saturating_sub(usize::from(visible_height)) as u16
    }

    fn scroll_page_forward(offset: &mut u16, visible_height: u16, total_lines: usize) {
        let next = offset.saturating_add(visible_height.saturating_sub(1).max(1));
        *offset = next.min(Self::max_scroll_offset(visible_height, total_lines));
    }

    fn scroll_page_backward(offset: &mut u16, visible_height: u16) {
        *offset = offset.saturating_sub(visible_height.saturating_sub(1).max(1));
    }

    fn scroll_to_end(offset: &mut u16, visible_height: u16, total_lines: usize) {
        *offset = Self::max_scroll_offset(visible_height, total_lines);
    }

    fn current_inline_logo_env(&self) -> InlineLogoEnvironment {
        InlineLogoEnvironment {
            term: std::env::var("TERM").unwrap_or_default(),
            term_program: std::env::var("TERM_PROGRAM").ok(),
            inside_tmux: std::env::var_os("TMUX").is_some(),
            tmux_passthrough_allowed: Self::tmux_passthrough_allowed(),
            kitty_window_id_present: std::env::var_os("KITTY_WINDOW_ID").is_some(),
            wezterm_executable_present: std::env::var_os("WEZTERM_EXECUTABLE").is_some(),
            ghostty_resources_dir_present: std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some(),
            inline_logo_disabled: std::env::var_os("LOONGCLAW_ONBOARD_NO_INLINE_LOGO").is_some(),
        }
    }

    fn tmux_passthrough_allowed() -> bool {
        static TMUX_PASSTHROUGH_ALLOWED: OnceLock<bool> = OnceLock::new();
        *TMUX_PASSTHROUGH_ALLOWED.get_or_init(|| {
            if std::env::var_os("TMUX").is_none() {
                return false;
            }

            Command::new("tmux")
                .args(["show-options", "-gv", "allow-passthrough"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|value| matches!(value.trim(), "on" | "all"))
                .unwrap_or(false)
        })
    }

    fn detect_inline_logo_support_for_env(
        env: &InlineLogoEnvironment,
    ) -> Option<InlineLogoSupport> {
        if env.inline_logo_disabled {
            return None;
        }

        let base = if env.kitty_window_id_present || env.term.contains("kitty") {
            Some(InlineLogoSupport {
                terminal: InlineLogoTerminal::Kitty,
                protocol: InlineLogoProtocol::Kitty,
                tmux_passthrough: env.inside_tmux,
            })
        } else if matches!(env.term_program.as_deref(), Some("iTerm.app")) {
            Some(InlineLogoSupport {
                terminal: InlineLogoTerminal::Iterm2,
                protocol: InlineLogoProtocol::Iterm2,
                tmux_passthrough: env.inside_tmux,
            })
        } else if matches!(env.term_program.as_deref(), Some("WezTerm"))
            || env.wezterm_executable_present
            || env.term.contains("wezterm")
        {
            Some(InlineLogoSupport {
                terminal: InlineLogoTerminal::WezTerm,
                protocol: if env.inside_tmux {
                    InlineLogoProtocol::Kitty
                } else {
                    InlineLogoProtocol::Iterm2
                },
                tmux_passthrough: env.inside_tmux,
            })
        } else if matches!(env.term_program.as_deref(), Some("ghostty"))
            || env.ghostty_resources_dir_present
            || env.term.contains("ghostty")
        {
            Some(InlineLogoSupport {
                terminal: InlineLogoTerminal::Ghostty,
                protocol: InlineLogoProtocol::Kitty,
                tmux_passthrough: env.inside_tmux,
            })
        } else {
            None
        }?;

        if env.inside_tmux && !env.tmux_passthrough_allowed {
            return None;
        }

        Some(base)
    }

    fn inline_logo_support(&self) -> Option<InlineLogoSupport> {
        if !self.owns_tty {
            return None;
        }
        let env = self.current_inline_logo_env();
        Self::detect_inline_logo_support_for_env(&env)
    }

    fn inline_logo_protocol(&self) -> Option<InlineLogoProtocol> {
        self.inline_logo_support().map(|support| support.protocol)
    }

    fn brand_mark_caption_lines(tags: &[BrandMarkTag], width: u16) -> Vec<Line<'static>> {
        Self::badge_lines(
            tags.iter()
                .map(|tag| Self::filled_badge_span(tag.label, tag.color))
                .collect::<Vec<_>>(),
            width,
        )
    }

    fn brand_mark_story_lines(image_transport_ready: bool, width: u16) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let mut lines = Vec::new();
        if width >= 34 {
            let route_copy = if image_transport_ready {
                "Inline logo active when the terminal supports image transport."
            } else {
                "Banner fallback active. The layout stays the same even without image transport."
            };
            lines.push(Line::from(Span::styled(
                route_copy,
                Style::default().fg(palette.muted_text),
            )));
        }
        lines
    }

    fn brand_mark_footer_lines(
        tags: &[BrandMarkTag],
        image_transport_ready: bool,
        width: u16,
    ) -> Vec<Line<'static>> {
        let mut lines = Self::brand_mark_story_lines(image_transport_ready, width);
        let caption_lines = Self::brand_mark_caption_lines(tags, width);
        if !caption_lines.is_empty() {
            lines.push(Line::from(""));
            lines.extend(caption_lines);
        }
        lines
    }

    fn brand_mark_fallback_lines(
        tags: &[BrandMarkTag],
        image_transport_ready: bool,
        width: u16,
    ) -> Vec<Line<'static>> {
        let mut lines = Self::brand_ascii_logo_lines(width, 6);
        let footer_lines = Self::brand_mark_footer_lines(tags, image_transport_ready, width);
        if !footer_lines.is_empty() {
            lines.push(Line::from(""));
            lines.extend(footer_lines);
        }
        lines
    }

    fn welcome_support_lines_for_route(_render_route_copy: &str) -> Vec<Line<'static>> {
        let palette = Self::palette();
        vec![
            Line::from(Span::styled(
                "Nothing is written until Verify & Write.",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                "Safe to rerun whenever your setup changes.",
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    fn welcome_version_line(version: &str) -> Line<'static> {
        Line::from(Span::styled(
            version.to_owned(),
            Style::default().fg(Self::palette().muted_text),
        ))
    }

    fn welcome_primary_lines() -> Vec<Line<'static>> {
        let palette = Self::palette();
        vec![
            Line::from(Span::styled(
                HERO_SLOGAN_LINES[0],
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(Span::styled(
                HERO_SLOGAN_LINES[1],
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "A focused full-screen deck for first setup",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                "and every deliberate tune-up that follows.",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to begin.",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
        ]
    }

    #[cfg(test)]
    fn hero_slogan_lines() -> Vec<Line<'static>> {
        let palette = Self::palette();
        vec![
            Line::from(Span::styled(
                HERO_SLOGAN_LINES[0],
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(Span::styled(
                HERO_SLOGAN_LINES[1],
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::ITALIC),
            )),
        ]
    }

    #[cfg(test)]
    fn risk_stage_lines() -> Vec<Line<'static>> {
        let palette = Self::palette();
        vec![
            Line::from(Span::styled(
                "Security Check",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Confirm the trust boundary on this machine before local files or tools are touched.",
                Style::default().fg(palette.secondary_text),
            )),
        ]
    }

    #[cfg(test)]
    fn risk_gate_lines() -> Vec<Line<'static>> {
        let palette = Self::palette();
        vec![
            Line::from(Span::styled(
                "Before this deck reads local files or invokes tools,",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                "confirm the trust boundary on this machine.",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "• May read local files and call local tools.",
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                "• Keep credentials in env vars, not prompts.",
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                "• Start with the narrowest tool and file scope.",
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                "Enter or y continues. n or Esc cancels before any config is written.",
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    #[cfg(test)]
    fn render_risk_centerpiece(
        frame: &mut ratatui::Frame<'_>,
        content_area: Rect,
        _version: &str,
        risk_lines: &[Line<'static>],
    ) {
        let stage_lines = Self::risk_stage_lines();
        let stage_height =
            Self::guided_brand_stage_desired_height(content_area.width, &stage_lines, &[]);
        let shell = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(stage_height), Constraint::Min(0)])
            .split(content_area);

        Self::render_guided_brand_stage(
            frame,
            shell[0],
            &stage_lines,
            &[],
            Alignment::Center,
            Alignment::Center,
            74,
            0,
        );

        let body_area = Self::centered_rect(
            shell[1],
            shell[1].width.saturating_sub(8).min(74),
            shell[1].height,
        );
        frame.render_widget(
            Paragraph::new(risk_lines.to_vec())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Self::palette().secondary_text)),
            body_area,
        );
    }

    fn render_welcome_centerpiece(
        frame: &mut ratatui::Frame<'_>,
        content_area: Rect,
        version: &str,
        primary_lines: &[Line<'static>],
        support_lines: &[Line<'static>],
        inline_logo: bool,
    ) -> Option<Rect> {
        Self::render_open_brand_stage(
            frame,
            content_area,
            version,
            primary_lines,
            support_lines,
            inline_logo,
            Alignment::Center,
            Alignment::Center,
            76,
            70,
        )
    }

    fn render_open_brand_stage(
        frame: &mut ratatui::Frame<'_>,
        content_area: Rect,
        version: &str,
        primary_lines: &[Line<'static>],
        support_lines: &[Line<'static>],
        inline_logo: bool,
        primary_alignment: Alignment,
        support_alignment: Alignment,
        primary_max_width: u16,
        support_max_width: u16,
    ) -> Option<Rect> {
        let column_width = Self::brand_stage_column_width(content_area.width, 48);
        let wordmark_height = Self::preferred_welcome_logo_height(column_width);
        let primary_height = u16::try_from(primary_lines.len()).ok().unwrap_or(0);
        let support_height = u16::try_from(support_lines.len()).ok().unwrap_or(0);
        let column_height = 1u16
            .saturating_add(1)
            .saturating_add(wordmark_height)
            .saturating_add(1)
            .saturating_add(primary_height)
            .saturating_add(if support_height > 0 {
                1 + support_height
            } else {
                0
            })
            .min(content_area.height.max(12));
        let top_padding = content_area.height.saturating_sub(column_height) / 2;
        let stage = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(top_padding),
                Constraint::Length(column_height),
                Constraint::Min(0),
            ])
            .split(content_area);
        let column_area = Self::centered_rect(stage[1], column_width, stage[1].height);
        let rows = if support_height > 0 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(wordmark_height),
                    Constraint::Length(1),
                    Constraint::Length(primary_height),
                    Constraint::Length(1),
                    Constraint::Length(support_height),
                ])
                .split(column_area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(wordmark_height),
                    Constraint::Length(1),
                    Constraint::Length(primary_height),
                ])
                .split(column_area)
        };

        frame.render_widget(
            Paragraph::new(vec![Self::welcome_version_line(version)]).alignment(Alignment::Center),
            rows[0],
        );

        let wordmark_area = rows[2];
        let captured_logo_area =
            Self::render_welcome_brand_media(frame, wordmark_area, inline_logo, Style::default());

        let primary_area = Self::centered_rect(
            rows[4],
            rows[4].width.min(primary_max_width),
            rows[4].height,
        );
        frame.render_widget(
            Paragraph::new(primary_lines.to_vec())
                .alignment(primary_alignment)
                .wrap(Wrap { trim: false }),
            primary_area,
        );

        if support_height > 0 {
            let support_area = Self::centered_rect(
                rows[6],
                rows[6].width.min(support_max_width),
                rows[6].height,
            );
            frame.render_widget(
                Paragraph::new(support_lines.to_vec())
                    .alignment(support_alignment)
                    .wrap(Wrap { trim: false }),
                support_area,
            );
        }

        captured_logo_area
    }

    fn render_guided_brand_stage(
        frame: &mut ratatui::Frame<'_>,
        content_area: Rect,
        primary_lines: &[Line<'static>],
        support_lines: &[Line<'static>],
        primary_alignment: Alignment,
        support_alignment: Alignment,
        primary_max_width: u16,
        support_max_width: u16,
    ) {
        let mut metrics =
            Self::guided_brand_stage_metrics(content_area.width, primary_lines, support_lines);
        let mut desired_height = metrics.desired_height;
        let available_height = content_area.height;

        if desired_height > available_height && metrics.gap_before_logo > 0 {
            metrics.gap_before_logo = 0;
            desired_height = desired_height.saturating_sub(1);
        }
        if desired_height > available_height && metrics.gap_after_logo > 0 {
            metrics.gap_after_logo = 0;
            desired_height = desired_height.saturating_sub(1);
        }
        if desired_height > available_height && metrics.gap_before_support > 0 {
            metrics.gap_before_support = 0;
            desired_height = desired_height.saturating_sub(1);
        }

        let column_height = desired_height.min(available_height.max(1));
        let top_padding = content_area.height.saturating_sub(column_height) / 2;
        let stage = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(top_padding),
                Constraint::Length(column_height),
                Constraint::Min(0),
            ])
            .split(content_area);
        let column_area = Self::centered_rect(stage[1], metrics.column_width, stage[1].height);
        let rows = if metrics.support_height > 0 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(metrics.gap_before_logo),
                    Constraint::Length(metrics.wordmark_height),
                    Constraint::Length(metrics.gap_after_logo),
                    Constraint::Length(metrics.primary_height),
                    Constraint::Length(metrics.gap_before_support),
                    Constraint::Length(metrics.support_height),
                ])
                .split(column_area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(metrics.gap_before_logo),
                    Constraint::Length(metrics.wordmark_height),
                    Constraint::Length(metrics.gap_after_logo),
                    Constraint::Length(metrics.primary_height),
                ])
                .split(column_area)
        };

        let wordmark_area = rows[1];
        Self::render_guided_brand_media(frame, wordmark_area, Style::default());

        let primary_area = Self::centered_rect(
            rows[3],
            rows[3].width.min(primary_max_width),
            rows[3].height,
        );
        frame.render_widget(
            Paragraph::new(primary_lines.to_vec())
                .alignment(primary_alignment)
                .wrap(Wrap { trim: false }),
            primary_area,
        );

        if metrics.support_height > 0 {
            let support_area = Self::centered_rect(
                rows[5],
                rows[5].width.min(support_max_width),
                rows[5].height,
            );
            frame.render_widget(
                Paragraph::new(support_lines.to_vec())
                    .alignment(support_alignment)
                    .wrap(Wrap { trim: false }),
                support_area,
            );
        }
    }

    #[cfg(test)]
    fn welcome_compact_lines(version: &str, render_route_copy: &str) -> Vec<Line<'static>> {
        let mut lines = Self::welcome_hero_lines(version);
        lines.extend(Self::welcome_support_lines_for_route(render_route_copy));
        lines
    }

    #[cfg(test)]
    fn welcome_hero_lines(version: &str) -> Vec<Line<'static>> {
        let mut lines = vec![Self::welcome_version_line(version), Line::from("")];
        lines.extend(Self::welcome_primary_lines());
        lines
    }

    fn render_welcome_brand_media(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        _inline_logo: bool,
        fill_style: Style,
    ) -> Option<Rect> {
        let hero_lines = Self::welcome_hero_wordmark_lines(area.width);
        if !hero_lines.is_empty() {
            let shadow_style = Self::welcome_wordmark_shadow_style();
            let shadow_width = area.width.saturating_sub(1).max(1);
            let shadow_height = area.height.saturating_sub(1).max(1);
            let shadow_area = Rect::new(
                area.x.saturating_add(1),
                area.y.saturating_add(1),
                shadow_width,
                shadow_height,
            );

            frame.render_widget(
                Paragraph::new(hero_lines.clone())
                    .alignment(Alignment::Center)
                    .style(shadow_style),
                shadow_area,
            );
            frame.render_widget(
                Paragraph::new(hero_lines)
                    .alignment(Alignment::Center)
                    .style(fill_style),
                area,
            );
            return None;
        }

        frame.render_widget(
            Paragraph::new(Self::brand_ascii_logo_lines(area.width, area.height))
                .alignment(Alignment::Center)
                .style(fill_style),
            area,
        );
        None
    }

    fn render_guided_brand_media(frame: &mut ratatui::Frame<'_>, area: Rect, fill_style: Style) {
        frame.render_widget(
            Paragraph::new(Self::brand_ascii_logo_lines(area.width, area.height))
                .alignment(Alignment::Center)
                .style(fill_style),
            area,
        );
    }

    fn welcome_wordmark_shadow_style() -> Style {
        let palette = Self::palette();
        let light_palette = OnboardPalette::light();
        let dark_palette = OnboardPalette::dark();
        let shadow_color = if palette == light_palette {
            Color::Rgb(181, 138, 143)
        } else if palette == dark_palette {
            Color::Rgb(88, 34, 43)
        } else {
            palette.muted_text
        };

        Style::default()
            .fg(shadow_color)
            .add_modifier(Modifier::BOLD)
    }

    fn step_stage_identity(step: OnboardWizardStep) -> (&'static str, &'static str, &'static str) {
        match step {
            OnboardWizardStep::Welcome => (
                "Opening ceremony",
                "Start in a calm full-screen deck before any real config is written.",
                "ENTRY",
            ),
            OnboardWizardStep::Authentication => (
                "Access calibration",
                "Choose provider access, model routing, and search defaults.",
                "ACCESS",
            ),
            OnboardWizardStep::RuntimeDefaults => (
                "Behavior tuning",
                "Set memory, operator voice, and runtime surfaces.",
                "BEHAVIOR",
            ),
            OnboardWizardStep::Workspace => (
                "Boundary definition",
                "Confirm the local paths LoongClaw will use and remember.",
                "BOUNDARY",
            ),
            OnboardWizardStep::Protocols => (
                "Bridge posture",
                "Turn channels and ACP bridges on only where they are needed.",
                "BRIDGE",
            ),
            OnboardWizardStep::EnvironmentCheck => (
                "Verification gate",
                "Check local signals before trusting the write path.",
                "VERIFY",
            ),
            OnboardWizardStep::ReviewAndWrite => (
                "Write gate",
                "Review the draft and decide whether to write it.",
                "WRITE",
            ),
            OnboardWizardStep::Ready => (
                "Launch handoff",
                "Setup is complete. Open chat or leave the deck cleanly.",
                "LAUNCH",
            ),
        }
    }

    fn selection_stage_lines(
        _step: OnboardWizardStep,
        title: &str,
        selected_index: usize,
        total: usize,
        _width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        vec![
            Line::from(Span::styled(
                title.to_owned(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("Choice {} of {}", selected_index + 1, total),
                Style::default().fg(palette.secondary_text),
            )),
        ]
    }

    fn selection_card_theme_for_step(_step: OnboardWizardStep) -> SelectionCardTheme {
        Self::stable_selection_theme()
    }

    #[allow(dead_code)]
    fn launch_action_card_theme() -> SelectionCardTheme {
        Self::stable_selection_theme()
    }

    fn selection_focus_lines(
        title: &str,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let position = format!("Choice {} of {}", selected_index + 1, total);
        let mut lines = vec![
            Line::from(Span::styled(
                position,
                Style::default().fg(palette.muted_text),
            )),
            Line::from(Span::styled(
                title.to_owned(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                item.label.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
        ];
        if let Some(hint) = item.hint.as_deref() {
            lines.push(Line::from(Span::styled(
                hint.to_owned(),
                Style::default().fg(palette.secondary_text),
            )));
        }
        lines
    }

    fn selection_guidance_lines(
        step: OnboardWizardStep,
        title: &str,
        item: &SelectionItem,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let guidance = match (step, title) {
            (OnboardWizardStep::Authentication, "Provider") => {
                "Sets the provider family before model and credential defaults lock in."
            }
            (OnboardWizardStep::RuntimeDefaults, "Memory Profile") => {
                "Sets how much earlier context LoongClaw keeps live during future sessions."
            }
            (OnboardWizardStep::Protocols, "ACP Protocol") => {
                "Decides whether this setup stays local-only or exposes a protocol surface."
            }
            (OnboardWizardStep::Protocols, "ACP Backend") => {
                "Chooses how ACP traffic is carried once the bridge is enabled."
            }
            _ => "Keeps this draft explicit before the next page adds more detail.",
        };

        vec![
            Line::from(Span::styled(
                guidance,
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                format!("Live focus: {}", item.label),
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    #[allow(dead_code)] // retained for rendering copy tests during the layout transition
    fn selection_compact_sidebar_lines(
        step: OnboardWizardStep,
        title: &str,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = Self::selection_focus_lines(title, item, selected_index, total);
        lines.extend(Self::selection_guidance_lines(step, title, item));
        lines
    }

    #[allow(dead_code)] // retained for rendering copy tests during the layout transition
    fn multi_selection_sidebar_lines(
        _step: OnboardWizardStep,
        title: &str,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
        selected_count: usize,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let mut lines = vec![Line::from(Span::styled(
            format!("Selected {selected_count} of {total}"),
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ))];
        lines.extend(Self::selection_focus_lines(
            title,
            item,
            selected_index,
            total,
        ));
        lines
    }

    fn provider_picker_options(
        &self,
        current_provider: &mvp::config::ProviderConfig,
    ) -> Vec<ProviderPickerOption> {
        let imported_kinds = self
            .provider_selection_plan
            .imported_choices
            .iter()
            .map(|choice| choice.kind.as_str())
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        let mut ordered_kinds = Vec::new();

        let mut push_kind = |kind: mvp::config::ProviderKind| {
            if seen.insert(kind.as_str()) {
                ordered_kinds.push(kind);
            }
        };

        push_kind(current_provider.kind);
        for choice in &self.provider_selection_plan.imported_choices {
            push_kind(choice.kind);
        }
        for kind in mvp::config::ProviderKind::all_sorted() {
            push_kind(*kind);
        }

        let mut options = ordered_kinds
            .into_iter()
            .flat_map(|kind| {
                let preview =
                    crate::migration::provider_selection::resolve_provider_config_from_selection(
                        current_provider,
                        &self.provider_selection_plan,
                        kind,
                    );
                let base_status_label = if kind == current_provider.kind {
                    "current draft"
                } else if imported_kinds.contains(kind.as_str()) {
                    "detected profile"
                } else {
                    "fresh profile"
                };
                if kind != mvp::config::ProviderKind::Openai {
                    let prefers_oauth_route = Self::provider_prefers_oauth_route(&preview);
                    let profile_id =
                        Self::provider_picker_profile_id(kind, &preview, prefers_oauth_route);
                    let item = SelectionItem::new(
                        kind.display_name(),
                        Some(Self::provider_picker_hint(base_status_label, &preview)),
                    );
                    return vec![ProviderPickerOption {
                        profile_id,
                        kind,
                        item,
                        preview,
                    }];
                }

                let current_route_is_oauth = Self::provider_prefers_oauth_route(current_provider);
                let api_status_label = if kind == current_provider.kind && !current_route_is_oauth {
                    "current draft"
                } else {
                    "api route"
                };
                let oauth_status_label = if kind == current_provider.kind && current_route_is_oauth
                {
                    "current draft"
                } else {
                    "oauth route"
                };

                let api_preview = Self::openai_api_picker_preview(preview.clone());
                let api_profile_id = Self::provider_picker_profile_id(kind, &api_preview, false);
                let api_item = SelectionItem::new(
                    "OpenAI API",
                    Some(Self::provider_picker_hint(api_status_label, &api_preview)),
                );
                let oauth_preview = Self::openai_codex_oauth_picker_preview(preview);
                let oauth_profile_id = Self::provider_picker_profile_id(kind, &oauth_preview, true);
                let oauth_item = SelectionItem::new(
                    "OpenAI Codex OAuth",
                    Some(Self::provider_picker_hint(
                        oauth_status_label,
                        &oauth_preview,
                    )),
                );

                vec![
                    ProviderPickerOption {
                        profile_id: api_profile_id,
                        kind,
                        item: api_item,
                        preview: api_preview,
                    },
                    ProviderPickerOption {
                        profile_id: oauth_profile_id,
                        kind,
                        item: oauth_item,
                        preview: oauth_preview,
                    },
                ]
            })
            .collect::<Vec<_>>();

        options.sort_by(|left, right| {
            let left_label = left.item.label.to_ascii_lowercase();
            let right_label = right.item.label.to_ascii_lowercase();
            left_label
                .cmp(&right_label)
                .then(left.profile_id.cmp(&right.profile_id))
        });
        options
    }

    fn provider_picker_profile_id(
        kind: mvp::config::ProviderKind,
        preview: &mvp::config::ProviderConfig,
        prefers_oauth_route: bool,
    ) -> String {
        if kind == mvp::config::ProviderKind::Openai && prefers_oauth_route {
            return "openai-codex-oauth".to_owned();
        }

        preview.inferred_profile_id()
    }

    fn default_onboard_config() -> &'static mvp::config::LoongClawConfig {
        static DEFAULT_CONFIG: OnceLock<mvp::config::LoongClawConfig> = OnceLock::new();
        DEFAULT_CONFIG.get_or_init(mvp::config::LoongClawConfig::default)
    }

    fn sorted_service_channel_catalog_entries(
        config: &mvp::config::LoongClawConfig,
    ) -> Vec<mvp::channel::ChannelCatalogEntry> {
        let inventory = mvp::channel::channel_inventory(config);
        let mut entries = inventory
            .channel_surfaces
            .into_iter()
            .map(|surface| surface.catalog)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            let left_label = left.label.to_ascii_lowercase();
            let right_label = right.label.to_ascii_lowercase();
            left_label
                .cmp(&right_label)
                .then(left.selection_order.cmp(&right.selection_order))
                .then(left.id.cmp(right.id))
        });
        entries
    }

    fn supports_onboard_acp_runtime(metadata: &mvp::acp::AcpBackendMetadata) -> bool {
        crate::onboard_preflight::supports_onboard_acp_runtime(metadata)
    }

    fn onboard_acp_backend_options() -> CliResult<Vec<OnboardAcpBackendOption>> {
        let metadata = mvp::acp::list_acp_backend_metadata()?;
        let mut options = metadata
            .into_iter()
            .filter(Self::supports_onboard_acp_runtime)
            .map(|backend| OnboardAcpBackendOption {
                id: backend.id.to_owned(),
                summary: backend.summary.to_owned(),
            })
            .collect::<Vec<_>>();

        options.sort_by(|left, right| left.id.cmp(&right.id));

        if options.is_empty() {
            return Err("no runnable ACP backends are registered".to_owned());
        }

        Ok(options)
    }

    fn default_onboard_acp_backend_index(
        draft: &OnboardDraft,
        options: &[OnboardAcpBackendOption],
    ) -> usize {
        let current_backend = draft
            .protocols
            .acp_backend
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(current_backend) = current_backend {
            let current_index = options
                .iter()
                .position(|option| option.id == current_backend);
            if let Some(current_index) = current_index {
                return current_index;
            }
        }

        let resolved_backend = mvp::acp::resolve_acp_backend_selection(&draft.config).id;
        let resolved_index = options
            .iter()
            .position(|option| option.id == resolved_backend);
        if let Some(resolved_index) = resolved_index {
            return resolved_index;
        }

        0
    }

    fn channel_primary_operation(
        entry: &mvp::channel::ChannelCatalogEntry,
    ) -> Option<mvp::channel::ChannelCatalogOperation> {
        let implemented_send = entry.operation("send").copied().filter(|operation| {
            operation.availability == mvp::channel::ChannelCatalogOperationAvailability::Implemented
        });
        let implemented_serve = entry.operation("serve").copied().filter(|operation| {
            operation.availability == mvp::channel::ChannelCatalogOperationAvailability::Implemented
        });
        match entry.implementation_status {
            mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked => {
                implemented_serve.or(implemented_send)
            }
            mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
            | mvp::channel::ChannelCatalogImplementationStatus::Stub => {
                implemented_send.or(implemented_serve)
            }
        }
    }

    fn config_string_path_value(
        config: &mvp::config::LoongClawConfig,
        path: &str,
    ) -> Option<String> {
        let config_value_result = serde_json::to_value(config);
        let Ok(config_value) = config_value_result else {
            return None;
        };
        Self::read_string_path_value(&config_value, path)
    }

    fn config_path_value(config: &mvp::config::LoongClawConfig, path: &str) -> Option<Value> {
        let config_value_result = serde_json::to_value(config);
        let Ok(config_value) = config_value_result else {
            return None;
        };
        Self::read_json_path_value(&config_value, path).cloned()
    }

    fn read_json_path_value<'a>(config_value: &'a Value, path: &str) -> Option<&'a Value> {
        let mut current_value = config_value;
        for segment in path.split('.').filter(|segment| !segment.trim().is_empty()) {
            let object = current_value.as_object()?;
            current_value = object.get(segment)?;
        }
        Some(current_value)
    }

    fn read_string_path_value(config_value: &Value, path: &str) -> Option<String> {
        Self::read_json_path_value(config_value, path)?
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    fn path_accepts_string_input(config: &mvp::config::LoongClawConfig, path: &str) -> bool {
        let Some(path_value) = Self::config_path_value(config, path) else {
            return false;
        };
        matches!(path_value, Value::String(_) | Value::Null)
    }

    fn build_channel_pairing_prompt(
        config: &mvp::config::LoongClawConfig,
        requirement: &mvp::channel::ChannelCatalogOperationRequirement,
    ) -> Option<ChannelPairingPrompt> {
        let env_pointer_path = requirement
            .env_pointer_paths
            .iter()
            .find(|path| !path.contains("<account>"))
            .copied();
        if let Some(env_pointer_path) = env_pointer_path {
            let current_value = Self::config_string_path_value(config, env_pointer_path);
            let default_value = current_value
                .or_else(|| {
                    Self::config_string_path_value(Self::default_onboard_config(), env_pointer_path)
                })
                .or_else(|| requirement.default_env_var.map(str::to_owned))
                .unwrap_or_default();
            return Some(ChannelPairingPrompt {
                field_key: env_pointer_path,
                label: format!("{} env", requirement.label),
                default_value,
            });
        }

        let config_path = requirement
            .config_paths
            .iter()
            .find(|path| !path.contains("<account>"))
            .copied()?;
        let config_accepts_string = Self::path_accepts_string_input(config, config_path);
        let default_accepts_string =
            Self::path_accepts_string_input(Self::default_onboard_config(), config_path);
        if !config_accepts_string && !default_accepts_string {
            return None;
        }

        let current_value = Self::config_string_path_value(config, config_path);
        let default_value = current_value
            .or_else(|| Self::config_string_path_value(Self::default_onboard_config(), config_path))
            .unwrap_or_default();
        Some(ChannelPairingPrompt {
            field_key: config_path,
            label: requirement.label.to_owned(),
            default_value,
        })
    }

    fn channel_pairing_prompts(
        config: &mvp::config::LoongClawConfig,
        entry: &mvp::channel::ChannelCatalogEntry,
    ) -> Vec<ChannelPairingPrompt> {
        let Some(primary_operation) = Self::channel_primary_operation(entry) else {
            return Vec::new();
        };
        let feishu_mode = Self::config_string_path_value(config, "feishu.mode");
        let feishu_webhook_mode = feishu_mode
            .as_deref()
            .is_some_and(|mode| mode.eq_ignore_ascii_case("webhook"));
        primary_operation
            .requirements
            .iter()
            .filter(|requirement| requirement.id != "enabled")
            .filter(|requirement| {
                if entry.id != "feishu" {
                    return true;
                }
                if requirement.id != "verification_token" && requirement.id != "encrypt_key" {
                    return true;
                }
                feishu_webhook_mode
            })
            .filter_map(|requirement| Self::build_channel_pairing_prompt(config, requirement))
            .collect()
    }

    fn run_selected_channel_pairing_sequence(
        &mut self,
        draft: &mut OnboardDraft,
        entries: &[mvp::channel::ChannelCatalogEntry],
    ) -> CliResult<bool> {
        if entries.is_empty() {
            return Ok(true);
        }

        let mut channel_index = 0usize;
        let mut prompt_index = 0usize;

        loop {
            let Some(entry) = entries.get(channel_index) else {
                return Ok(true);
            };
            let prompts = Self::channel_pairing_prompts(&draft.config, entry);
            if prompts.is_empty() {
                channel_index += 1;
                prompt_index = 0;
                continue;
            }

            let Some(prompt) = prompts.get(prompt_index) else {
                channel_index += 1;
                prompt_index = 0;
                continue;
            };

            let input_label = format!("{} · {}", entry.label, prompt.label);
            let footer_hint = "Enter keeps or submits this binding";
            let input_result = self.run_input_loop(
                OnboardWizardStep::Protocols,
                input_label.as_str(),
                prompt.default_value.as_str(),
                footer_hint,
            )?;
            match input_result {
                InputLoopResult::Back => {
                    if prompt_index > 0 {
                        prompt_index -= 1;
                        continue;
                    }
                    if channel_index == 0 {
                        return Ok(false);
                    }

                    channel_index -= 1;
                    let previous_entry = &entries[channel_index];
                    let previous_prompts =
                        Self::channel_pairing_prompts(&draft.config, previous_entry);
                    prompt_index = previous_prompts.len().saturating_sub(1);
                }
                InputLoopResult::Submitted(value) => {
                    let updated =
                        draft.set_channel_pairing_string_path(prompt.field_key, Some(value));
                    let _ = updated;
                    prompt_index += 1;
                }
            }
        }
    }

    fn provider_picker_hint(status_label: &str, preview: &mvp::config::ProviderConfig) -> String {
        let cue = preview
            .preview_transport_summary()
            .or_else(|| {
                provider_credential_policy::provider_credential_env_hint(preview)
                    .map(|env_name| format!("env {env_name}"))
            })
            .unwrap_or_else(|| {
                if preview.requires_explicit_auth_configuration() {
                    "manual auth".to_owned()
                } else {
                    "local runtime".to_owned()
                }
            });
        format!("{status_label} · {cue}")
    }

    fn provider_prefers_oauth_route(preview: &mvp::config::ProviderConfig) -> bool {
        let has_oauth_access_token = preview.oauth_access_token.is_some();
        if has_oauth_access_token {
            return true;
        }

        let configured_binding =
            provider_credential_policy::configured_provider_credential_env_binding(preview);
        let Some(configured_binding) = configured_binding else {
            return false;
        };

        configured_binding.field
            == provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken
    }

    fn openai_api_picker_preview(
        mut preview: mvp::config::ProviderConfig,
    ) -> mvp::config::ProviderConfig {
        preview.oauth_access_token = None;
        preview.clear_oauth_access_token_env_binding();

        let configured_api_key = preview.configured_api_key_env_override();
        let has_inline_api_key = preview.api_key.is_some();
        let should_apply_default_api_key_env = configured_api_key.is_none() && !has_inline_api_key;
        let default_api_key_env = if should_apply_default_api_key_env {
            preview.kind.default_api_key_env()
        } else {
            None
        };
        if let Some(default_api_key_env) = default_api_key_env {
            preview.set_api_key_env_binding(Some(default_api_key_env.to_owned()));
        }

        preview
    }

    fn openai_codex_oauth_picker_preview(
        mut preview: mvp::config::ProviderConfig,
    ) -> mvp::config::ProviderConfig {
        preview.api_key = None;
        preview.clear_api_key_env_binding();

        let configured_oauth = preview.configured_oauth_access_token_env_override();
        let has_oauth_secret = preview.oauth_access_token.is_some();
        let should_apply_default_oauth_env = configured_oauth.is_none() && !has_oauth_secret;
        let default_oauth_env = if should_apply_default_oauth_env {
            preview.kind.default_oauth_access_token_env()
        } else {
            None
        };
        if let Some(default_oauth_env) = default_oauth_env {
            preview.set_oauth_access_token_env_binding(Some(default_oauth_env.to_owned()));
        }

        preview
    }

    fn current_provider_picker_index(
        current_provider: &mvp::config::ProviderConfig,
        options: &[ProviderPickerOption],
    ) -> usize {
        let current_route_is_oauth = Self::provider_prefers_oauth_route(current_provider);
        options
            .iter()
            .position(|option| {
                if option.kind != current_provider.kind {
                    return false;
                }
                let option_route_is_oauth = Self::provider_prefers_oauth_route(&option.preview);
                option_route_is_oauth == current_route_is_oauth
            })
            .unwrap_or(0)
    }

    fn web_search_picker_options(
        config: &mvp::config::LoongClawConfig,
    ) -> Vec<WebSearchPickerOption> {
        let mut options = mvp::config::web_search_provider_descriptors()
            .iter()
            .map(|descriptor| {
                let hint = if descriptor.requires_api_key {
                    let configured_source =
                        crate::onboard_web_search::configured_web_search_provider_credential_source_value(
                            config,
                            descriptor.id,
                        );
                    let source_label = configured_source
                        .map(|value| format!("ready via {value}"))
                        .unwrap_or_else(|| "credential env".to_owned());
                    format!("{} · {}", descriptor.description, source_label)
                } else {
                    format!("{} · no credential required", descriptor.description)
                };
                let item = SelectionItem::new(descriptor.display_name, Some(hint));

                WebSearchPickerOption {
                    id: descriptor.id,
                    item,
                    requires_api_key: descriptor.requires_api_key,
                }
            })
            .collect::<Vec<_>>();

        options.sort_by(|left, right| {
            let left_label = left.item.label.to_ascii_lowercase();
            let right_label = right.item.label.to_ascii_lowercase();
            left_label.cmp(&right_label).then(left.id.cmp(right.id))
        });
        options
    }

    fn current_web_search_picker_index(
        config: &mvp::config::LoongClawConfig,
        options: &[WebSearchPickerOption],
    ) -> usize {
        let current_provider = crate::onboard_web_search::current_web_search_provider(config);
        options
            .iter()
            .position(|option| option.id == current_provider)
            .unwrap_or(0)
    }

    fn web_search_picker_checked_indices(
        config: &mvp::config::LoongClawConfig,
        options: &[WebSearchPickerOption],
        selected_provider_ids: &[String],
    ) -> Vec<usize> {
        if !selected_provider_ids.is_empty() {
            return options
                .iter()
                .enumerate()
                .filter(|(_, option)| {
                    selected_provider_ids
                        .iter()
                        .any(|provider_id| provider_id == option.id)
                })
                .map(|(index, _)| index)
                .collect();
        }

        let default_provider = crate::onboard_web_search::current_web_search_provider(config);
        let mut checked_indices = options
            .iter()
            .enumerate()
            .filter(|(_, option)| {
                if option.id == default_provider {
                    return true;
                }

                crate::onboard_web_search::configured_web_search_provider_credential_source_value(
                    config, option.id,
                )
                .is_some()
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        if checked_indices.is_empty() {
            let default_index = Self::current_web_search_picker_index(config, options);
            checked_indices.push(default_index);
        }

        checked_indices
    }

    fn selected_web_search_provider_ids(
        options: &[WebSearchPickerOption],
        checked_indices: &[usize],
    ) -> Vec<String> {
        let checked_index_set = checked_indices.iter().copied().collect::<BTreeSet<_>>();
        options
            .iter()
            .enumerate()
            .filter(|(index, _)| checked_index_set.contains(index))
            .map(|(_, option)| option.id.to_owned())
            .collect()
    }

    fn selected_web_search_options<'a>(
        options: &'a [WebSearchPickerOption],
        selected_provider_ids: &[String],
    ) -> Vec<&'a WebSearchPickerOption> {
        options
            .iter()
            .filter(|option| {
                selected_provider_ids
                    .iter()
                    .any(|provider_id| provider_id == option.id)
            })
            .collect()
    }

    fn selected_web_search_credential_options<'a>(
        options: &'a [WebSearchPickerOption],
        selected_provider_ids: &[String],
    ) -> Vec<&'a WebSearchPickerOption> {
        Self::selected_web_search_options(options, selected_provider_ids)
            .into_iter()
            .filter(|option| option.requires_api_key)
            .collect()
    }

    fn apply_web_search_provider_selection(
        draft: &mut OnboardDraft,
        options: &[WebSearchPickerOption],
        selected_provider_ids: &[String],
        default_provider_id: &str,
    ) {
        draft.set_web_search_default_provider(default_provider_id.to_owned());

        for option in options {
            let is_selected = selected_provider_ids
                .iter()
                .any(|provider_id| provider_id == option.id);
            if !is_selected {
                draft.clear_web_search_credential(option.id);
            }
        }
    }

    fn run_provider_selection_loop(
        &mut self,
        current_provider: &mvp::config::ProviderConfig,
    ) -> CliResult<Option<Vec<ProviderPickerOption>>> {
        let options = self.provider_picker_options(current_provider);
        if options.is_empty() {
            return Err("no providers available to select".to_owned());
        }

        let default_index = Self::current_provider_picker_index(current_provider, &options);
        let initial_focus_index = 0usize;
        let items = options
            .iter()
            .map(|option| option.item.clone())
            .collect::<Vec<_>>();
        let checked_indices = vec![default_index];
        let selection_result = self.run_multi_selection_loop(
            OnboardWizardStep::Authentication,
            "Providers",
            items,
            initial_focus_index,
            checked_indices,
            "Space toggle  Enter confirm",
        )?;

        match selection_result {
            MultiSelectionLoopResult::Back => Ok(None),
            MultiSelectionLoopResult::Submitted(indices) => {
                let effective_indices = if indices.is_empty() {
                    vec![default_index]
                } else {
                    indices
                };
                let selected_options = effective_indices
                    .into_iter()
                    .filter_map(|index| options.get(index))
                    .cloned()
                    .collect::<Vec<_>>();
                Ok(Some(selected_options))
            }
        }
    }

    fn run_default_provider_selection_loop(
        &mut self,
        options: &[ProviderPickerOption],
        default_profile_id: &str,
    ) -> CliResult<Option<String>> {
        let items = options
            .iter()
            .map(|option| option.item.clone())
            .collect::<Vec<_>>();
        let default_index = options
            .iter()
            .position(|option| option.profile_id == default_profile_id)
            .unwrap_or(0);
        match self.run_selection_loop(
            OnboardWizardStep::Authentication,
            "Default Provider",
            items,
            default_index,
            "Enter confirm",
        )? {
            SelectionLoopResult::Back => Ok(None),
            SelectionLoopResult::Selected(index) => {
                let selected_profile_id = options
                    .get(index)
                    .map(|option| option.profile_id.clone())
                    .ok_or_else(|| "invalid default provider selection".to_owned())?;
                Ok(Some(selected_profile_id))
            }
        }
    }

    fn run_provider_configuration_loop(
        &mut self,
        provider_label: &str,
        provider: &mvp::config::ProviderConfig,
    ) -> CliResult<ProviderConfigurationLoopResult> {
        let mut configured_provider = provider.clone();
        let mut sub_step: u8 = 0;
        let mut model_return_step: u8 = 0;

        loop {
            match sub_step {
                0 => {
                    let model_context =
                        crate::onboarding_model_policy::onboarding_model_selection_context(
                            &configured_provider,
                        );
                    let reviewed_model = model_context.recommended_model.clone();
                    let reviewed_model_is_available = reviewed_model.is_some();
                    let reviewed_label = reviewed_model
                        .as_deref()
                        .map(|model| format!("Use reviewed model ({model})"))
                        .unwrap_or_else(|| "Use reviewed model".to_owned());
                    let auto_hint = if model_context.allows_auto_fallback_hint {
                        "preserve provider-side fallback discovery"
                    } else {
                        "keep the configured automatic routing behavior"
                    };
                    let custom_hint = if configured_provider
                        .model
                        .trim()
                        .eq_ignore_ascii_case("auto")
                    {
                        "type an exact provider model id"
                    } else {
                        "override with a different explicit model id"
                    };
                    let mut items = Vec::new();
                    let mut strategies = Vec::new();

                    if let Some(reviewed_model) = reviewed_model.as_deref() {
                        items.push(SelectionItem::new(
                            reviewed_label,
                            Some(format!("recommended default: {reviewed_model}")),
                        ));
                        strategies.push("reviewed");
                    }

                    items.push(SelectionItem::new("Keep auto fallback", Some(auto_hint)));
                    strategies.push("auto");
                    items.push(SelectionItem::new("Enter custom model", Some(custom_hint)));
                    strategies.push("custom");

                    let current_model = configured_provider.model.trim();
                    let use_auto_default = current_model.eq_ignore_ascii_case("auto");
                    let use_reviewed_default = reviewed_model
                        .as_deref()
                        .is_some_and(|recommended| current_model == recommended);
                    let default_index = if reviewed_model_is_available
                        && (use_auto_default || use_reviewed_default)
                    {
                        0
                    } else if use_auto_default {
                        strategies
                            .iter()
                            .position(|strategy| *strategy == "auto")
                            .unwrap_or(0)
                    } else {
                        strategies
                            .iter()
                            .position(|strategy| *strategy == "custom")
                            .unwrap_or(0)
                    };

                    match self.run_selection_loop(
                        OnboardWizardStep::Authentication,
                        format!("{provider_label} model").as_str(),
                        items,
                        default_index,
                        "Enter confirm",
                    )? {
                        SelectionLoopResult::Back => {
                            return Ok(ProviderConfigurationLoopResult::Back);
                        }
                        SelectionLoopResult::Selected(index) => {
                            let strategy = strategies
                                .get(index)
                                .copied()
                                .ok_or_else(|| "invalid model strategy selection".to_owned())?;
                            if strategy == "reviewed" {
                                let reviewed_model = reviewed_model
                                    .clone()
                                    .ok_or_else(|| "reviewed model missing".to_owned())?;
                                configured_provider.model = reviewed_model;
                                model_return_step = 0;
                                sub_step = 2;
                                continue;
                            }
                            if strategy == "auto" {
                                configured_provider.model = "auto".to_owned();
                                model_return_step = 0;
                                sub_step = 2;
                                continue;
                            }
                            sub_step = 1;
                        }
                    }
                }
                1 => {
                    let custom_default =
                        crate::onboarding_model_policy::resolve_onboarding_model_prompt_default(
                            &configured_provider,
                            None,
                        )
                        .unwrap_or_else(|_| configured_provider.model.clone());
                    match self.run_input_loop(
                        OnboardWizardStep::Authentication,
                        format!("{provider_label} custom model:").as_str(),
                        &custom_default,
                        "Enter confirm model",
                    )? {
                        InputLoopResult::Back => {
                            sub_step = 0;
                        }
                        InputLoopResult::Submitted(model) => {
                            let trimmed_model = model.trim();
                            if trimmed_model.is_empty() {
                                return Err("model cannot be empty".to_owned());
                            }
                            configured_provider.model = trimmed_model.to_owned();
                            model_return_step = 1;
                            sub_step = 2;
                        }
                    }
                }
                2 => {
                    if !configured_provider.requires_explicit_auth_configuration() {
                        let configured_provider = Box::new(configured_provider);
                        return Ok(ProviderConfigurationLoopResult::Configured(
                            configured_provider,
                        ));
                    }
                    let provider_prefers_oauth =
                        Self::provider_prefers_oauth_route(&configured_provider);
                    if provider_prefers_oauth {
                        match self.run_openai_codex_oauth_loop()? {
                            OpenaiCodexOauthLoopResult::Back => {
                                sub_step = model_return_step;
                            }
                            OpenaiCodexOauthLoopResult::Authorized(grant) => {
                                configured_provider.api_key = None;
                                configured_provider.clear_api_key_env_binding();
                                configured_provider.clear_oauth_access_token_env_binding();
                                configured_provider.oauth_access_token = Some(
                                    loongclaw_contracts::SecretRef::Inline(grant.access_token),
                                );
                                let configured_provider = Box::new(configured_provider);
                                return Ok(ProviderConfigurationLoopResult::Configured(
                                    configured_provider,
                                ));
                            }
                        }
                        continue;
                    }
                    let current_credential_env =
                        provider_credential_policy::provider_credential_env_hint(
                            &configured_provider,
                        )
                        .unwrap_or_default();
                    match self.run_input_loop(
                        OnboardWizardStep::Authentication,
                        format!("{provider_label} credential env:").as_str(),
                        &current_credential_env,
                        "Enter confirm env name",
                    )? {
                        InputLoopResult::Back => {
                            sub_step = model_return_step;
                        }
                        InputLoopResult::Submitted(env_name) => {
                            let selected_api_key_env = env_name.trim();
                            if selected_api_key_env.is_empty() {
                                configured_provider.clear_api_key_env_binding();
                                configured_provider.clear_oauth_access_token_env_binding();
                            } else {
                                configured_provider.api_key = None;
                                configured_provider.oauth_access_token = None;
                                let selected_field =
                                    provider_credential_policy::selected_provider_credential_env_field(
                                        &configured_provider,
                                        selected_api_key_env,
                                    );
                                if selected_field
                                    == provider_credential_policy::ProviderCredentialEnvField::ApiKey
                                {
                                    configured_provider.clear_oauth_access_token_env_binding();
                                    configured_provider.set_api_key_env_binding(Some(
                                        selected_api_key_env.to_owned(),
                                    ));
                                } else {
                                    configured_provider.clear_api_key_env_binding();
                                    configured_provider.set_oauth_access_token_env_binding(Some(
                                        selected_api_key_env.to_owned(),
                                    ));
                                }
                            }
                            let configured_provider = Box::new(configured_provider);
                            return Ok(ProviderConfigurationLoopResult::Configured(
                                configured_provider,
                            ));
                        }
                    }
                }
                _ => {
                    return Err("invalid provider configuration state".to_owned());
                }
            }
        }
    }

    fn run_selected_provider_configuration_sequence(
        &mut self,
        selected_options: &[ProviderPickerOption],
    ) -> CliResult<Option<Vec<ProviderPickerOption>>> {
        let mut configured_options = selected_options.to_vec();
        let mut provider_index = 0usize;

        loop {
            let Some(current_option) = configured_options.get(provider_index).cloned() else {
                return Ok(Some(configured_options));
            };
            let configuration_result = self.run_provider_configuration_loop(
                current_option.item.label.as_str(),
                &current_option.preview,
            )?;
            match configuration_result {
                ProviderConfigurationLoopResult::Back => {
                    if provider_index == 0 {
                        return Ok(None);
                    }
                    provider_index -= 1;
                }
                ProviderConfigurationLoopResult::Configured(configured_provider) => {
                    let configured_provider = *configured_provider;
                    if let Some(option) = configured_options.get_mut(provider_index) {
                        option.preview = configured_provider;
                    }
                    provider_index += 1;
                }
            }
        }
    }

    fn selected_provider_profiles(
        options: &[ProviderPickerOption],
    ) -> BTreeMap<String, mvp::config::ProviderProfileConfig> {
        let mut profiles = BTreeMap::new();
        for option in options {
            let mut profile =
                mvp::config::ProviderProfileConfig::from_provider(option.preview.clone());
            profile.default_for_kind = false;
            profiles.insert(option.profile_id.clone(), profile);
        }
        profiles
    }

    fn default_active_provider_profile_id(
        current_provider: &mvp::config::ProviderConfig,
        options: &[ProviderPickerOption],
    ) -> String {
        let current_index = Self::current_provider_picker_index(current_provider, options);
        options
            .get(current_index)
            .map(|option| option.profile_id.clone())
            .or_else(|| options.first().map(|option| option.profile_id.clone()))
            .unwrap_or_default()
    }

    fn apply_selected_provider_profiles(
        draft: &mut OnboardDraft,
        selected_options: &[ProviderPickerOption],
        active_profile_id: &str,
    ) -> CliResult<()> {
        let profiles = Self::selected_provider_profiles(selected_options);
        draft.set_provider_runtime_profiles(profiles, active_profile_id.to_owned())
    }

    fn showcase_stage_identity(
        variant: ShowcaseStageVariant,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        match variant {
            ShowcaseStageVariant::EntryPath => (
                "Opening Gate",
                "Entry Stage",
                "Choose the first move",
                "Commit to the path that gets the operator from discovery to a credible first run fastest.",
                "FIRST MOVE",
            ),
            ShowcaseStageVariant::DetectedStartingPoint => (
                "Signal Intake",
                "Import Stage",
                "Audit reusable local signals",
                "Choose which detected machine posture deserves to seed the draft before the guided steps begin.",
                "LOCAL SIGNALS",
            ),
            ShowcaseStageVariant::ShortcutChoice => (
                "Shortcut Gate",
                "Shortcut Stage",
                "Decide the run-up tempo",
                "Make the fast path explicit: either trust this draft now or drop into the full editing deck before writing.",
                "FAST PATH",
            ),
            ShowcaseStageVariant::Generic => (
                "Selection Stage",
                "Selection Stage",
                "Lock one explicit route",
                "This compatibility path still stays full-screen so the transition never collapses into a tiny dialog.",
                "FULLSCREEN",
            ),
        }
    }

    #[allow(dead_code)]
    fn showcase_brand_tags(variant: ShowcaseStageVariant) -> [BrandMarkTag; 2] {
        let palette = Self::palette();
        match variant {
            ShowcaseStageVariant::EntryPath => [
                BrandMarkTag {
                    label: "ENTRY",
                    color: palette.brand,
                },
                BrandMarkTag {
                    label: "CEREMONY",
                    color: Color::Cyan,
                },
            ],
            ShowcaseStageVariant::DetectedStartingPoint => [
                BrandMarkTag {
                    label: "IMPORT",
                    color: palette.brand,
                },
                BrandMarkTag {
                    label: "SIGNALS",
                    color: Color::Cyan,
                },
            ],
            ShowcaseStageVariant::ShortcutChoice => [
                BrandMarkTag {
                    label: "SHORTCUT",
                    color: palette.brand,
                },
                BrandMarkTag {
                    label: "FAST TRACK",
                    color: Color::Cyan,
                },
            ],
            ShowcaseStageVariant::Generic => [
                BrandMarkTag {
                    label: "GUIDED",
                    color: palette.brand,
                },
                BrandMarkTag {
                    label: "PREVIEW",
                    color: Color::Cyan,
                },
            ],
        }
    }

    #[allow(dead_code)]
    fn showcase_signal_copy(
        variant: ShowcaseStageVariant,
        show_focus_detail: bool,
    ) -> &'static str {
        match (variant, show_focus_detail) {
            (ShowcaseStageVariant::EntryPath, true) => {
                "The lens isolates what this first move really buys before you commit the route."
            }
            (ShowcaseStageVariant::EntryPath, false) => {
                "The snapshot keeps the wider starting posture visible while the cursor moves across paths."
            }
            (ShowcaseStageVariant::DetectedStartingPoint, true) => {
                "The lens isolates the exact detected setup that will seed the draft if you confirm this path."
            }
            (ShowcaseStageVariant::DetectedStartingPoint, false) => {
                "The snapshot shows which local signals survive the import before the guided deck takes over."
            }
            (ShowcaseStageVariant::ShortcutChoice, true) => {
                "The lens makes the shortcut explicit so a fast handoff still reads like an intentional decision."
            }
            (ShowcaseStageVariant::ShortcutChoice, false) => {
                "The snapshot keeps the current draft in view while you decide between speed and full editing."
            }
            (ShowcaseStageVariant::Generic, true) => {
                "The lens narrows attention to the active card without dropping out of the full-screen shell."
            }
            (ShowcaseStageVariant::Generic, false) => {
                "The snapshot keeps surrounding context visible even on this compatibility path."
            }
        }
    }

    fn showcase_stage_lines(
        variant: ShowcaseStageVariant,
        choices_title: &str,
        intro_lines: &[String],
        selected_index: usize,
        total: usize,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (_, _, _, body, _) = Self::showcase_stage_identity(variant);
        let lead_copy = intro_lines
            .iter()
            .find(|line| !line.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| body.to_owned());
        let detail_copy = if width >= 64 && lead_copy != body {
            Some(body.to_owned())
        } else {
            None
        };

        let mut lines = vec![
            Line::from(Span::styled(
                choices_title.to_owned(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                lead_copy,
                Style::default().fg(palette.secondary_text),
            )),
        ];
        if let Some(detail_copy) = detail_copy {
            lines.push(Line::from(Span::styled(
                detail_copy,
                Style::default().fg(palette.muted_text),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!("Route {} of {}", selected_index + 1, total),
            Style::default().fg(palette.muted_text),
        )));
        lines
    }

    #[cfg(test)]
    fn showcase_compact_sidebar_lines(
        variant: ShowcaseStageVariant,
        choices_title: &str,
        intro_lines: &[String],
        panel_lines: &[String],
        item: &SelectionItem,
        _selected_index: usize,
        _total: usize,
        show_focus_detail: bool,
        _width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (_, _, _, body, _) = Self::showcase_stage_identity(variant);
        let lead_intro = intro_lines
            .iter()
            .find(|line| !line.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| body.to_owned());
        let preview_line = panel_lines
            .iter()
            .find(|line| !line.trim().is_empty())
            .cloned();
        let mut lines = vec![
            Line::from(Span::styled(
                choices_title.to_owned(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        if show_focus_detail {
            lines.push(Line::from(Span::styled(
                item.label.clone(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )));
            if let Some(hint) = item.hint.as_deref() {
                lines.push(Line::from(Span::styled(
                    hint.to_owned(),
                    Style::default().fg(palette.secondary_text),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                body,
                Style::default().fg(palette.secondary_text),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter commits this path.",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "Tab or h returns to the wider overview.",
                Style::default().fg(palette.muted_text),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                item.label.clone(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )));
            if let Some(hint) = item.hint.as_deref() {
                lines.push(Line::from(Span::styled(
                    hint.to_owned(),
                    Style::default().fg(palette.secondary_text),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                lead_intro,
                Style::default().fg(palette.secondary_text),
            )));
            if let Some(preview_line) = preview_line {
                lines.push(Line::from(Span::styled(
                    preview_line,
                    Style::default().fg(palette.muted_text),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter commits this path.",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                "Tab or l inspects the active path.",
                Style::default().fg(palette.muted_text),
            )));
        }
        lines
    }

    #[cfg(test)]
    fn showcase_signal_lines(
        variant: ShowcaseStageVariant,
        panel_title: &str,
        show_focus_detail: bool,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let signal_badge = match variant {
            ShowcaseStageVariant::EntryPath => "FIRST LOCK",
            ShowcaseStageVariant::DetectedStartingPoint => "SIGNAL CHECK",
            ShowcaseStageVariant::ShortcutChoice => "TEMPO CHECK",
            ShowcaseStageVariant::Generic => "DECK LIVE",
        };
        let lens_badge = if show_focus_detail {
            "DETAIL LENS"
        } else {
            "DECK SNAPSHOT"
        };

        let mut lines = vec![
            Line::from(Span::styled(
                "Stage signals",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span(signal_badge, palette.brand),
                Self::filled_badge_span(
                    lens_badge,
                    if show_focus_detail {
                        palette.info
                    } else {
                        palette.success
                    },
                ),
                Self::filled_badge_span(
                    format!("{} OF {}", selected_index + 1, total),
                    palette.warning,
                ),
            ],
            width,
        ));
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                format!("Live path: {}", item.label),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("Panel: {panel_title}"),
                Style::default().fg(palette.secondary_text),
            )),
        ]);
        if let Some(hint) = item.hint.as_deref() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                hint.to_owned(),
                Style::default().fg(palette.secondary_text),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            Self::showcase_signal_copy(variant, show_focus_detail),
            Style::default().fg(palette.muted_text),
        )));
        lines
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn showcase_cue_title(variant: ShowcaseStageVariant) -> &'static str {
        match variant {
            ShowcaseStageVariant::EntryPath => "Opening Rhythm",
            ShowcaseStageVariant::DetectedStartingPoint => "Import Rhythm",
            ShowcaseStageVariant::ShortcutChoice => "Tempo Cue",
            ShowcaseStageVariant::Generic => "Deck Rhythm",
        }
    }

    fn showcase_choices_accent(_variant: ShowcaseStageVariant) -> Color {
        Self::palette().brand
    }

    fn showcase_card_theme(_variant: ShowcaseStageVariant) -> SelectionCardTheme {
        Self::stable_selection_theme()
    }

    fn showcase_control_copy(footer_hint: &str, _show_focus_detail: bool) -> String {
        let compact = "j/k move  1..9 jump  Enter confirm  Esc cancel  ? help";
        let hint = footer_hint.trim();
        let needs_compaction = hint.len() < 24
            || hint.len() > 72
            || hint.contains("Up/Down move")
            || hint.contains("Enter confirm");
        if needs_compaction {
            compact.to_owned()
        } else {
            hint.to_owned()
        }
    }

    #[cfg(test)]
    fn showcase_footer_status_line(
        variant: ShowcaseStageVariant,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
        _show_focus_detail: bool,
    ) -> Line<'static> {
        let palette = Self::palette();
        let stage_label = match variant {
            ShowcaseStageVariant::EntryPath => "entry",
            ShowcaseStageVariant::DetectedStartingPoint => "import",
            ShowcaseStageVariant::ShortcutChoice => "shortcut",
            ShowcaseStageVariant::Generic => "deck",
        };
        Line::from(Span::styled(
            format!(
                "{stage_label} {}/{} · {}",
                selected_index + 1,
                total,
                item.label
            ),
            Style::default().fg(palette.secondary_text),
        ))
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn showcase_uses_poster_layout(variant: ShowcaseStageVariant) -> bool {
        !matches!(variant, ShowcaseStageVariant::Generic)
    }

    #[allow(dead_code)]
    fn showcase_stack_lines(
        variant: ShowcaseStageVariant,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (headline, body, lane_badge) = match variant {
            ShowcaseStageVariant::EntryPath => (
                "Opening routes",
                "Choose whether the deck begins from current state, imported signals, or a fresh draft.",
                "ENTRY LANES",
            ),
            ShowcaseStageVariant::DetectedStartingPoint => (
                "Detected sources",
                "Audit the machine signals here before one of them seeds the imported draft.",
                "SOURCE PICK",
            ),
            ShowcaseStageVariant::ShortcutChoice => (
                "Run-up lanes",
                "This stack decides between immediate momentum and reopening the full editing deck.",
                "TEMPO PICK",
            ),
            ShowcaseStageVariant::Generic => (
                "Selection stack",
                "Keep one explicit route visible while the rest of the deck stays in frame.",
                "DECK PICK",
            ),
        };
        let mut lines = vec![
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span(lane_badge, Self::showcase_choices_accent(variant)),
                Self::filled_badge_span(
                    format!("{} OF {}", selected_index + 1, total),
                    palette.info,
                ),
                Self::filled_badge_span("STACK LIVE", palette.success),
            ],
            width,
        ));
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                format!("Current lane: {}", item.label),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                body,
                Style::default().fg(palette.secondary_text),
            )),
        ]);
        lines
    }

    #[allow(dead_code)]
    fn showcase_cue_lines(
        variant: ShowcaseStageVariant,
        show_focus_detail: bool,
        item: &SelectionItem,
        footer_hint: &str,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (headline, body, commit_badge) = match variant {
            ShowcaseStageVariant::EntryPath => (
                "Set the opening tempo before the guided deck starts narrowing the draft.",
                "This screen should feel like the first ceremonial lock, not a disposable prompt.",
                "ROUTE LOCK",
            ),
            ShowcaseStageVariant::DetectedStartingPoint => (
                "Compare local signals before one imported posture becomes the seed draft.",
                "The deck stays explicit so import still feels audited instead of automatic.",
                "IMPORT LOCK",
            ),
            ShowcaseStageVariant::ShortcutChoice => (
                "Decide whether to keep moving now or reopen the full editing deck.",
                "The fast path still needs to read like a deliberate operator handoff.",
                "FAST COMMIT",
            ),
            ShowcaseStageVariant::Generic => (
                "Keep one live route visible while the deck stays interactive.",
                "Even compatibility flows should preserve the full-screen rhythm.",
                "COMMIT",
            ),
        };
        let lens_badge = if show_focus_detail {
            "PULL BACK"
        } else {
            "ZOOM IN"
        };
        let mut lines = vec![
            Line::from(Span::styled(
                "Control cadence",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span("MOVE", palette.info),
                Self::filled_badge_span(lens_badge, palette.warning),
                Self::filled_badge_span(commit_badge, palette.success),
            ],
            width,
        ));
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                format!("Live card: {}", item.label),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                headline,
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(body, Style::default().fg(palette.muted_text))),
            Line::from(""),
            Line::from(Span::styled(
                Self::showcase_control_copy(footer_hint, show_focus_detail),
                Style::default().fg(palette.secondary_text),
            )),
        ]);
        lines
    }

    #[cfg(test)]
    #[cfg(test)]
    fn showcase_focus_lines(
        variant: ShowcaseStageVariant,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (headline, body, consequence) = match variant {
            ShowcaseStageVariant::EntryPath => (
                "First move",
                "This route decides whether onboarding begins from current machine state, imported signals, or a clean slate.",
                "Enter commits the opening move and turns the rest of the deck into that route's consequences.",
            ),
            ShowcaseStageVariant::DetectedStartingPoint => (
                "Detected posture",
                "This source becomes the seed draft before the guided steps take over.",
                "Confirm only when this imported posture is close enough to trust as the starting draft.",
            ),
            ShowcaseStageVariant::ShortcutChoice => (
                "Tempo decision",
                "This choice decides whether you keep momentum now or drop into the full editing deck.",
                "Use the fast track only when the current draft already reads like the setup you want to keep.",
            ),
            ShowcaseStageVariant::Generic => (
                "Focused path",
                "The active card is the explicit route the deck is preparing to lock next.",
                "Enter commits this route and keeps the transition inside the same full-screen shell.",
            ),
        };
        let mut lines = vec![
            Line::from(Span::styled(
                format!("Choice {} of {}", selected_index + 1, total),
                Style::default().fg(palette.muted_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                item.label.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
        ];
        if let Some(hint) = item.hint.as_deref() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                hint.to_owned(),
                Style::default().fg(palette.secondary_text),
            )));
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "No secondary note is attached to this path.",
                Style::default().fg(palette.muted_text),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            body,
            Style::default().fg(palette.secondary_text),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            consequence,
            Style::default().fg(palette.muted_text),
        )));
        lines
    }

    #[allow(dead_code)]
    fn showcase_snapshot_lines(
        variant: ShowcaseStageVariant,
        panel_lines: &[String],
        item: &SelectionItem,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (headline, cue) = match variant {
            ShowcaseStageVariant::EntryPath => (
                "Starting posture",
                "Press Tab or l to inspect the active route instead of the broader opening snapshot.",
            ),
            ShowcaseStageVariant::DetectedStartingPoint => (
                "Imported signal readout",
                "Press Tab or l to zoom into the specific detected source before you import it.",
            ),
            ShowcaseStageVariant::ShortcutChoice => (
                "Draft momentum snapshot",
                "Press Tab or l to switch from the broader draft readout to the active shortcut lens.",
            ),
            ShowcaseStageVariant::Generic => (
                "Deck snapshot",
                "Press Tab or l to swap this broader snapshot for the active choice lens.",
            ),
        };
        if panel_lines.is_empty() {
            let fallback = item
                .hint
                .as_deref()
                .unwrap_or("No additional snapshot is available for this choice.")
                .to_owned();
            return vec![
                Line::from(Span::styled(
                    headline,
                    Style::default()
                        .fg(palette.brand)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    fallback,
                    Style::default().fg(palette.secondary_text),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    cue,
                    Style::default().fg(palette.secondary_text),
                )),
            ];
        }

        let mut lines = vec![
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        lines.extend(
            panel_lines
                .iter()
                .map(|line| {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(palette.secondary_text),
                    ))
                })
                .collect::<Vec<_>>(),
        );
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            cue,
            Style::default().fg(palette.secondary_text),
        )));
        lines
    }

    #[allow(dead_code)]
    fn showcase_detail_lines(
        variant: ShowcaseStageVariant,
        item: &SelectionItem,
        selected_index: usize,
        total: usize,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (headline, cue) = match variant {
            ShowcaseStageVariant::EntryPath => (
                "Route detail",
                "Press h or Tab to pull back to the broader opening snapshot.",
            ),
            ShowcaseStageVariant::DetectedStartingPoint => (
                "Imported signal detail",
                "Press h or Tab to return to the wider machine signal snapshot.",
            ),
            ShowcaseStageVariant::ShortcutChoice => (
                "Shortcut detail",
                "Press h or Tab to return to the draft-wide momentum snapshot.",
            ),
            ShowcaseStageVariant::Generic => (
                "Choice detail",
                "Press h or Tab to return to the broader deck snapshot.",
            ),
        };
        let mut lines = vec![
            Line::from(Span::styled(
                format!("Lens locked on choice {} of {}", selected_index + 1, total),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                item.label.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
        ];
        if let Some(hint) = item.hint.as_deref() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                hint.to_owned(),
                Style::default().fg(palette.secondary_text),
            )));
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "This option arrives without extra inline detail, so the choice label is the contract.",
                Style::default().fg(palette.secondary_text),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            cue,
            Style::default().fg(palette.secondary_text),
        )));
        lines
    }

    #[allow(dead_code)]
    fn input_snapshot_lines(label: &str, state: &TextInputState) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let normalized = label.trim_end_matches(':');
        let current_value = state.display_value();
        let value_preview = if current_value.is_empty() {
            "empty".to_owned()
        } else {
            current_value.to_owned()
        };
        let mode = if state.is_default_active() {
            "using the current draft as a placeholder"
        } else if state.has_default() {
            "explicit override in progress"
        } else {
            "fresh value in progress"
        };
        let char_count = current_value.chars().count();

        vec![
            Line::from(Span::styled(
                normalized.to_owned(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("Mode: {mode}"),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                format!(
                    "Length: {char_count} character{}",
                    if char_count == 1 { "" } else { "s" }
                ),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Preview",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                value_preview,
                Style::default().fg(palette.text),
            )),
        ]
    }

    fn input_guidance_lines(
        step: OnboardWizardStep,
        label: &str,
        state: &TextInputState,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let guidance = match (step, label) {
            (OnboardWizardStep::Authentication, "Custom model:") => {
                "Use the exact provider model id you want LoongClaw to call."
            }
            (OnboardWizardStep::Authentication, "Provider credential env:") => {
                "Type the env var name only. Keep the secret itself outside the config."
            }
            (OnboardWizardStep::Authentication, "Web search credential env:") => {
                "Type the env var name only. Leave blank if you want to clear this binding."
            }
            (OnboardWizardStep::Workspace, "SQLite path:") => {
                "Choose a stable local path for persisted memory state."
            }
            (OnboardWizardStep::Workspace, "File root:") => {
                "Choose the main local workspace boundary for file tools."
            }
            (OnboardWizardStep::Protocols, _) => {
                "This binding is what the selected channel will read at runtime."
            }
            _ => "Keep this field explicit so the final review reads cleanly.",
        };

        let mode_note = if state.is_default_active() {
            "Press Enter to keep the current draft value, or start typing to replace it."
        } else {
            "Enter submits this exact value."
        };

        vec![
            Line::from(Span::styled(
                guidance,
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                mode_note,
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    fn input_stage_lines(
        step: OnboardWizardStep,
        label: &str,
        state: &TextInputState,
        _width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (headline, body, _) = Self::step_stage_identity(step);
        let mode_copy = if state.is_default_active() {
            "Current draft stays live until you type."
        } else if state.has_default() {
            "Typing here overrides the current draft value."
        } else {
            "This field will be written exactly as shown."
        };
        vec![
            Line::from(Span::styled(
                headline,
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                label.trim_end_matches(':').to_owned(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                body,
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                mode_copy,
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    fn check_level_counts(
        checks: &[crate::onboard_preflight::OnboardCheck],
    ) -> (usize, usize, usize) {
        checks
            .iter()
            .fold((0, 0, 0), |(pass, warn, fail), check| match check.level {
                crate::onboard_preflight::OnboardCheckLevel::Pass => (pass + 1, warn, fail),
                crate::onboard_preflight::OnboardCheckLevel::Warn => (pass, warn + 1, fail),
                crate::onboard_preflight::OnboardCheckLevel::Fail => (pass, warn, fail + 1),
            })
    }

    fn check_signal_meta(
        warning_count: usize,
        failure_count: usize,
    ) -> (&'static str, &'static str, Color) {
        let palette = Self::palette();
        if failure_count > 0 {
            (
                "BLOCKED",
                "Blocking checks remain. Use this gate to inspect the failures before attempting wider rollout.",
                palette.error,
            )
        } else if warning_count > 0 {
            (
                "REVIEW",
                "The draft is viable, but warnings still deserve an explicit operator call before launch.",
                palette.warning,
            )
        } else {
            (
                "CLEAR",
                "All critical checks are green. This gate is now about trust and confirmation, not diagnosis.",
                palette.success,
            )
        }
    }

    fn preflight_summary_lines(
        pass_count: usize,
        warning_count: usize,
        failure_count: usize,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (signal, copy, signal_color) = Self::check_signal_meta(warning_count, failure_count);
        vec![
            Line::from(vec![
                Span::styled(
                    "Signal ",
                    Style::default()
                        .fg(palette.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    signal.to_owned(),
                    Style::default()
                        .fg(signal_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                format!("Counts: {pass_count} pass  {warning_count} warn  {failure_count} fail"),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(copy, Style::default().fg(palette.muted_text))),
        ]
    }

    fn verify_gate_compact_hero_lines(
        hero_status: &str,
        hero_copy: &str,
        pass_count: usize,
        warning_count: usize,
        failure_count: usize,
        config_path: &str,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (signal, _, signal_color) = Self::check_signal_meta(warning_count, failure_count);
        let write_gate_label = if failure_count > 0 {
            "RETURN ONLY"
        } else if warning_count > 0 {
            "WRITE WITH WARNINGS"
        } else {
            "WRITE READY"
        };
        let write_gate_color = if failure_count > 0 {
            palette.error
        } else if warning_count > 0 {
            palette.warning
        } else {
            palette.success
        };
        let enter_copy = if failure_count > 0 {
            "Enter leaves this gate without writing so the draft can be corrected first."
        } else if warning_count > 0 {
            "Enter writes the draft with warnings preserved, then continues to the ready screen."
        } else {
            "Enter writes the draft and continues to the ready screen."
        };
        let mut lines = vec![
            Line::from(Span::styled(
                hero_status.to_owned(),
                Style::default()
                    .fg(signal_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                hero_copy.to_owned(),
                Style::default().fg(palette.secondary_text),
            )),
        ];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span(signal, signal_color),
                Self::filled_badge_span(write_gate_label, write_gate_color),
                Self::filled_badge_span("ESC CANCEL", palette.info),
            ],
            width,
        ));
        lines.extend([
            Line::from(Span::styled(
                format!("Checks: {pass_count} pass  {warning_count} warn  {failure_count} fail"),
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                format!("Write target: {config_path}"),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                enter_copy,
                Style::default().fg(palette.muted_text),
            )),
        ]);
        lines
    }

    fn verify_gate_narrow_hero_lines(
        hero_status: &str,
        pass_count: usize,
        warning_count: usize,
        failure_count: usize,
        config_path: &str,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (signal, _, signal_color) = Self::check_signal_meta(warning_count, failure_count);
        let write_gate_label = if failure_count > 0 {
            "RETURN ONLY"
        } else if warning_count > 0 {
            "WRITE WITH WARNINGS"
        } else {
            "WRITE READY"
        };
        let write_gate_color = if failure_count > 0 {
            palette.error
        } else if warning_count > 0 {
            palette.warning
        } else {
            palette.success
        };
        let concise_copy = if failure_count > 0 {
            "Failing checks remain. Review the draft before writing."
        } else if warning_count > 0 {
            "Warnings remain. Review the draft, then decide whether to write."
        } else {
            "Verification is clear. Review the draft, then write when ready."
        };
        let enter_copy = if failure_count > 0 {
            "Enter returns without writing."
        } else if warning_count > 0 {
            "Enter writes with warnings and continues."
        } else {
            "Enter writes and continues."
        };
        let target_label = Path::new(config_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(config_path);
        let mut lines = vec![
            Line::from(Span::styled(
                hero_status.to_owned(),
                Style::default()
                    .fg(signal_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                concise_copy,
                Style::default().fg(palette.secondary_text),
            )),
        ];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span(signal, signal_color),
                Self::filled_badge_span(write_gate_label, write_gate_color),
            ],
            width,
        ));
        lines.extend([
            Line::from(Span::styled(
                format!("Checks: {pass_count} pass  {warning_count} warn  {failure_count} fail"),
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                format!("Write target: {target_label}"),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                enter_copy,
                Style::default().fg(palette.muted_text),
            )),
        ]);
        lines
    }

    #[allow(dead_code)]
    fn decision_gate_status_lines(
        title: &str,
        scroll_offset: u16,
        total_lines: usize,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let mut lines = vec![Line::from(Span::styled(
            "Operator gate",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ))];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span("READ FIRST", palette.brand),
                Self::filled_badge_span("DECIDE", palette.warning),
                Self::filled_badge_span("NO WRITE", palette.info),
            ],
            width,
        ));
        lines.extend([
            Line::from(Span::styled(
                title.to_owned(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Read the full briefing on the left, then make one explicit decision.",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                "Mode: explicit accept / decline",
                Style::default().fg(palette.muted_text),
            )),
            Line::from(Span::styled(
                format!("Briefing lines: {total_lines}  Scroll offset: {scroll_offset}"),
                Style::default().fg(palette.muted_text),
            )),
        ]);
        lines
    }

    fn confirm_compact_hero_lines(
        title: &str,
        scroll_offset: u16,
        visible_height: u16,
        total_lines: usize,
        footer_hint: &str,
        _logo_status: &str,
        _width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (start, end) = Self::viewed_window(scroll_offset, visible_height, total_lines);
        vec![
            Line::from(Span::styled(
                title.to_owned(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Read the briefing below, then make one explicit accept or decline decision.",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                format!("Briefing window: {start}..{end} of {total_lines}"),
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                footer_hint.to_owned(),
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    fn info_stage_lines(
        title: &str,
        scroll_offset: u16,
        visible_height: u16,
        total_lines: usize,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let reading_window = if total_lines == 0 {
            "Read 0 of 0".to_owned()
        } else {
            let (start, end) = Self::viewed_window(scroll_offset, visible_height, total_lines);
            format!("Read {start}..{end} of {total_lines}")
        };

        vec![
            Line::from(Span::styled(
                title.to_owned(),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                reading_window,
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                "Scroll for details. Enter or Esc returns when you are done reading.",
                Style::default().fg(palette.secondary_text),
            )),
        ]
    }

    fn viewed_window(
        scroll_offset: u16,
        visible_height: u16,
        total_lines: usize,
    ) -> (usize, usize) {
        if total_lines == 0 {
            return (0, 0);
        }

        let start_index = usize::from(scroll_offset).min(total_lines.saturating_sub(1));
        let start = start_index + 1;
        let end =
            (usize::from(scroll_offset) + usize::from(visible_height.max(1))).min(total_lines);
        (start, end.max(start))
    }

    #[cfg(test)]
    fn viewed_progress_badge(
        scroll_offset: u16,
        visible_height: u16,
        total_lines: usize,
    ) -> String {
        if total_lines == 0 {
            return "0% READ".to_owned();
        }

        let (_, end) = Self::viewed_window(scroll_offset, visible_height, total_lines);
        format!("{}% READ", (end * 100) / total_lines)
    }

    #[cfg(test)]
    fn decision_stage_signal_lines(
        scroll_offset: u16,
        visible_height: u16,
        total_lines: usize,
        logo_status: &str,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let (start, end) = Self::viewed_window(scroll_offset, visible_height, total_lines);
        let progress_badge =
            Self::viewed_progress_badge(scroll_offset, visible_height, total_lines);
        let mut lines = vec![
            Line::from(Span::styled(
                "Decision telemetry",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span("YES / NO", palette.warning),
                Self::filled_badge_span(progress_badge, palette.success),
                Self::filled_badge_span("NO WRITE", palette.info),
            ],
            width,
        ));
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                format!("Briefing window: {start}..{end} of {total_lines}"),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Accept advances the deck. Decline returns without silently carrying this gate forward.",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                logo_status.to_owned(),
                Style::default().fg(palette.muted_text),
            )),
        ]);
        lines
    }

    #[cfg(test)]
    fn launch_stage_signal_lines(
        summary: &OnboardingSuccessSummary,
        focus_actions: bool,
        selected_action: Option<&OnboardingAction>,
        logo_status: &str,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let focus_label = if focus_actions {
            "ACTIONS LIVE"
        } else {
            "SETUP LIVE"
        };
        let handoff_label = if selected_action.is_some() {
            "COMMAND ARMED"
        } else {
            "NO COMMAND"
        };
        let handoff_color = if selected_action.is_some() {
            Color::Green
        } else {
            Color::Yellow
        };
        let mut lines = vec![
            Line::from(Span::styled(
                "Handoff telemetry",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        lines.extend(Self::badge_lines(
            [
                Self::launch_outcome_badge(summary.outcome),
                Self::filled_badge_span(focus_label, palette.brand),
                Self::filled_badge_span(handoff_label, handoff_color),
            ],
            width,
        ));
        lines.push(Line::from(""));
        match selected_action {
            Some(action) => {
                lines.push(Line::from(Span::styled(
                    format!("Armed command: {}", action.command),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    "Enter exits the deck and echoes this command back into the shell.",
                    Style::default().fg(Color::Gray),
                )));
            }
            None => {
                lines.push(Line::from(Span::styled(
                    "No command is currently armed for shell handoff.",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    "Review the saved setup, or move focus back to actions before leaving the deck.",
                    Style::default().fg(Color::Gray),
                )));
            }
        }
        lines.push(Line::from(Span::styled(
            logo_status.to_owned(),
            Style::default().fg(Color::DarkGray),
        )));
        lines
    }

    fn brand_logo_base64() -> &'static str {
        static BRAND_LOGO_BASE64: OnceLock<String> = OnceLock::new();
        BRAND_LOGO_BASE64.get_or_init(|| {
            BASE64_STANDARD.encode(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/logo/loongclaw-icon.png"
            )))
        })
    }

    #[allow(dead_code)]
    fn brand_icon_base64() -> &'static str {
        Self::brand_logo_base64()
    }

    #[allow(dead_code)]
    fn render_brand_mark(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        title: &str,
        inline_logo: bool,
        tags: &[BrandMarkTag],
    ) -> Option<Rect> {
        let border_color = Self::palette().brand;
        let fill_style = Self::panel_fill_style(border_color);
        let block = Self::rounded_panel(title, border_color).style(fill_style);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let footer_lines = Self::brand_mark_footer_lines(tags, inline_logo, inner.width);
        let footer_height = u16::try_from(footer_lines.len()).ok().unwrap_or(0);
        let reserve_footer = if footer_height == 0 {
            false
        } else if inline_logo {
            inner.height > INLINE_LOGO_MIN_HEIGHT.saturating_add(footer_height)
        } else {
            false
        };
        let (media_area, footer_area) = if reserve_footer {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(footer_height)])
                .split(inner);
            (split[0], Some(split[1]))
        } else {
            (inner, None)
        };

        if inline_logo
            && media_area.width >= INLINE_LOGO_MIN_WIDTH
            && media_area.height >= INLINE_LOGO_MIN_HEIGHT
        {
            if let Some(footer_area) = footer_area {
                frame.render_widget(
                    Paragraph::new(footer_lines)
                        .alignment(Alignment::Center)
                        .style(fill_style),
                    footer_area,
                );
            }
            Some(media_area)
        } else {
            frame.render_widget(
                Paragraph::new(Self::brand_mark_fallback_lines(
                    tags,
                    inline_logo,
                    media_area.width,
                ))
                .alignment(Alignment::Center)
                .style(fill_style),
                media_area,
            );
            None
        }
    }

    fn paint_inline_image(
        &mut self,
        area: Rect,
        min_width: u16,
        min_height: u16,
        encoded: &str,
    ) -> CliResult<()> {
        let Some(support) = self.inline_logo_support() else {
            return Ok(());
        };
        if area.width < min_width || area.height < min_height {
            return Ok(());
        }

        let raw_sequence = match support.protocol {
            InlineLogoProtocol::Kitty => format!(
                "\u{1b}_Gq=2,a=T,f=100,c={},r={};{}\u{1b}\\",
                area.width.saturating_sub(1),
                area.height,
                encoded
            ),
            InlineLogoProtocol::Iterm2 => format!(
                "\u{1b}]1337;File=inline=1;preserveAspectRatio=1;width={};height={}:{}\u{7}",
                area.width.saturating_sub(1),
                area.height,
                encoded
            ),
        };
        let sequence = if support.tmux_passthrough {
            let escaped = raw_sequence.replace('\u{1b}', "\u{1b}\u{1b}");
            format!("\u{1b}Ptmux;{escaped}\u{1b}\\")
        } else {
            raw_sequence
        };

        let backend = self.terminal.backend_mut();
        execute!(backend, MoveTo(area.x, area.y), Print(sequence), Hide)
            .map_err(|e| e.to_string())?;
        backend.flush().map_err(|e| e.to_string())
    }

    fn paint_inline_logo(&mut self, area: Rect) -> CliResult<()> {
        self.paint_inline_image(
            area,
            INLINE_LOGO_MIN_WIDTH,
            INLINE_LOGO_MIN_HEIGHT,
            Self::brand_logo_base64(),
        )
    }

    #[allow(dead_code)]
    fn paint_inline_icon(&mut self, area: Rect) -> CliResult<()> {
        self.paint_inline_image(
            area,
            INLINE_ICON_MIN_WIDTH,
            INLINE_ICON_MIN_HEIGHT,
            Self::brand_icon_base64(),
        )
    }

    fn help_binding_line(key: &str, detail: &str) -> Line<'static> {
        let palette = Self::palette();
        Line::from(vec![
            Span::styled(
                format!("{key:<12}"),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                detail.to_owned(),
                Style::default().fg(palette.secondary_text),
            ),
        ])
    }

    fn help_note_line(detail: &str) -> Line<'static> {
        let palette = Self::palette();
        Line::from(Span::styled(
            detail.to_owned(),
            Style::default().fg(palette.muted_text),
        ))
    }

    fn help_overlay_lines(topic: HelpOverlayTopic) -> (&'static str, Vec<Line<'static>>) {
        match topic {
            HelpOverlayTopic::Welcome => (
                "Hero Controls",
                vec![
                    Self::help_binding_line("Enter", "open the guided setup deck"),
                    Self::help_binding_line("Esc", "cancel onboarding before any write"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                    Line::from(""),
                    Self::help_note_line(
                        "The opening screen is deliberately ceremonial: hero first, cockpit second.",
                    ),
                ],
            ),
            HelpOverlayTopic::Selection => (
                "Selection Controls",
                vec![
                    Self::help_binding_line("j / k", "move between choices"),
                    Self::help_binding_line("1..9", "jump directly to a visible choice"),
                    Self::help_binding_line("PgUp / PgDn", "jump through the choice stack"),
                    Self::help_binding_line("g / G", "jump to the first or last choice"),
                    Self::help_binding_line("Enter", "confirm the highlighted choice"),
                    Self::help_binding_line("Esc", "go back to the previous guided step"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                ],
            ),
            HelpOverlayTopic::Input => (
                "Input Controls",
                vec![
                    Self::help_binding_line("Type", "edit the current value"),
                    Self::help_binding_line("← / →", "move the cursor"),
                    Self::help_binding_line("Backspace", "delete before the cursor"),
                    Self::help_binding_line("Ctrl+a / Ctrl+e", "jump to the start or end"),
                    Self::help_binding_line("Ctrl+u", "clear the current value"),
                    Self::help_binding_line("Enter", "accept the current value"),
                    Self::help_binding_line("Esc", "return to the previous field"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                ],
            ),
            HelpOverlayTopic::Showcase => (
                "Showcase Controls",
                vec![
                    Self::help_binding_line("j / k", "move between starting-point options"),
                    Self::help_binding_line("1..9", "jump directly to a visible choice"),
                    Self::help_binding_line("PgUp / PgDn", "jump through the choice stack"),
                    Self::help_binding_line("g / G", "jump to the first or last choice"),
                    Self::help_binding_line("Enter", "lock in the current choice"),
                    Self::help_binding_line("Esc", "leave this showcase screen"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                    Line::from(""),
                    Self::help_note_line(
                        "These entry screens are intentionally centered: one brand center, one choice stack, one clear commit.",
                    ),
                ],
            ),
            HelpOverlayTopic::Confirm => (
                "Confirm Controls",
                vec![
                    Self::help_binding_line(
                        "j / k",
                        "scroll the briefing when it is taller than one panel",
                    ),
                    Self::help_binding_line("PgUp / PgDn", "scroll the briefing by one page"),
                    Self::help_binding_line("g / G", "jump to the top or bottom of the briefing"),
                    Self::help_binding_line("Enter / y", "accept and continue"),
                    Self::help_binding_line("n / Esc", "decline or cancel"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                ],
            ),
            HelpOverlayTopic::Info => (
                "Info Controls",
                vec![
                    Self::help_binding_line("j / k", "scroll long output"),
                    Self::help_binding_line("PgUp / PgDn", "scroll by one page"),
                    Self::help_binding_line("g / G", "jump to the top or bottom"),
                    Self::help_binding_line("Enter", "continue or exit the screen"),
                    Self::help_binding_line("Esc", "close the screen"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                ],
            ),
            HelpOverlayTopic::VerifyWrite => (
                "Verify & Write Controls",
                vec![
                    Self::help_binding_line("j / k", "scroll the review"),
                    Self::help_binding_line("PgUp / PgDn", "scroll the review by one page"),
                    Self::help_binding_line("g / G", "jump to the top or bottom of the review"),
                    Self::help_binding_line("Enter", "write when clear, or return when blocked"),
                    Self::help_binding_line("n / Esc", "cancel before writing"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                    Line::from(""),
                    Self::help_note_line(
                        "Checks and draft live in one review stream so the write decision stays readable.",
                    ),
                ],
            ),
            HelpOverlayTopic::Launch => (
                "Launch Controls",
                vec![
                    Self::help_binding_line("j / k", "scroll the saved setup"),
                    Self::help_binding_line("PgUp / PgDn", "page through the saved setup"),
                    Self::help_binding_line("g / G", "jump to the top or bottom of the setup"),
                    Self::help_binding_line("Enter / Esc", "leave the ready screen"),
                    Self::help_binding_line("?", "toggle this help overlay"),
                    Line::from(""),
                    Self::help_note_line(
                        "Configured channels keep their own runtime surfaces while local chat stays available here.",
                    ),
                ],
            ),
        }
    }

    fn render_help_overlay(frame: &mut ratatui::Frame<'_>, area: Rect, topic: HelpOverlayTopic) {
        let palette = Self::palette();
        let (title, lines) = Self::help_overlay_lines(topic);
        let content_height = u16::try_from(lines.len())
            .ok()
            .unwrap_or(1)
            .saturating_add(2);
        let width = area.width.saturating_sub(8).clamp(34, 78);
        let height = content_height.min(area.height.saturating_sub(2)).max(8);
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let rect = Rect::new(x, y, width, height);

        frame.render_widget(Clear, rect);
        let fill_style = Self::panel_fill_style(palette.warning);
        let block = Self::rounded_panel(title, palette.warning).style(fill_style);
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Left)
                .wrap(Wrap { trim: false })
                .style(fill_style),
            inner,
        );
    }

    fn handle_help_overlay_event(show_help: &mut bool, event: &Event) -> CliResult<bool> {
        match event {
            Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers,
                ..
            }) if modifiers.contains(KeyModifiers::CONTROL) => {
                Err("interrupted by user".to_owned())
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char('?'),
                ..
            }) => {
                *show_help = !*show_help;
                Ok(true)
            }
            _ if *show_help => match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Esc | KeyCode::Enter,
                    ..
                }) => {
                    *show_help = false;
                    Ok(true)
                }
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => Ok(true),
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => Ok(true),
            },
            Event::Resize(..)
            | Event::FocusGained
            | Event::FocusLost
            | Event::Key(_)
            | Event::Mouse(_)
            | Event::Paste(_) => Ok(false),
        }
    }

    fn render_guided_shell_with_header<F>(
        &mut self,
        step: OnboardWizardStep,
        title: &str,
        header_hint: &str,
        footer_hint: &str,
        mut render_content: F,
    ) -> CliResult<()>
    where
        F: FnMut(&mut ratatui::Frame<'_>, Rect),
    {
        let step_number = step_ordinal(step);
        let title_owned = title.to_owned();
        let header_hint_owned = header_hint.to_owned();
        let hint_owned = footer_hint.to_owned();

        self.terminal
            .draw(|frame| {
                if Self::is_terminal_too_small(frame.area()) {
                    Self::render_too_small_fallback(frame);
                    return;
                }

                frame.render_widget(Clear, frame.area());
                let areas = layout::compute_layout(frame.area(), true);
                frame.render_widget(
                    Paragraph::new(Self::screen_header_line(&title_owned, &header_hint_owned)),
                    areas.header,
                );
                frame.render_widget(ProgressSpineWidget::new(step), areas.spine);
                let content_area = Self::shell_content_area(areas.content);
                render_content(frame, content_area);
                let footer_line =
                    Self::guided_footer_line(step_number, &hint_owned, areas.footer.width);
                frame.render_widget(Paragraph::new(footer_line), areas.footer);
            })
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    fn render_no_spine_shell_with_header<F>(
        &mut self,
        title: &str,
        header_hint: &str,
        footer_hint: &str,
        mut render_content: F,
    ) -> CliResult<()>
    where
        F: FnMut(&mut ratatui::Frame<'_>, Rect),
    {
        let title_owned = title.to_owned();
        let header_hint_owned = header_hint.to_owned();
        let hint_owned = footer_hint.to_owned();

        self.terminal
            .draw(|frame| {
                if Self::is_terminal_too_small(frame.area()) {
                    Self::render_too_small_fallback(frame);
                    return;
                }

                frame.render_widget(Clear, frame.area());
                let areas = layout::compute_layout(frame.area(), false);
                frame.render_widget(
                    Paragraph::new(Self::screen_header_line(&title_owned, &header_hint_owned)),
                    areas.header,
                );
                let content_area = Self::shell_content_area(areas.content);
                render_content(frame, content_area);
                frame.render_widget(
                    Paragraph::new(Self::shell_footer_line(&hint_owned)),
                    areas.footer,
                );
            })
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    #[allow(dead_code)] // retained for incremental migration of older guided loops
    fn render_guided_shell<F>(
        &mut self,
        step: OnboardWizardStep,
        title: &str,
        footer_hint: &str,
        render_content: F,
    ) -> CliResult<()>
    where
        F: FnMut(&mut ratatui::Frame<'_>, Rect),
    {
        self.render_guided_shell_with_header(step, title, "Esc back", footer_hint, render_content)
    }

    fn render_showcase_shell(
        &mut self,
        variant: ShowcaseStageVariant,
        title: &str,
        intro_lines: &[&str],
        choices_title: &str,
        _panel_lines: &[String],
        items: &[SelectionItem],
        state: &mut SelectionCardState,
        footer_hint: &str,
        show_help: bool,
    ) -> CliResult<Option<Rect>> {
        let title_owned = title.to_owned();
        let footer_hint_owned = Self::showcase_control_copy(footer_hint, false);
        let intro_lines_owned = intro_lines
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>();
        let palette = Self::palette();
        let choices_title_owned = choices_title.to_owned();
        let captured_logo_area = None;
        self.terminal
            .draw(|frame| {
                if Self::is_terminal_too_small(frame.area()) {
                    Self::render_too_small_fallback(frame);
                    return;
                }

                frame.render_widget(Clear, frame.area());
                let shell = Self::showcase_shell_sections(frame.area());
                frame.render_widget(
                    Paragraph::new(Self::screen_header_line(&title_owned, "Esc cancel  ? help")),
                    shell[0],
                );

                let selected_index = state.selected();
                let selection_height =
                    Self::focus_selection_panel_height(shell[2].height, items.len());
                let stage_lines = Self::showcase_stage_lines(
                    variant,
                    &choices_title_owned,
                    &intro_lines_owned,
                    selected_index,
                    items.len(),
                    shell[2].width.saturating_sub(12),
                );
                let main_width = shell[2].width.saturating_sub(4).min(92);
                let stage_height =
                    Self::guided_brand_stage_desired_height(main_width, &stage_lines, &[]);
                let max_stage_height = shell[2].height.saturating_sub(selection_height).max(1);
                let stage_height = stage_height.min(max_stage_height);
                let main_area = Self::centered_rect(shell[2], main_width, shell[2].height);
                let main = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(stage_height),
                        Constraint::Min(selection_height),
                    ])
                    .split(main_area);
                Self::render_guided_brand_stage(
                    frame,
                    main[0],
                    &stage_lines,
                    &[],
                    Alignment::Center,
                    Alignment::Center,
                    72,
                    0,
                );

                let widget = SelectionCardWidget::new(
                    items
                        .iter()
                        .map(|i| SelectionItem::new(i.label.as_str(), i.hint.as_deref()))
                        .collect(),
                )
                .with_theme(Self::showcase_card_theme(variant))
                .with_layout(SelectionCardLayout::Framed);
                let selection_area = Self::centered_rect(
                    main[1],
                    main[1].width.saturating_sub(4).min(88),
                    main[1].height,
                );
                frame.render_stateful_widget(widget, selection_area, state);

                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(" {footer_hint_owned} "),
                        Style::default().fg(palette.secondary_text),
                    )))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: false }),
                    shell[3],
                );

                if show_help {
                    Self::render_help_overlay(frame, shell[2], HelpOverlayTopic::Showcase);
                }
            })
            .map_err(|e| e.to_string())?;

        Ok(captured_logo_area)
    }

    // -----------------------------------------------------------------------
    // Welcome step
    // -----------------------------------------------------------------------

    fn run_welcome_step(&mut self) -> CliResult<OnboardFlowStepAction> {
        if self.skip_next_guided_welcome {
            self.skip_next_guided_welcome = false;
            return Ok(OnboardFlowStepAction::Next);
        }

        let version = mvp::presentation::BuildVersionInfo::current().render_version_line();
        let inline_logo_enabled = self.inline_logo_protocol().is_some();
        let mut show_help = false;
        loop {
            let ver = version.clone();
            let primary_lines = Self::welcome_primary_lines();
            let support_lines = Self::welcome_support_lines_for_route("");
            let mut captured_logo_area = None;
            let show_help_now = show_help;
            self.render_no_spine_shell_with_header(
                "hero",
                "Esc cancel  ? help",
                "Enter begin  ? help  Esc cancel",
                |frame, content_area| {
                    captured_logo_area = Self::render_welcome_centerpiece(
                        frame,
                        content_area,
                        &ver,
                        &primary_lines,
                        &support_lines,
                        inline_logo_enabled,
                    );

                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Welcome);
                    }
                },
            )?;
            if let Some(logo_area) = captured_logo_area {
                self.paint_inline_logo(logo_area)?;
            }

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(OnboardFlowStepAction::Next),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Err("onboarding cancelled".to_owned()),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Generic selection loop
    // -----------------------------------------------------------------------

    fn run_selection_loop(
        &mut self,
        step: OnboardWizardStep,
        title: &str,
        items: Vec<SelectionItem>,
        default_index: usize,
        footer_hint: &str,
    ) -> CliResult<SelectionLoopResult> {
        if items.is_empty() {
            return Err("no items to select from".to_owned());
        }

        let mut state = SelectionCardState::new(items.len());
        state.select(default_index);
        let mut show_help = false;
        let selection_theme = Self::selection_card_theme_for_step(step);

        loop {
            let footer_with_help = format!("{footer_hint}  1..9 jump  g/G edge  ? help");
            let show_help_now = show_help;
            let stage_lines =
                Self::selection_stage_lines(step, title, state.selected(), items.len(), 0);
            self.render_no_spine_shell_with_header(
                title,
                "Esc back  ? help",
                &footer_with_help,
                |frame, content_area| {
                    let shell =
                        Self::selection_shell_sections(content_area, &stage_lines, items.len());

                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &stage_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        72,
                        0,
                    );

                    let selection_inner = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(4).min(88),
                        shell[1].height,
                    );
                    let widget = SelectionCardWidget::new(
                        items
                            .iter()
                            .map(|i| SelectionItem::new(i.label.as_str(), i.hint.as_deref()))
                            .collect(),
                    )
                    .with_theme(selection_theme)
                    .with_layout(SelectionCardLayout::Framed);
                    frame.render_stateful_widget(widget, selection_inner, &mut state);
                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Selection);
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(SelectionLoopResult::Selected(state.selected())),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => state.next(),
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => state.previous(),
                Event::Key(KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                }) => {
                    for _ in 0..3 {
                        state.next();
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                }) => {
                    for _ in 0..3 {
                        state.previous();
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Home | KeyCode::Char('g'),
                    ..
                }) => state.select_first(),
                Event::Key(KeyEvent {
                    code: KeyCode::End | KeyCode::Char('G'),
                    ..
                }) => state.select_last(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    ..
                }) if ('1'..='9').contains(&c) => {
                    let idx = usize::from(c as u8 - b'1');
                    if idx < items.len() {
                        state.select(idx);
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(SelectionLoopResult::Back),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    fn run_multi_selection_loop(
        &mut self,
        step: OnboardWizardStep,
        title: &str,
        items: Vec<SelectionItem>,
        default_index: usize,
        checked_indices: Vec<usize>,
        footer_hint: &str,
    ) -> CliResult<MultiSelectionLoopResult> {
        if items.is_empty() {
            return Err("no items to select from".to_owned());
        }

        let mut state = SelectionCardState::new(items.len());
        state.select(default_index);
        state.set_checked_indices(checked_indices);
        let mut show_help = false;
        let selection_theme = Self::selection_card_theme_for_step(step);

        loop {
            let footer_with_help = format!("{footer_hint}  1..9 jump  g/G edge  ? help");
            let show_help_now = show_help;
            let stage_lines =
                Self::selection_stage_lines(step, title, state.selected(), items.len(), 0);
            self.render_no_spine_shell_with_header(
                title,
                "Esc back  ? help",
                &footer_with_help,
                |frame, content_area| {
                    let shell =
                        Self::selection_shell_sections(content_area, &stage_lines, items.len());

                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &stage_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        72,
                        0,
                    );

                    let selection_inner = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(4).min(88),
                        shell[1].height,
                    );
                    let widget = SelectionCardWidget::new(
                        items
                            .iter()
                            .map(|item| {
                                SelectionItem::new(item.label.as_str(), item.hint.as_deref())
                            })
                            .collect(),
                    )
                    .with_theme(selection_theme)
                    .with_layout(SelectionCardLayout::Framed);
                    frame.render_stateful_widget(widget, selection_inner, &mut state);
                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Selection);
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(MultiSelectionLoopResult::Submitted(state.checked_indices())),
                Event::Key(KeyEvent {
                    code: KeyCode::Char(' '),
                    ..
                }) => state.toggle_selected(),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => state.next(),
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => state.previous(),
                Event::Key(KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                }) => {
                    for _ in 0..3 {
                        state.next();
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                }) => {
                    for _ in 0..3 {
                        state.previous();
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Home | KeyCode::Char('g'),
                    ..
                }) => state.select_first(),
                Event::Key(KeyEvent {
                    code: KeyCode::End | KeyCode::Char('G'),
                    ..
                }) => state.select_last(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    ..
                }) if ('1'..='9').contains(&c) => {
                    let index = usize::from(c as u8 - b'1');
                    if index < items.len() {
                        state.select(index);
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(MultiSelectionLoopResult::Back),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => {}
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Generic text input loop
    // -----------------------------------------------------------------------

    fn run_input_loop(
        &mut self,
        step: OnboardWizardStep,
        label: &str,
        default_value: &str,
        footer_hint: &str,
    ) -> CliResult<InputLoopResult> {
        let mut input_state = if default_value.is_empty() {
            TextInputState::new()
        } else {
            TextInputState::with_default(default_value)
        };
        let mut show_help = false;
        let palette = Self::palette();

        loop {
            let footer_with_help = format!("{footer_hint}  Ctrl+u clear  Ctrl+a/e edge  ? help");
            let show_help_now = show_help;
            let stage_lines = Self::input_stage_lines(step, label, &input_state, 28);
            let guidance_lines = Self::input_guidance_lines(step, label, &input_state);
            self.render_no_spine_shell_with_header(
                label,
                "Esc back  ? help",
                &footer_with_help,
                |frame, content_area| {
                    let edit_height = 5_u16;
                    let stage_height = Self::guided_brand_stage_desired_height(
                        content_area.width,
                        &stage_lines,
                        &[],
                    );
                    let max_stage_height = content_area.height.saturating_sub(edit_height).max(1);
                    let stage_height = stage_height.min(max_stage_height);
                    let shell = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(stage_height), Constraint::Length(5)])
                        .split(content_area);

                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &stage_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        72,
                        0,
                    );

                    let edit_area = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(6).min(84),
                        shell[1].height,
                    );
                    let edit_block = Self::rounded_panel("Edit", palette.brand)
                        .style(Self::panel_fill_style(palette.brand));
                    let edit_inner = Self::inset_rect(edit_block.inner(edit_area), 1, 0);
                    frame.render_widget(edit_block, edit_area);
                    let edit_layout = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(1), Constraint::Min(1)])
                        .split(edit_inner);
                    let widget = TextInputWidget::new(label).without_label();
                    widget.render_with_state(edit_layout[0], frame.buffer_mut(), &input_state);
                    let guidance_line = guidance_lines
                        .first()
                        .cloned()
                        .unwrap_or_else(Line::default);
                    let controls_line = Line::from(Span::styled(
                        "Enter keeps or submits. Ctrl+u clears. Ctrl+a / Ctrl+e jump to the edges.",
                        Style::default().fg(palette.muted_text),
                    ));
                    frame.render_widget(
                        Paragraph::new(vec![guidance_line, controls_line])
                            .wrap(Wrap { trim: false }),
                        edit_layout[1],
                    );
                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Input);
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    let value = input_state.submit_value().to_owned();
                    return Ok(InputLoopResult::Submitted(value));
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                }) => input_state.backspace(),
                Event::Key(KeyEvent {
                    code: KeyCode::Delete,
                    ..
                }) => input_state.delete(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('u'),
                    modifiers,
                    ..
                }) if modifiers.contains(KeyModifiers::CONTROL) => input_state.clear(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('a'),
                    modifiers,
                    ..
                }) if modifiers.contains(KeyModifiers::CONTROL) => input_state.move_home(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('e'),
                    modifiers,
                    ..
                }) if modifiers.contains(KeyModifiers::CONTROL) => input_state.move_end(),
                Event::Key(KeyEvent {
                    code: KeyCode::Left,
                    ..
                }) => input_state.move_left(),
                Event::Key(KeyEvent {
                    code: KeyCode::Right,
                    ..
                }) => input_state.move_right(),
                Event::Key(KeyEvent {
                    code: KeyCode::Home,
                    ..
                }) => input_state.move_home(),
                Event::Key(KeyEvent {
                    code: KeyCode::End, ..
                }) => input_state.move_end(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    ..
                }) => input_state.push(c),
                Event::Paste(text) => {
                    for character in text.chars() {
                        input_state.push(character);
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(InputLoopResult::Back),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // OpenAI Codex OAuth screen
    // -----------------------------------------------------------------------

    fn openai_codex_oauth_stage_lines() -> Vec<Line<'static>> {
        let palette = Self::palette();
        vec![
            Line::from(Span::styled(
                "Browser authorization",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "OpenAI Codex OAuth",
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Authorize with your ChatGPT or Codex account, then return here automatically once the browser callback lands.",
                Style::default().fg(palette.secondary_text),
            )),
        ]
    }

    fn openai_codex_oauth_support_lines(
        callback_redirect_uri: &str,
        authorization_url: &str,
        last_error: Option<&str>,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let mut lines = vec![
            Line::from(Span::styled(
                "Press Enter to open the browser flow and wait for the localhost callback.",
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                "Press p to paste the final redirect URL or authorization code if the callback does not land.",
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                format!("Callback: {callback_redirect_uri}"),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                "Authorization link",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                authorization_url.to_owned(),
                Style::default().fg(palette.muted_text),
            )),
            Line::from(Span::styled(
                "Esc returns to provider setup.",
                Style::default().fg(palette.muted_text),
            )),
        ];

        if let Some(last_error) = last_error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Last attempt",
                Style::default()
                    .fg(palette.warning)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                last_error.to_owned(),
                Style::default().fg(palette.secondary_text),
            )));
        }

        lines
    }

    fn run_openai_codex_oauth_loop(&mut self) -> CliResult<OpenaiCodexOauthLoopResult> {
        let start = self.openai_codex_oauth_start;
        let mut flow = start()?;
        let mut last_error = None;
        let mut show_help = false;

        loop {
            let callback_redirect_uri = flow.callback_redirect_uri().to_owned();
            let authorization_url = flow.authorization_url().to_owned();
            let header_hint = "Enter open  P paste  Esc back  ? help";
            let footer_hint = "Enter launch browser  P paste redirect  Esc back  ? help";
            let stage_lines = Self::openai_codex_oauth_stage_lines();
            let support_lines = Self::openai_codex_oauth_support_lines(
                &callback_redirect_uri,
                &authorization_url,
                last_error.as_deref(),
            );
            let show_help_now = show_help;

            self.render_no_spine_shell_with_header(
                "Codex OAuth",
                header_hint,
                footer_hint,
                |frame, content_area| {
                    let support_height = u16::try_from(support_lines.len()).ok().unwrap_or(0);
                    let reserved_height = support_height.saturating_add(2);
                    let stage_height = Self::guided_brand_stage_desired_height(
                        content_area.width,
                        &stage_lines,
                        &[],
                    );
                    let max_stage_height =
                        content_area.height.saturating_sub(reserved_height).max(1);
                    let stage_height = stage_height.min(max_stage_height);
                    let shell = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(stage_height),
                            Constraint::Length(support_height.saturating_add(2)),
                            Constraint::Min(0),
                        ])
                        .split(content_area);

                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &stage_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        72,
                        0,
                    );

                    let support_area = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(6).min(78),
                        shell[1].height,
                    );
                    frame.render_widget(
                        Paragraph::new(support_lines.clone())
                            .alignment(Alignment::Center)
                            .wrap(Wrap { trim: false }),
                        support_area,
                    );

                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Info);
                    }
                },
            )?;

            let event = self
                .event_source
                .next_event()
                .map_err(|error| error.to_string())?;
            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    let open_result = flow.open_browser();
                    if let Err(error) = open_result {
                        last_error = Some(error);
                        show_help = false;
                        continue;
                    }

                    match flow.wait_for_browser_callback() {
                        Ok(grant) => {
                            return Ok(OpenaiCodexOauthLoopResult::Authorized(grant));
                        }
                        Err(error) => {
                            last_error = Some(error);
                            show_help = false;
                        }
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Char('p') | KeyCode::Char('P'),
                    ..
                }) => match self.run_input_loop(
                    OnboardWizardStep::Authentication,
                    "Paste redirect URL or code:",
                    "",
                    "Enter confirm  Esc back",
                )? {
                    InputLoopResult::Back => {}
                    InputLoopResult::Submitted(input) => {
                        match flow.complete_from_manual_input(&input) {
                            Ok(grant) => {
                                return Ok(OpenaiCodexOauthLoopResult::Authorized(grant));
                            }
                            Err(error) => {
                                last_error = Some(error);
                                show_help = false;
                            }
                        }
                    }
                },
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => {
                    if show_help {
                        show_help = false;
                    } else {
                        return Ok(OpenaiCodexOauthLoopResult::Back);
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Char('?'),
                    ..
                }) => {
                    show_help = !show_help;
                }
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => {}
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Authentication step
    // -----------------------------------------------------------------------

    fn run_authentication_step(
        &mut self,
        draft: &mut OnboardDraft,
    ) -> CliResult<OnboardFlowStepAction> {
        let mut sub_step: u8 = 0;
        let mut selected_provider_options = Vec::new();
        let mut active_provider_profile_id = String::new();
        let mut web_search_return_step: u8 = 1;
        let mut selected_web_search_provider_ids = Vec::new();
        let mut default_web_search_provider_id = String::new();
        let mut web_search_credential_index = 0usize;
        loop {
            match sub_step {
                0 => match self.run_provider_selection_loop(&draft.config.provider)? {
                    None => return Ok(OnboardFlowStepAction::Back),
                    Some(options) => {
                        active_provider_profile_id = Self::default_active_provider_profile_id(
                            &draft.config.provider,
                            &options,
                        );
                        selected_provider_options = options;
                        sub_step = 1;
                    }
                },
                1 => {
                    match self
                        .run_selected_provider_configuration_sequence(&selected_provider_options)?
                    {
                        None => {
                            sub_step = 0;
                        }
                        Some(configured_options) => {
                            selected_provider_options = configured_options;
                            if selected_provider_options.len() == 1 {
                                if let Some(option) = selected_provider_options.first() {
                                    active_provider_profile_id = option.profile_id.clone();
                                }
                                Self::apply_selected_provider_profiles(
                                    draft,
                                    &selected_provider_options,
                                    active_provider_profile_id.as_str(),
                                )?;
                                web_search_return_step = 1;
                                sub_step = 3;
                            } else {
                                active_provider_profile_id =
                                    Self::default_active_provider_profile_id(
                                        &draft.config.provider,
                                        &selected_provider_options,
                                    );
                                sub_step = 2;
                            }
                        }
                    }
                }
                2 => {
                    match self.run_default_provider_selection_loop(
                        &selected_provider_options,
                        active_provider_profile_id.as_str(),
                    )? {
                        None => {
                            sub_step = 1;
                        }
                        Some(profile_id) => {
                            active_provider_profile_id = profile_id;
                            Self::apply_selected_provider_profiles(
                                draft,
                                &selected_provider_options,
                                active_provider_profile_id.as_str(),
                            )?;
                            web_search_return_step = 2;
                            sub_step = 3;
                        }
                    }
                }
                3 => {
                    let options = Self::web_search_picker_options(&draft.config);
                    let items = options
                        .iter()
                        .map(|option| option.item.clone())
                        .collect::<Vec<_>>();
                    let default_index =
                        Self::current_web_search_picker_index(&draft.config, &options);
                    let initial_focus_index = 0usize;
                    let checked_indices = Self::web_search_picker_checked_indices(
                        &draft.config,
                        &options,
                        &selected_web_search_provider_ids,
                    );
                    match self.run_multi_selection_loop(
                        OnboardWizardStep::Authentication,
                        "Web Search",
                        items,
                        initial_focus_index,
                        checked_indices,
                        "Space toggle  Enter confirm",
                    )? {
                        MultiSelectionLoopResult::Back => {
                            sub_step = web_search_return_step;
                        }
                        MultiSelectionLoopResult::Submitted(indices) => {
                            let mut selected_provider_ids =
                                Self::selected_web_search_provider_ids(&options, &indices);
                            if selected_provider_ids.is_empty() {
                                let fallback_option =
                                    options.get(default_index).ok_or_else(|| {
                                        "invalid web search provider selection".to_owned()
                                    })?;
                                selected_provider_ids.push(fallback_option.id.to_owned());
                            }

                            let current_provider =
                                crate::onboard_web_search::current_web_search_provider(
                                    &draft.config,
                                );
                            let keep_current_default = selected_provider_ids
                                .iter()
                                .any(|provider_id| provider_id == current_provider);
                            let keep_selected_default = selected_provider_ids
                                .iter()
                                .any(|provider_id| provider_id == &default_web_search_provider_id);
                            if keep_selected_default {
                                // Preserve the prior default when the selected set still contains it.
                            } else if keep_current_default {
                                default_web_search_provider_id = current_provider.to_owned();
                            } else {
                                let first_provider = selected_provider_ids
                                    .first()
                                    .cloned()
                                    .ok_or_else(|| "missing web search provider".to_owned())?;
                                default_web_search_provider_id = first_provider;
                            }

                            selected_web_search_provider_ids = selected_provider_ids;
                            web_search_credential_index = 0;
                            Self::apply_web_search_provider_selection(
                                draft,
                                &options,
                                &selected_web_search_provider_ids,
                                default_web_search_provider_id.as_str(),
                            );
                            if selected_web_search_provider_ids.len() > 1 {
                                sub_step = 4;
                                continue;
                            }

                            let credential_options = Self::selected_web_search_credential_options(
                                &options,
                                &selected_web_search_provider_ids,
                            );
                            if credential_options.is_empty() {
                                return Ok(OnboardFlowStepAction::Next);
                            }

                            sub_step = 5;
                        }
                    }
                }
                4 => {
                    let options = Self::web_search_picker_options(&draft.config);
                    let selected_options = Self::selected_web_search_options(
                        &options,
                        &selected_web_search_provider_ids,
                    );
                    let items = selected_options
                        .iter()
                        .map(|option| option.item.clone())
                        .collect::<Vec<_>>();
                    let default_index = selected_options
                        .iter()
                        .position(|option| option.id == default_web_search_provider_id)
                        .unwrap_or(0);
                    match self.run_selection_loop(
                        OnboardWizardStep::Authentication,
                        "Default Web Search",
                        items,
                        default_index,
                        "Enter confirm",
                    )? {
                        SelectionLoopResult::Back => {
                            sub_step = 3;
                        }
                        SelectionLoopResult::Selected(index) => {
                            let selected_option = selected_options.get(index).ok_or_else(|| {
                                "invalid default web search provider selection".to_owned()
                            })?;
                            default_web_search_provider_id = selected_option.id.to_owned();
                            web_search_credential_index = 0;
                            Self::apply_web_search_provider_selection(
                                draft,
                                &options,
                                &selected_web_search_provider_ids,
                                default_web_search_provider_id.as_str(),
                            );
                            let credential_options = Self::selected_web_search_credential_options(
                                &options,
                                &selected_web_search_provider_ids,
                            );
                            if credential_options.is_empty() {
                                return Ok(OnboardFlowStepAction::Next);
                            }

                            sub_step = 5;
                        }
                    }
                }
                5 => {
                    let options = Self::web_search_picker_options(&draft.config);
                    Self::apply_web_search_provider_selection(
                        draft,
                        &options,
                        &selected_web_search_provider_ids,
                        default_web_search_provider_id.as_str(),
                    );
                    let credential_options = Self::selected_web_search_credential_options(
                        &options,
                        &selected_web_search_provider_ids,
                    );
                    let selected_option = credential_options
                        .get(web_search_credential_index)
                        .copied()
                        .ok_or_else(|| "missing web search credential prompt".to_owned())?;
                    let provider_id = selected_option.id;
                    let default_env =
                        crate::onboard_web_search::preferred_web_search_credential_env_default(
                            &draft.config,
                            provider_id,
                        );
                    let label = format!("{} credential env:", selected_option.item.label);
                    match self.run_input_loop(
                        OnboardWizardStep::Authentication,
                        label.as_str(),
                        &default_env,
                        "Enter confirm env name",
                    )? {
                        InputLoopResult::Back => {
                            if web_search_credential_index > 0 {
                                web_search_credential_index =
                                    web_search_credential_index.saturating_sub(1);
                            } else if selected_web_search_provider_ids.len() > 1 {
                                sub_step = 4;
                            } else {
                                sub_step = 3;
                            }
                        }
                        InputLoopResult::Submitted(env_name) => {
                            let trimmed_env = env_name.trim();
                            if trimmed_env.is_empty() {
                                draft.clear_web_search_credential(provider_id);
                            } else {
                                let env_name = trimmed_env.to_owned();
                                draft.set_web_search_credential_env(provider_id, env_name);
                            }

                            web_search_credential_index =
                                web_search_credential_index.saturating_add(1);
                            if web_search_credential_index >= credential_options.len() {
                                return Ok(OnboardFlowStepAction::Next);
                            }
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "internal error: unexpected auth sub-step {sub_step}"
                    ));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Runtime defaults step
    // -----------------------------------------------------------------------

    fn run_runtime_defaults_step(
        &mut self,
        draft: &mut OnboardDraft,
    ) -> CliResult<OnboardFlowStepAction> {
        let mut sub_step: u8 = 0;
        loop {
            match sub_step {
                0 => {
                    let profiles = [
                        mvp::config::MemoryProfile::WindowOnly,
                        mvp::config::MemoryProfile::WindowPlusSummary,
                        mvp::config::MemoryProfile::ProfilePlusWindow,
                    ];
                    let current = draft.config.memory.profile;
                    let default_idx = profiles.iter().position(|p| *p == current).unwrap_or(0);
                    let items = profiles
                        .iter()
                        .map(|profile| {
                            let hint = match profile {
                                mvp::config::MemoryProfile::WindowOnly => "sliding window only",
                                mvp::config::MemoryProfile::WindowPlusSummary => "window + summary",
                                mvp::config::MemoryProfile::ProfilePlusWindow => "profile + window",
                            };
                            SelectionItem::new(profile.as_str(), Some(hint))
                        })
                        .collect::<Vec<_>>();

                    match self.run_selection_loop(
                        OnboardWizardStep::RuntimeDefaults,
                        "Memory Profile",
                        items,
                        default_idx,
                        "Up/Down to select, Enter to confirm",
                    )? {
                        SelectionLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
                        SelectionLoopResult::Selected(index) => {
                            let profile = profiles
                                .get(index)
                                .copied()
                                .ok_or_else(|| "invalid memory profile selection".to_owned())?;
                            draft.set_memory_profile(profile);
                            sub_step = 1;
                        }
                    }
                }
                1 => {
                    let personalities = [
                        mvp::prompt::PromptPersonality::CalmEngineering,
                        mvp::prompt::PromptPersonality::FriendlyCollab,
                        mvp::prompt::PromptPersonality::AutonomousExecutor,
                    ];
                    let items = vec![
                        SelectionItem::new(
                            "Calm engineering",
                            Some("concise, rigorous, and technical by default"),
                        ),
                        SelectionItem::new(
                            "Friendly collab",
                            Some("lighter tone with cooperative guidance"),
                        ),
                        SelectionItem::new(
                            "Autonomous executor",
                            Some("more proactive and action-forward"),
                        ),
                    ];
                    let current_personality = draft.config.cli.resolved_personality();
                    let default_index = personalities
                        .iter()
                        .position(|personality| *personality == current_personality)
                        .unwrap_or(0);

                    match self.run_selection_loop(
                        OnboardWizardStep::RuntimeDefaults,
                        "CLI Personality",
                        items,
                        default_index,
                        "Up/Down to select, Enter to confirm",
                    )? {
                        SelectionLoopResult::Back => {
                            sub_step = 0;
                        }
                        SelectionLoopResult::Selected(index) => {
                            let personality = personalities
                                .get(index)
                                .copied()
                                .ok_or_else(|| "invalid personality selection".to_owned())?;
                            let prompt_addendum = draft.config.cli.system_prompt_addendum.clone();
                            let should_update_prompt = personality != current_personality
                                || !draft.config.cli.uses_native_prompt_pack();
                            if should_update_prompt {
                                draft.use_native_prompt_pack(personality, prompt_addendum);
                            }
                            sub_step = 2;
                        }
                    }
                }
                2 => {
                    let items = vec![
                        SelectionItem::new("CLI shell", Some("interactive local operator surface")),
                        SelectionItem::new(
                            "External skills",
                            Some("downloadable skills with approval required"),
                        ),
                    ];
                    let mut checked_indices = Vec::new();
                    if draft.config.cli.enabled {
                        checked_indices.push(0);
                    }
                    if draft.config.external_skills.enabled {
                        checked_indices.push(1);
                    }

                    match self.run_multi_selection_loop(
                        OnboardWizardStep::RuntimeDefaults,
                        "Runtime Surfaces",
                        items,
                        0,
                        checked_indices,
                        "Space toggle  Enter confirm",
                    )? {
                        MultiSelectionLoopResult::Back => {
                            sub_step = 1;
                        }
                        MultiSelectionLoopResult::Submitted(indices) => {
                            let cli_enabled = indices.contains(&0);
                            let external_skills_enabled = indices.contains(&1);
                            draft.set_cli_enabled(cli_enabled);
                            draft.set_external_skills_runtime_enabled(external_skills_enabled);
                            return Ok(OnboardFlowStepAction::Next);
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "internal error: unexpected runtime defaults sub-step {sub_step}"
                    ));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Workspace step
    // -----------------------------------------------------------------------

    fn run_workspace_step(&mut self, draft: &mut OnboardDraft) -> CliResult<OnboardFlowStepAction> {
        let mut sub_step: u8 = 0;
        loop {
            match sub_step {
                0 => {
                    // Sub-step 1: SQLite path
                    let current_sqlite = draft.workspace.sqlite_path.display().to_string();
                    match self.run_input_loop(
                        OnboardWizardStep::Workspace,
                        "SQLite path:",
                        &current_sqlite,
                        "Enter to confirm, or type a custom path",
                    )? {
                        InputLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
                        InputLoopResult::Submitted(path) => {
                            if !path.is_empty() {
                                draft.set_workspace_sqlite_path(PathBuf::from(path));
                            }
                            sub_step = 1;
                        }
                    }
                }
                1 => {
                    // Sub-step 2: File root
                    let current_file_root = draft.workspace.file_root.display().to_string();
                    match self.run_input_loop(
                        OnboardWizardStep::Workspace,
                        "File root:",
                        &current_file_root,
                        "Enter to confirm, or type a custom path",
                    )? {
                        InputLoopResult::Back => {
                            sub_step = 0;
                        }
                        InputLoopResult::Submitted(path) => {
                            if !path.is_empty() {
                                draft.set_workspace_file_root(PathBuf::from(path));
                            }
                            return Ok(OnboardFlowStepAction::Next);
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "internal error: unexpected workspace sub-step {sub_step}"
                    ));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre/post-flow screens (no spine, full-width content)
    // -----------------------------------------------------------------------

    /// Generic confirmation screen: renders lines of content with a yes/no
    /// key binding.  Returns `true` when the user accepts.
    fn run_confirm_screen(
        &mut self,
        title: &str,
        body_lines: Vec<Line<'static>>,
        footer_hint: &str,
    ) -> CliResult<bool> {
        let mut scroll_offset: u16 = 0;
        let mut captured_visible_height: u16 = body_lines.len() as u16;
        let mut show_help = false;
        loop {
            let lines = body_lines.clone();
            let show_help_now = show_help;
            let current_visible_height = captured_visible_height;
            let compact_lines = Self::confirm_compact_hero_lines(
                title,
                scroll_offset,
                current_visible_height,
                body_lines.len(),
                footer_hint,
                "",
                52,
            );

            self.render_no_spine_shell_with_header(
                title,
                "Esc cancel  ? help",
                &format!("{footer_hint}  j/k scroll  PgUp/PgDn page  ? help"),
                |frame, content_area| {
                    let stage_height = Self::guided_brand_stage_desired_height(
                        content_area.width,
                        &compact_lines,
                        &[],
                    );
                    let shell = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(stage_height), Constraint::Min(10)])
                        .split(content_area);

                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &compact_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        76,
                        0,
                    );

                    let briefing_area = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(2).min(108),
                        shell[1].height,
                    );
                    captured_visible_height = Self::render_scrollable_panel(
                        frame,
                        briefing_area,
                        "Briefing",
                        Self::palette().brand,
                        true,
                        &lines,
                        scroll_offset,
                        Alignment::Left,
                    );

                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Confirm);
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter | KeyCode::Char('y' | 'Y'),
                    ..
                }) => return Ok(true),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('n' | 'N') | KeyCode::Esc,
                    ..
                }) => return Ok(false),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => {
                    Self::scroll_forward(
                        &mut scroll_offset,
                        captured_visible_height,
                        body_lines.len(),
                    );
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => {
                    Self::scroll_backward(&mut scroll_offset);
                }
                Event::Key(KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                }) => {
                    Self::scroll_page_forward(
                        &mut scroll_offset,
                        captured_visible_height,
                        body_lines.len(),
                    );
                }
                Event::Key(KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                }) => {
                    Self::scroll_page_backward(&mut scroll_offset, captured_visible_height);
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Home | KeyCode::Char('g'),
                    ..
                }) => {
                    scroll_offset = 0;
                }
                Event::Key(KeyEvent {
                    code: KeyCode::End | KeyCode::Char('G'),
                    ..
                }) => {
                    Self::scroll_to_end(
                        &mut scroll_offset,
                        captured_visible_height,
                        body_lines.len(),
                    );
                }
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    /// Generic "press Enter to continue" screen with scrollable content.
    #[allow(dead_code)] // kept as a compatibility path while the new launch deck settles
    fn run_info_screen(
        &mut self,
        title: &str,
        body_lines: Vec<Line<'static>>,
        footer_hint: &str,
    ) -> CliResult<()> {
        let palette = Self::palette();
        let mut scroll_offset: u16 = 0;
        let total_lines = body_lines.len();
        let mut captured_visible_height: u16 = total_lines as u16;
        let mut show_help = false;

        loop {
            let lines = body_lines.clone();
            let show_help_now = show_help;

            self.render_no_spine_shell_with_header(
                title,
                "Esc cancel  ? help",
                &format!("{footer_hint}  PgUp/PgDn page  g/G edge  ? help"),
                |frame, content_area| {
                    let stage_lines = Self::info_stage_lines(
                        title,
                        scroll_offset,
                        captured_visible_height,
                        total_lines,
                    );
                    let stage_height = Self::guided_brand_stage_desired_height(
                        content_area.width,
                        &stage_lines,
                        &[],
                    );
                    let shell = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(stage_height), Constraint::Min(10)])
                        .split(content_area);
                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &stage_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        76,
                        0,
                    );

                    let panel_area = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(2).min(96),
                        shell[1].height,
                    );
                    captured_visible_height = Self::render_scrollable_panel(
                        frame,
                        panel_area,
                        "Details",
                        palette.brand,
                        true,
                        &lines,
                        scroll_offset,
                        Alignment::Left,
                    );

                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Info);
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(()),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(()),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => {
                    Self::scroll_forward(
                        &mut scroll_offset,
                        captured_visible_height,
                        body_lines.len(),
                    );
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => {
                    scroll_offset = scroll_offset.saturating_sub(1);
                }
                Event::Key(KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                }) => {
                    Self::scroll_page_forward(
                        &mut scroll_offset,
                        captured_visible_height,
                        total_lines,
                    );
                }
                Event::Key(KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                }) => {
                    Self::scroll_page_backward(&mut scroll_offset, captured_visible_height);
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Home | KeyCode::Char('g'),
                    ..
                }) => {
                    scroll_offset = 0;
                }
                Event::Key(KeyEvent {
                    code: KeyCode::End | KeyCode::Char('G'),
                    ..
                }) => {
                    Self::scroll_to_end(&mut scroll_offset, captured_visible_height, total_lines);
                }
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre-flow: risk acknowledgement screen
    // -----------------------------------------------------------------------

    #[cfg(test)]
    pub fn run_risk_screen(&mut self) -> CliResult<bool> {
        let version = mvp::presentation::BuildVersionInfo::current().render_version_line();
        let risk_lines = Self::risk_gate_lines();
        let mut show_help = false;

        loop {
            let ver = version.clone();
            let show_help_now = show_help;

            self.render_no_spine_shell_with_header(
                "security check",
                "Esc cancel  ? help",
                "Enter accept  y continue  n cancel  ? help",
                |frame, content_area| {
                    Self::render_risk_centerpiece(frame, content_area, &ver, &risk_lines);

                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Confirm);
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter | KeyCode::Char('y' | 'Y'),
                    ..
                }) => return Ok(true),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('n' | 'N') | KeyCode::Esc,
                    ..
                }) => return Ok(false),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre-flow: entry choice (current / detected / start fresh)
    // -----------------------------------------------------------------------

    pub fn run_entry_choice_screen(
        &mut self,
        options: &[(String, String)],
        default_index: usize,
        summary_lines: &[String],
    ) -> CliResult<usize> {
        let items: Vec<SelectionItem> = options
            .iter()
            .map(|(label, detail)| SelectionItem::new(label.as_str(), Some(detail.as_str())))
            .collect();

        match self.run_showcase_selection_loop(
            ShowcaseStageVariant::EntryPath,
            "choose your path",
            "Choose A Path",
            &HERO_INTRO_LINES,
            "Starting Point Snapshot",
            summary_lines,
            items,
            default_index,
            "Up/Down move  1..9 jump  Enter confirm  Tab/h/l detail",
        )? {
            StandaloneSelectionResult::Selected(idx) => Ok(idx),
            StandaloneSelectionResult::Cancel => {
                Err("onboarding cancelled: entry choice declined".to_owned())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre-flow: import candidate selection
    // -----------------------------------------------------------------------

    pub fn run_import_candidate_screen(
        &mut self,
        candidates: &[(String, String)],
        default_index: usize,
        summary_lines: &[String],
    ) -> CliResult<Option<usize>> {
        let mut items: Vec<SelectionItem> = candidates
            .iter()
            .map(|(label, detail)| SelectionItem::new(label.as_str(), Some(detail.as_str())))
            .collect();
        items.push(SelectionItem::new(
            "Start fresh",
            Some("begin with default config"),
        ));

        match self.run_showcase_selection_loop(
            ShowcaseStageVariant::DetectedStartingPoint,
            "choose a detected starting point",
            "Detected Starting Points",
            &[
                "We found reusable setup signals on this machine.",
                "Choose one to carry forward into onboarding.",
            ],
            "Signal Snapshot",
            summary_lines,
            items,
            default_index,
            "Up/Down move  1..9 jump  Enter confirm  Tab/h/l detail",
        )? {
            StandaloneSelectionResult::Selected(idx) if idx < candidates.len() => Ok(Some(idx)),
            StandaloneSelectionResult::Selected(_) => Ok(None),
            StandaloneSelectionResult::Cancel => {
                Err("onboarding cancelled: import selection cancelled".to_owned())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre-flow: shortcut choice (use detected/current vs full setup)
    // -----------------------------------------------------------------------

    pub fn run_shortcut_choice_screen(
        &mut self,
        primary_label: &str,
        snapshot_lines: &[String],
    ) -> CliResult<bool> {
        let items = vec![
            SelectionItem::new(primary_label, Some("skip detailed edits")),
            SelectionItem::new("Adjust settings", Some("go through full setup")),
        ];

        match self.run_showcase_selection_loop(
            ShowcaseStageVariant::ShortcutChoice,
            "choose how to continue",
            "Next Action",
            &[
                "LoongClaw can keep this draft as-is or walk through every setting.",
                "Use the quick path when the snapshot already looks right.",
            ],
            "Current Draft",
            snapshot_lines,
            items,
            0,
            "Up/Down move  1..9 jump  Enter confirm  Tab/h/l detail",
        )? {
            StandaloneSelectionResult::Selected(0) => Ok(true),
            StandaloneSelectionResult::Selected(_) => Ok(false),
            StandaloneSelectionResult::Cancel => {
                Err("onboarding cancelled: shortcut choice cancelled".to_owned())
            }
        }
    }

    fn run_showcase_selection_loop(
        &mut self,
        variant: ShowcaseStageVariant,
        title: &str,
        choices_title: &str,
        intro_lines: &[&str],
        _panel_title: &str,
        panel_lines: &[String],
        items: Vec<SelectionItem>,
        default_index: usize,
        footer_hint: &str,
    ) -> CliResult<StandaloneSelectionResult> {
        if items.is_empty() {
            return Err("no items to select from".to_owned());
        }

        let mut state = SelectionCardState::new(items.len());
        state.select(default_index);
        let mut show_help = false;

        loop {
            let captured_logo_area = self.render_showcase_shell(
                variant,
                title,
                intro_lines,
                choices_title,
                panel_lines,
                &items,
                &mut state,
                footer_hint,
                show_help,
            )?;
            if let Some(logo_area) = captured_logo_area {
                self.paint_inline_logo(logo_area)?;
            }

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(StandaloneSelectionResult::Selected(state.selected())),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => state.next(),
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => state.previous(),
                Event::Key(KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                }) => {
                    for _ in 0..3 {
                        state.next();
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                }) => {
                    for _ in 0..3 {
                        state.previous();
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Home | KeyCode::Char('g'),
                    ..
                }) => state.select_first(),
                Event::Key(KeyEvent {
                    code: KeyCode::End | KeyCode::Char('G'),
                    ..
                }) => state.select_last(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    ..
                }) if ('1'..='9').contains(&c) => {
                    let idx = usize::from(c as u8 - b'1');
                    if idx < items.len() {
                        state.select(idx);
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(StandaloneSelectionResult::Cancel),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => {}
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Post-flow: preflight check results screen
    // -----------------------------------------------------------------------

    #[allow(dead_code)] // retained for incremental migration and focused tests
    pub fn run_preflight_screen(
        &mut self,
        checks: &[crate::onboard_preflight::OnboardCheck],
    ) -> CliResult<bool> {
        let palette = Self::palette();
        let (pass_count, warning_count, failure_count) = Self::check_level_counts(checks);
        let mut body_lines: Vec<Line<'static>> = Vec::new();
        body_lines.push(Line::from(""));
        body_lines.push(Line::from(Span::styled(
            "Preflight checks:",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        )));
        body_lines.push(Line::from(""));
        body_lines.extend(Self::preflight_summary_lines(
            pass_count,
            warning_count,
            failure_count,
        ));
        body_lines.push(Line::from(""));

        for check in checks {
            let (icon, color) = match check.level {
                crate::onboard_preflight::OnboardCheckLevel::Pass => ("\u{2713}", palette.success),
                crate::onboard_preflight::OnboardCheckLevel::Warn => ("\u{26a0}", palette.warning),
                crate::onboard_preflight::OnboardCheckLevel::Fail => ("\u{2717}", palette.error),
            };
            body_lines.push(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(
                    check.name.to_owned(),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", check.detail),
                    Style::default().fg(palette.secondary_text),
                ),
            ]));
        }
        body_lines.push(Line::from(""));

        if failure_count > 0 {
            self.run_info_screen("preflight blocked", body_lines, "Enter to return")?;
            Ok(false)
        } else if warning_count > 0 {
            self.run_confirm_screen(
                "preflight results",
                body_lines,
                "Enter/y continue with warnings  n/Esc cancel",
            )
        } else {
            // All green — just show the results and continue.
            self.run_info_screen("preflight results", body_lines, "Enter to continue")?;
            Ok(true)
        }
    }

    // -----------------------------------------------------------------------
    // Post-flow: review screen (scrollable config summary)
    // -----------------------------------------------------------------------

    #[allow(dead_code)] // retained for incremental migration and focused tests
    pub fn run_review_screen(&mut self, review_lines: &[String]) -> CliResult<()> {
        let palette = Self::palette();
        let body_lines: Vec<Line<'static>> = review_lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    line.to_owned(),
                    Style::default().fg(palette.secondary_text),
                ))
            })
            .collect();

        self.run_info_screen(
            "review config",
            body_lines,
            "Up/Down scroll  Enter continue",
        )
    }

    // -----------------------------------------------------------------------
    // Post-flow: write confirmation screen
    // -----------------------------------------------------------------------

    #[allow(dead_code)] // retained for incremental migration and focused tests
    pub fn run_write_confirmation_screen(
        &mut self,
        config_path: &str,
        warnings_kept: bool,
    ) -> CliResult<bool> {
        let palette = Self::palette();
        let status = if warnings_kept {
            "warnings were kept by choice"
        } else {
            "all checks green"
        };
        let body_lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("Config path: {config_path}"),
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                format!("Status: {status}"),
                Style::default().fg(if warnings_kept {
                    palette.warning
                } else {
                    palette.success
                }),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Write this configuration?",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        self.run_confirm_screen("write config", body_lines, "Enter/y write  n/Esc cancel")
    }

    // -----------------------------------------------------------------------
    // Post-flow: combined verify + write screen
    // -----------------------------------------------------------------------

    pub fn run_verify_and_write_screen(
        &mut self,
        checks: &[crate::onboard_preflight::OnboardCheck],
        review_lines: &[String],
        config_path: &str,
    ) -> CliResult<bool> {
        let palette = Self::palette();
        let (pass_count, warning_count, failure_count) = Self::check_level_counts(checks);
        let accent_color = if failure_count > 0 {
            palette.error
        } else if warning_count > 0 {
            palette.warning
        } else {
            palette.brand
        };
        let mut review_panel_lines = vec![
            Line::from(vec![
                Span::styled(
                    "Verification",
                    Style::default()
                        .fg(accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {pass_count} pass  {warning_count} warn  {failure_count} fail"),
                    Style::default().fg(palette.muted_text),
                ),
            ]),
            Line::from(""),
        ];
        for check in checks {
            let (icon, color) = match check.level {
                crate::onboard_preflight::OnboardCheckLevel::Pass => ("\u{2713}", palette.success),
                crate::onboard_preflight::OnboardCheckLevel::Warn => ("\u{26a0}", palette.warning),
                crate::onboard_preflight::OnboardCheckLevel::Fail => ("\u{2717}", palette.error),
            };
            review_panel_lines.push(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(
                    check.name.to_owned(),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            review_panel_lines.push(Line::from(Span::styled(
                format!("  {}", check.detail),
                Style::default().fg(palette.secondary_text),
            )));
            review_panel_lines.push(Line::from(""));
        }
        if warning_count > 0 {
            review_panel_lines.push(Line::from(Span::styled(
                "Warnings remain in this draft. Writing will keep them as-is.",
                Style::default().fg(palette.warning),
            )));
        } else if failure_count > 0 {
            review_panel_lines.push(Line::from(Span::styled(
                "Blocking verification failures remain. Writing is disabled until these checks are cleared.",
                Style::default().fg(palette.error),
            )));
        } else if failure_count == 0 {
            review_panel_lines.push(Line::from(Span::styled(
                "All verification gates are green. This draft is ready to write.",
                Style::default().fg(palette.success),
            )));
        }
        review_panel_lines.push(Line::from(""));
        review_panel_lines.push(Line::from(Span::styled(
            "Draft review",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        )));
        review_panel_lines.push(Line::from(Span::styled(
            format!("Config target: {config_path}"),
            Style::default().fg(palette.text),
        )));
        review_panel_lines.push(Line::from(""));
        review_panel_lines.extend(review_lines.iter().map(|line| {
            Line::from(Span::styled(
                line.to_owned(),
                Style::default().fg(palette.secondary_text),
            ))
        }));

        let hero_copy = if failure_count > 0 {
            "Verification found blockers. Review the failing checks before attempting to write this draft."
        } else if warning_count > 0 {
            "Warnings remain. Review the draft, then decide whether to write with those warnings kept."
        } else {
            "Verification is clear. Do one final review, then write the draft when everything reads cleanly."
        };
        let hero_status = if failure_count > 0 {
            "blocked by verification"
        } else if warning_count > 0 {
            "ready with warnings"
        } else {
            "ready to write"
        };

        let mut review_scroll = 0u16;
        let mut show_help = false;
        loop {
            let mut captured_review_height = 0u16;
            let review_panel_lines_now = review_panel_lines.clone();
            let current_review_scroll = review_scroll;
            let show_help_now = show_help;
            let hero_lines = if checks.len() > 6 {
                Self::verify_gate_narrow_hero_lines(
                    hero_status,
                    pass_count,
                    warning_count,
                    failure_count,
                    config_path,
                    84,
                )
            } else {
                Self::verify_gate_compact_hero_lines(
                    hero_status,
                    hero_copy,
                    pass_count,
                    warning_count,
                    failure_count,
                    config_path,
                    84,
                )
            };

            self.render_no_spine_shell_with_header(
                "verify and write",
                "n / Esc cancel  ? help",
                if failure_count > 0 {
                    "j/k scroll  PgUp/PgDn page  g/G edge  Enter return  n/Esc cancel  ? help"
                } else {
                    "j/k scroll  PgUp/PgDn page  g/G edge  Enter write  n/Esc cancel  ? help"
                },
                |frame, content_area| {
                    let stage_height = Self::guided_brand_stage_desired_height(
                        content_area.width,
                        &hero_lines,
                        &[],
                    );
                    let max_stage_height = content_area.height.saturating_sub(12).max(1);
                    let stage_height = stage_height.min(max_stage_height);
                    let shell = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(stage_height), Constraint::Min(12)])
                        .split(content_area);

                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &hero_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        84,
                        0,
                    );

                    let review_area = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(2).min(96),
                        shell[1].height,
                    );
                    captured_review_height = Self::render_scrollable_panel(
                        frame,
                        review_area,
                        "Review",
                        accent_color,
                        true,
                        &review_panel_lines_now,
                        current_review_scroll,
                        Alignment::Left,
                    );

                    if show_help_now {
                        Self::render_help_overlay(
                            frame,
                            content_area,
                            HelpOverlayTopic::VerifyWrite,
                        );
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter | KeyCode::Char('y'),
                    ..
                }) => return Ok(failure_count == 0),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc | KeyCode::Char('n'),
                    ..
                }) => return Ok(false),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => Self::scroll_forward(
                    &mut review_scroll,
                    captured_review_height,
                    review_panel_lines.len(),
                ),
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => Self::scroll_backward(&mut review_scroll),
                Event::Key(KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                }) => Self::scroll_page_forward(
                    &mut review_scroll,
                    captured_review_height,
                    review_panel_lines.len(),
                ),
                Event::Key(KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                }) => Self::scroll_page_backward(&mut review_scroll, captured_review_height),
                Event::Key(KeyEvent {
                    code: KeyCode::Home | KeyCode::Char('g'),
                    ..
                }) => review_scroll = 0,
                Event::Key(KeyEvent {
                    code: KeyCode::End | KeyCode::Char('G'),
                    ..
                }) => Self::scroll_to_end(
                    &mut review_scroll,
                    captured_review_height,
                    review_panel_lines.len(),
                ),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => {}
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Post-flow: launch deck
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    fn launch_action_items(summary: &OnboardingSuccessSummary) -> Vec<SelectionItem> {
        if summary.next_actions.is_empty() {
            vec![SelectionItem::new(
                "No follow-up command",
                Some("review the saved setup on the right"),
            )]
        } else {
            summary
                .next_actions
                .iter()
                .map(|action| {
                    SelectionItem::new(
                        action.label.as_str(),
                        Some(Self::launch_action_kind_label(action.kind)),
                    )
                })
                .collect()
        }
    }

    #[allow(dead_code)]
    fn launch_status_lines(
        summary: &OnboardingSuccessSummary,
        hero_copy: &str,
        _focused_action: Option<usize>,
        accent_color: Color,
        _width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let config_label = Path::new(summary.config_path.as_str())
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(summary.config_path.as_str());
        let configured_surface_line = if summary.channels.is_empty() {
            "Local chat is ready. Channels can be added later from the same config.".to_owned()
        } else {
            let channel_count = summary.channels.len();
            let surface_suffix = if channel_count == 1 { "" } else { "s" };
            format!("{channel_count} channel surface{surface_suffix} ready beside local chat.")
        };

        vec![
            Line::from(Span::styled(
                summary.outcome.ready_label(),
                Style::default()
                    .fg(accent_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                hero_copy.to_owned(),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "{} · {} · {}",
                    summary.provider, summary.model, config_label
                ),
                Style::default().fg(palette.text),
            )),
            Line::from(Span::styled(
                configured_surface_line,
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    fn launch_handoff_lines(
        action: Option<&OnboardingAction>,
        summary: &OnboardingSuccessSummary,
        _focus_actions: bool,
        _width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let chat_command = action
            .filter(|action| action.kind == OnboardingActionKind::Chat)
            .map(|action| action.command.clone())
            .or_else(|| {
                summary
                    .next_actions
                    .iter()
                    .find(|next_action| next_action.kind == OnboardingActionKind::Chat)
                    .map(|next_action| next_action.command.clone())
            })
            .unwrap_or_else(|| format!("loong chat --config '{}'", summary.config_path));
        let runtime_note = if summary.channels.len() > 1 {
            "Configured channels keep their own session boundaries while chat is open."
        } else {
            "You can keep chatting here and add more channels later without rerunning the whole flow."
        };

        vec![
            Line::from(Span::styled(
                "Ready handoff",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                chat_command,
                Style::default()
                    .fg(palette.info)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Enter opens chat in this terminal. Esc finishes without opening chat.",
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                runtime_note,
                Style::default().fg(palette.muted_text),
            )),
        ]
    }

    #[allow(dead_code)]
    fn launch_action_kind_label(kind: OnboardingActionKind) -> &'static str {
        match kind {
            OnboardingActionKind::Ask => "CLI sanity check",
            OnboardingActionKind::Chat => "interactive operator session",
            OnboardingActionKind::Channel => "service channel bring-up",
            OnboardingActionKind::BrowserPreview => "browser companion path",
            OnboardingActionKind::Doctor => "diagnostics and recovery",
        }
    }

    #[allow(dead_code)]
    fn launch_action_badge_label(kind: OnboardingActionKind) -> &'static str {
        match kind {
            OnboardingActionKind::Ask => "ASK",
            OnboardingActionKind::Chat => "CHAT",
            OnboardingActionKind::Channel => "CHANNEL",
            OnboardingActionKind::BrowserPreview => "BROWSER",
            OnboardingActionKind::Doctor => "DOCTOR",
        }
    }

    #[allow(dead_code)]
    fn launch_action_color(_kind: OnboardingActionKind) -> Color {
        Self::palette().brand
    }

    #[allow(dead_code)]
    fn launch_outcome_badge(outcome: OnboardOutcome) -> Span<'static> {
        let palette = Self::palette();
        match outcome {
            OnboardOutcome::Success => Self::filled_badge_span("READY", palette.success),
            OnboardOutcome::SuccessWithWarnings => {
                Self::filled_badge_span("WARNINGS", palette.warning)
            }
            OnboardOutcome::Blocked => Self::filled_badge_span("BLOCKED", palette.error),
        }
    }

    #[allow(dead_code)]
    fn launch_action_detail_lines(
        action: Option<&OnboardingAction>,
        summary: &OnboardingSuccessSummary,
        selected_index: Option<usize>,
        width: u16,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let Some(action) = action else {
            let mut lines = vec![Line::from(Span::styled(
                "Action briefing",
                Style::default().fg(palette.secondary_text),
            ))];
            lines.extend(Self::badge_lines(
                [
                    Self::filled_badge_span("NO ACTION", palette.warning),
                    Self::launch_outcome_badge(summary.outcome),
                    Self::filled_badge_span("REVIEW SETUP", palette.brand),
                ],
                width,
            ));
            lines.extend([
                Line::from(""),
                Line::from(Span::styled(
                    "No action was generated for this draft.",
                    Style::default().fg(palette.secondary_text),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Arm an action on the left to turn this panel into the first-move briefing.",
                    Style::default().fg(palette.secondary_text),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Inspect the saved setup and use `loong doctor` if you need a runtime check.",
                    Style::default().fg(palette.secondary_text),
                )),
            ]);
            return lines;
        };

        let action_position = selected_index
            .map(|index| format!("Action {} of {}", index + 1, summary.next_actions.len()))
            .unwrap_or_else(|| "Action briefing".to_owned());
        let action_color = Self::launch_action_color(action.kind);

        let rationale = match action.kind {
            OnboardingActionKind::Ask => {
                "This is the fastest end-to-end proof that the provider, model, and CLI handoff all work."
            }
            OnboardingActionKind::Chat => {
                "Use chat when you want a longer interactive session instead of a single request."
            }
            OnboardingActionKind::Channel => {
                "This brings a runtime-backed service surface online after the local deck is ready."
            }
            OnboardingActionKind::BrowserPreview => {
                "This checks whether the browser companion path is ready, blocked, or still needs install work."
            }
            OnboardingActionKind::Doctor => {
                "Use doctor when there is no obvious first launch path or when verification needs another pass."
            }
        };
        let validation_note = match action.kind {
            OnboardingActionKind::Ask => {
                "Validates local launch, provider routing, and response formatting."
            }
            OnboardingActionKind::Chat => {
                "Validates long-running interactive session state and prompt behavior."
            }
            OnboardingActionKind::Channel => {
                "Validates downstream surface wiring and channel-specific runtime requirements."
            }
            OnboardingActionKind::BrowserPreview => {
                "Validates browser automation bridge availability and shell readiness."
            }
            OnboardingActionKind::Doctor => {
                "Validates the environment and points to any remaining blockers."
            }
        };

        let mut lines = vec![Line::from(Span::styled(
            action_position,
            Style::default().fg(palette.secondary_text),
        ))];
        lines.extend(Self::badge_lines(
            [
                Self::filled_badge_span(Self::launch_action_badge_label(action.kind), action_color),
                Self::filled_badge_span("LIVE COMMAND", palette.info),
                Self::launch_outcome_badge(summary.outcome),
            ],
            width,
        ));
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                Self::launch_action_kind_label(action.kind),
                Style::default()
                    .fg(action_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                action.label.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                action.command.clone(),
                Style::default()
                    .fg(palette.info)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Runs after you leave the launch deck.",
                Style::default().fg(palette.muted_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                rationale,
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(Span::styled(
                format!("Validates: {validation_note}"),
                Style::default().fg(palette.secondary_text),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("Current outcome: {}", summary.outcome.ready_label()),
                Style::default().fg(match summary.outcome {
                    OnboardOutcome::Success => palette.success,
                    OnboardOutcome::SuccessWithWarnings => palette.warning,
                    OnboardOutcome::Blocked => palette.error,
                }),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "The focused action is echoed after you leave the launch deck.",
                Style::default().fg(palette.muted_text),
            )),
        ]);
        if summary.outcome == OnboardOutcome::Blocked {
            lines.push(Line::from(Span::styled(
                "Launch is still blocked somewhere in verification. Prefer diagnostics before wider rollout.",
                Style::default().fg(palette.warning),
            )));
        }
        lines
    }

    fn saved_setup_lines(
        summary: &OnboardingSuccessSummary,
        accent_color: Color,
    ) -> Vec<Line<'static>> {
        let palette = Self::palette();
        let config_label = Path::new(summary.config_path.as_str())
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(summary.config_path.as_str());
        let provider_summary = format!("{} · {}", summary.provider, summary.model);
        let mut provider_profiles = summary.saved_provider_profiles.clone();
        provider_profiles.sort();
        let channel_summary = if summary.channels.is_empty() {
            "none yet".to_owned()
        } else {
            let mut channels = summary.channels.clone();
            channels.sort();
            channels.join(", ")
        };
        let mut lines = vec![
            Line::from(Span::styled(
                "Configured now",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("{} · {}", summary.outcome.ready_label(), config_label),
                Style::default()
                    .fg(accent_color)
                    .add_modifier(Modifier::BOLD),
            )),
        ];
        if let Some(verification_status) = summary.verification_status.as_deref() {
            lines.push(Line::from(Span::styled(
                format!("Verification · {verification_status}"),
                Style::default().fg(palette.secondary_text),
            )));
        }
        if let Some(import_source) = summary.import_source.as_deref() {
            lines.push(Line::from(Span::styled(
                format!("Starting point · {import_source}"),
                Style::default().fg(palette.muted_text),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Provider stack",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            provider_summary,
            Style::default().fg(palette.text),
        )));
        if provider_profiles.len() > 1 {
            lines.push(Line::from(Span::styled(
                format!("Profiles · {}", provider_profiles.join(", ")),
                Style::default().fg(palette.secondary_text),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!("Web search · {}", summary.web_search_provider),
            Style::default().fg(palette.secondary_text),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Surfaces",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!("Channels · {channel_summary}"),
            Style::default().fg(palette.secondary_text),
        )));
        if let Some(memory_path) = summary.memory_path.as_deref() {
            lines.push(Line::from(Span::styled(
                format!("Memory · {memory_path}"),
                Style::default().fg(palette.secondary_text),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!("Recall profile · {}", summary.memory_profile),
            Style::default().fg(palette.muted_text),
        )));
        lines
    }

    pub fn run_launch_screen(
        &mut self,
        summary: &OnboardingSuccessSummary,
    ) -> CliResult<LaunchDeckResult> {
        let palette = Self::palette();
        let accent_color = match summary.outcome {
            OnboardOutcome::Success => palette.success,
            OnboardOutcome::SuccessWithWarnings => palette.warning,
            OnboardOutcome::Blocked => palette.error,
        };
        let setup_lines = Self::saved_setup_lines(summary, accent_color);
        let hero_copy = if summary.channels.len() > 1 {
            "Config is written. Chat can open now while configured channels keep their own runtime surfaces."
        } else {
            "Config is written. Chat can open now, and more channels can be added later from the same setup."
        };
        let mut setup_scroll = 0u16;
        let mut show_help = false;

        loop {
            let mut captured_setup_height = 0u16;
            let setup_panel_lines = setup_lines.clone();
            let current_setup_scroll = setup_scroll;
            let show_help_now = show_help;
            let handoff_lines = Self::launch_handoff_lines(None, summary, false, 96);

            self.render_no_spine_shell_with_header(
                "launch",
                "Enter open chat  Esc finish  ? help",
                "j/k scroll  PgUp/PgDn page  g/G edge  Enter open chat  Esc finish  ? help",
                |frame, content_area| {
                    let status_lines =
                        Self::launch_status_lines(summary, hero_copy, None, accent_color, 82);
                    let handoff_height = 4_u16;
                    let stage_height = Self::guided_brand_stage_desired_height(
                        content_area.width,
                        &status_lines,
                        &[],
                    );
                    let reserved_height = handoff_height.saturating_add(10);
                    let max_stage_height =
                        content_area.height.saturating_sub(reserved_height).max(1);
                    let stage_height = stage_height.min(max_stage_height);
                    let shell = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(stage_height),
                            Constraint::Min(10),
                            Constraint::Length(handoff_height),
                        ])
                        .split(content_area);

                    Self::render_guided_brand_stage(
                        frame,
                        shell[0],
                        &status_lines,
                        &[],
                        Alignment::Center,
                        Alignment::Center,
                        82,
                        0,
                    );

                    let main_area = Self::centered_rect(
                        shell[1],
                        shell[1].width.saturating_sub(2).min(96),
                        shell[1].height,
                    );
                    captured_setup_height = Self::render_scrollable_panel(
                        frame,
                        main_area,
                        "Saved Setup",
                        accent_color,
                        true,
                        &setup_panel_lines,
                        current_setup_scroll,
                        Alignment::Left,
                    );
                    let handoff_area = Self::centered_rect(
                        shell[2],
                        shell[2].width.saturating_sub(4).min(96),
                        shell[2].height,
                    );
                    frame.render_widget(
                        Paragraph::new(handoff_lines.clone())
                            .alignment(Alignment::Center)
                            .wrap(Wrap { trim: false }),
                        handoff_area,
                    );

                    if show_help_now {
                        Self::render_help_overlay(frame, content_area, HelpOverlayTopic::Launch);
                    }
                },
            )?;

            let event = self.event_source.next_event().map_err(|e| e.to_string())?;
            if Self::handle_help_overlay_event(&mut show_help, &event)? {
                continue;
            }

            match event {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    let open_chat = summary.outcome != OnboardOutcome::Blocked;
                    return Ok(LaunchDeckResult {
                        focused_action: None,
                        open_chat,
                    });
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => {
                    return Ok(LaunchDeckResult {
                        focused_action: None,
                        open_chat: false,
                    });
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => Self::scroll_forward(
                    &mut setup_scroll,
                    captured_setup_height,
                    setup_lines.len(),
                ),
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => Self::scroll_backward(&mut setup_scroll),
                Event::Key(KeyEvent {
                    code: KeyCode::PageDown,
                    ..
                }) => Self::scroll_page_forward(
                    &mut setup_scroll,
                    captured_setup_height,
                    setup_lines.len(),
                ),
                Event::Key(KeyEvent {
                    code: KeyCode::PageUp,
                    ..
                }) => Self::scroll_page_backward(&mut setup_scroll, captured_setup_height),
                Event::Key(KeyEvent {
                    code: KeyCode::Home | KeyCode::Char('g'),
                    ..
                }) => setup_scroll = 0,
                Event::Key(KeyEvent {
                    code: KeyCode::End | KeyCode::Char('G'),
                    ..
                }) => {
                    Self::scroll_to_end(&mut setup_scroll, captured_setup_height, setup_lines.len())
                }
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => {}
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Post-flow: success summary screen
    // -----------------------------------------------------------------------

    #[allow(dead_code)] // retained for incremental migration and focused tests
    pub fn run_success_screen(&mut self, summary_lines: &[String]) -> CliResult<()> {
        let palette = Self::palette();
        let mut body_lines: Vec<Line<'static>> = Vec::new();
        body_lines.push(Line::from(""));
        body_lines.push(Line::from(vec![
            Span::styled("\u{2713}  ", Style::default().fg(palette.success)),
            Span::styled(
                "Setup complete!",
                Style::default()
                    .fg(palette.success)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        body_lines.push(Line::from(""));
        for line in summary_lines {
            body_lines.push(Line::from(Span::styled(
                line.to_owned(),
                Style::default().fg(palette.secondary_text),
            )));
        }
        body_lines.push(Line::from(""));

        self.run_info_screen("setup complete", body_lines, "Enter to exit")
    }

    // -----------------------------------------------------------------------
    // Standalone selection loop (no spine, for pre/post flow)
    // -----------------------------------------------------------------------

    #[allow(dead_code)] // kept until all standalone selection flows use showcase layout
    fn run_standalone_selection_loop(
        &mut self,
        title: &str,
        items: Vec<SelectionItem>,
        default_index: usize,
        footer_hint: &str,
    ) -> CliResult<StandaloneSelectionResult> {
        self.run_showcase_selection_loop(
            ShowcaseStageVariant::Generic,
            title,
            "Selection Deck",
            &[
                "Make one explicit choice before leaving this gate.",
                "This compatibility path now uses the full-screen onboarding shell.",
            ],
            "Selection Notes",
            &[],
            items,
            default_index,
            footer_hint,
        )
    }

    // -----------------------------------------------------------------------
    // Protocols step
    // -----------------------------------------------------------------------

    fn run_protocols_step(&mut self, draft: &mut OnboardDraft) -> CliResult<OnboardFlowStepAction> {
        let mut sub_step: u8 = 0;
        loop {
            match sub_step {
                0 => {
                    let entries = Self::sorted_service_channel_catalog_entries(&draft.config);
                    if entries.is_empty() {
                        draft.set_enabled_service_channels(Vec::<String>::new());
                        sub_step = 1;
                        continue;
                    }
                    let items = entries
                        .iter()
                        .map(|entry| {
                            let operation_label = Self::channel_primary_operation(entry)
                                .map(|operation| operation.label.to_owned())
                                .unwrap_or_else(|| "manual config".to_owned());
                            SelectionItem::new(entry.label, Some(operation_label))
                        })
                        .collect::<Vec<_>>();
                    let enabled_ids = draft.config.enabled_service_channel_ids();
                    let checked_indices = entries
                        .iter()
                        .enumerate()
                        .filter_map(|(index, entry)| {
                            let is_enabled = enabled_ids.iter().any(|id| id == entry.id);
                            if is_enabled {
                                return Some(index);
                            }
                            None
                        })
                        .collect::<Vec<_>>();
                    match self.run_multi_selection_loop(
                        OnboardWizardStep::Protocols,
                        "Service Channels",
                        items,
                        0,
                        checked_indices,
                        "Space toggle  Enter confirm",
                    )? {
                        MultiSelectionLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
                        MultiSelectionLoopResult::Submitted(indices) => {
                            let selected_ids = indices
                                .into_iter()
                                .filter_map(|index| entries.get(index))
                                .map(|entry| entry.id.to_owned())
                                .collect::<Vec<_>>();
                            draft.set_enabled_service_channels(selected_ids);
                            let enabled_ids = draft.config.enabled_service_channel_ids();
                            let selected_entries = entries
                                .iter()
                                .filter(|entry| enabled_ids.iter().any(|id| id == entry.id))
                                .cloned()
                                .collect::<Vec<_>>();
                            let pairing_completed = self
                                .run_selected_channel_pairing_sequence(draft, &selected_entries)?;
                            if !pairing_completed {
                                continue;
                            }
                            sub_step = 1;
                        }
                    }
                }
                1 => {
                    let current_enabled = draft.protocols.acp_enabled;
                    let default_idx = if current_enabled { 0 } else { 1 };
                    let items = vec![
                        SelectionItem::new("Enabled", Some("connect to ACP agents")),
                        SelectionItem::new("Disabled", Some("standalone mode")),
                    ];

                    match self.run_selection_loop(
                        OnboardWizardStep::Protocols,
                        "ACP Protocol",
                        items,
                        default_idx,
                        "Up/Down to select, Enter to confirm",
                    )? {
                        SelectionLoopResult::Back => {
                            sub_step = 0;
                        }
                        SelectionLoopResult::Selected(idx) => {
                            let enabled = idx == 0;
                            draft.set_acp_enabled(enabled);
                            if !enabled {
                                return Ok(OnboardFlowStepAction::Next);
                            }
                            sub_step = 2;
                        }
                    }
                }
                2 => {
                    let backend_options = Self::onboard_acp_backend_options()?;
                    let default_backend_idx =
                        Self::default_onboard_acp_backend_index(draft, &backend_options);
                    let items = backend_options
                        .iter()
                        .map(|backend| {
                            SelectionItem::new(backend.id.as_str(), Some(backend.summary.as_str()))
                        })
                        .collect::<Vec<_>>();

                    match self.run_selection_loop(
                        OnboardWizardStep::Protocols,
                        "ACP Backend",
                        items,
                        default_backend_idx,
                        "Up/Down to select, Enter to confirm",
                    )? {
                        SelectionLoopResult::Back => {
                            sub_step = 1;
                        }
                        SelectionLoopResult::Selected(idx) => {
                            let selected_backend = backend_options.get(idx);
                            if let Some(selected_backend) = selected_backend {
                                draft.set_acp_backend(Some(selected_backend.id.clone()));
                            }
                            return Ok(OnboardFlowStepAction::Next);
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "internal error: unexpected protocols sub-step {sub_step}"
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GuidedOnboardFlowStepRunner implementation
// ---------------------------------------------------------------------------

impl<E: OnboardEventSource> GuidedOnboardFlowStepRunner for RatatuiOnboardRunner<E> {
    async fn run_step(
        &mut self,
        step: OnboardWizardStep,
        draft: &mut OnboardDraft,
    ) -> CliResult<OnboardFlowStepAction> {
        match step {
            OnboardWizardStep::Welcome => self.run_welcome_step(),
            OnboardWizardStep::Authentication => self.run_authentication_step(draft),
            OnboardWizardStep::RuntimeDefaults => self.run_runtime_defaults_step(draft),
            OnboardWizardStep::Workspace => self.run_workspace_step(draft),
            OnboardWizardStep::Protocols => self.run_protocols_step(draft),
            // Post-boundary steps are handled outside the guided flow loop.
            OnboardWizardStep::EnvironmentCheck
            | OnboardWizardStep::ReviewAndWrite
            | OnboardWizardStep::Ready => Ok(OnboardFlowStepAction::Next),
        }
    }
}

// ---------------------------------------------------------------------------
// Drop — restore terminal
// ---------------------------------------------------------------------------

impl<E: OnboardEventSource> Drop for RatatuiOnboardRunner<E> {
    fn drop(&mut self) {
        if self.owns_tty {
            let _ = terminal::disable_raw_mode();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn step_ordinal(step: OnboardWizardStep) -> usize {
    use crate::onboard_flow::OnboardFlowController;
    OnboardFlowController::ordered_steps()
        .iter()
        .position(|s| *s == step)
        .map(|i| i + 1)
        .unwrap_or(1)
}

fn total_step_count() -> usize {
    use crate::onboard_flow::OnboardFlowController;
    OnboardFlowController::ordered_steps().len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use loongclaw_contracts::SecretRef;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::widgets::StatefulWidget;

    use super::*;
    use crate::onboard_finalize::{
        OnboardingAction, OnboardingActionKind, OnboardingSuccessSummary,
    };
    use crate::onboard_flow::OnboardFlowStepAction;
    use crate::onboard_state::{
        OnboardDraft, OnboardOutcome, OnboardValueOrigin, OnboardWizardStep,
    };
    use crate::onboard_tui::event_source::ScriptedEventSource;
    use crate::test_support::ScopedEnv;

    fn sample_draft() -> OnboardDraft {
        let mut config = mvp::config::LoongClawConfig::default();
        config.memory.sqlite_path = "/tmp/memory.sqlite3".to_owned();
        config.tools.file_root = Some("/tmp/workspace".to_owned());
        config.acp.backend = Some("builtin".to_owned());
        OnboardDraft::from_config(
            config,
            PathBuf::from("/tmp/loongclaw.toml"),
            Some(OnboardValueOrigin::DetectedStartingPoint),
        )
    }

    fn sample_success_summary() -> OnboardingSuccessSummary {
        OnboardingSuccessSummary {
            outcome: OnboardOutcome::Success,
            import_source: Some("recommended plan".to_owned()),
            config_path: "/tmp/loongclaw.toml".to_owned(),
            config_status: Some("config written".to_owned()),
            verification_status: Some("passed".to_owned()),
            provider: "OpenAI".to_owned(),
            saved_provider_profiles: vec!["openai".to_owned()],
            model: "gpt-5".to_owned(),
            transport: "api".to_owned(),
            provider_endpoint: None,
            credential: None,
            prompt_mode: "native".to_owned(),
            personality: Some("default".to_owned()),
            prompt_addendum: None,
            memory_profile: "window_plus_summary".to_owned(),
            web_search_provider: "Brave".to_owned(),
            web_search_credential: None,
            memory_path: Some("/tmp/memory.sqlite3".to_owned()),
            channels: vec!["cli".to_owned()],
            suggested_channels: Vec::new(),
            domain_outcomes: Vec::new(),
            next_actions: vec![OnboardingAction {
                kind: OnboardingActionKind::Ask,
                label: "Ask LoongClaw a question".to_owned(),
                command: "loong ask \"hello\"".to_owned(),
            }],
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl_key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn ctrl_c() -> Event {
        ctrl_key('c')
    }

    static OPENAI_CODEX_OAUTH_TEST_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    static OPENAI_CODEX_OAUTH_START_CALLS: AtomicUsize = AtomicUsize::new(0);
    static OPENAI_CODEX_OAUTH_BROWSER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static OPENAI_CODEX_OAUTH_MANUAL_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn openai_codex_oauth_test_guard() -> std::sync::MutexGuard<'static, ()> {
        let lock = OPENAI_CODEX_OAUTH_TEST_LOCK.get_or_init(|| std::sync::Mutex::new(()));
        lock.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    struct FakeOpenaiCodexOauthFlow {
        manual_only: bool,
    }

    impl crate::openai_codex_oauth::OpenaiCodexOauthFlow for FakeOpenaiCodexOauthFlow {
        fn authorization_url(&self) -> &str {
            "https://auth.openai.test/oauth/authorize?client_id=test-client"
        }

        fn callback_redirect_uri(&self) -> &str {
            "http://localhost:1455/auth/callback"
        }

        fn open_browser(&mut self) -> CliResult<()> {
            OPENAI_CODEX_OAUTH_BROWSER_CALLS.fetch_add(1, Ordering::SeqCst);
            if self.manual_only {
                return Err("browser launch failed in test".to_owned());
            }

            Ok(())
        }

        fn wait_for_browser_callback(
            &mut self,
        ) -> CliResult<crate::openai_codex_oauth::OpenaiCodexOauthGrant> {
            Ok(crate::openai_codex_oauth::OpenaiCodexOauthGrant {
                access_token: "oauth-derived-token".to_owned(),
            })
        }

        fn complete_from_manual_input(
            &mut self,
            input: &str,
        ) -> CliResult<crate::openai_codex_oauth::OpenaiCodexOauthGrant> {
            OPENAI_CODEX_OAUTH_MANUAL_CALLS.fetch_add(1, Ordering::SeqCst);
            if input.trim().is_empty() {
                return Err("manual oauth input was empty".to_owned());
            }

            Ok(crate::openai_codex_oauth::OpenaiCodexOauthGrant {
                access_token: "oauth-derived-token".to_owned(),
            })
        }
    }

    fn fake_openai_codex_oauth_start()
    -> CliResult<Box<dyn crate::openai_codex_oauth::OpenaiCodexOauthFlow>> {
        OPENAI_CODEX_OAUTH_START_CALLS.fetch_add(1, Ordering::SeqCst);
        let flow = FakeOpenaiCodexOauthFlow { manual_only: false };
        Ok(Box::new(flow))
    }

    fn fake_openai_codex_oauth_start_manual_only()
    -> CliResult<Box<dyn crate::openai_codex_oauth::OpenaiCodexOauthFlow>> {
        OPENAI_CODEX_OAUTH_START_CALLS.fetch_add(1, Ordering::SeqCst);
        let flow = FakeOpenaiCodexOauthFlow { manual_only: true };
        Ok(Box::new(flow))
    }

    fn down_events_to_index(index: usize) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..index {
            events.push(key(KeyCode::Down));
        }
        events
    }

    fn move_events_between_indices(from: usize, to: usize) -> Vec<Event> {
        let mut events = Vec::new();
        if to >= from {
            let down_count = to.saturating_sub(from);
            events.extend(down_events_to_index(down_count));
            return events;
        }

        let up_count = from.saturating_sub(to);
        for _ in 0..up_count {
            events.push(key(KeyCode::Up));
        }
        events
    }

    fn add_checked_option_from_first_page(target_index: usize) -> Vec<Event> {
        let initial_focus_index = 0usize;
        let mut events = move_events_between_indices(initial_focus_index, target_index);
        events.push(key(KeyCode::Char(' ')));
        events
    }

    fn replace_checked_option_from_first_page(
        current_checked_index: usize,
        target_index: usize,
    ) -> Vec<Event> {
        let mut events = add_checked_option_from_first_page(target_index);
        if current_checked_index == target_index {
            return events;
        }

        events.extend(move_events_between_indices(
            target_index,
            current_checked_index,
        ));
        events.push(key(KeyCode::Char(' ')));
        events
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("")
    }

    fn span_fg(line: &Line<'_>, index: usize) -> Option<Color> {
        line.spans.get(index).and_then(|span| span.style.fg)
    }

    fn light_palette() -> OnboardPalette {
        OnboardPalette::light()
    }

    fn badge_bg_colors(line: &Line<'_>) -> Vec<Color> {
        line.spans.iter().filter_map(|span| span.style.bg).collect()
    }

    fn buffer_has_fg(buf: &Buffer, color: Color) -> bool {
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].fg == color {
                    return true;
                }
            }
        }
        false
    }

    fn buffer_has_bg(buf: &Buffer, color: Color) -> bool {
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].bg == color {
                    return true;
                }
            }
        }
        false
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            text.push('\n');
        }
        text
    }

    fn rendered_line_start(rendered: &str, needle: &str) -> usize {
        let line = rendered
            .lines()
            .find(|line| line.contains(needle))
            .expect("expected rendered line");
        line.find(needle).expect("expected line start")
    }

    fn rendered_line_index(rendered: &str, needle: &str) -> Option<usize> {
        rendered.lines().position(|line| line.contains(needle))
    }

    fn inline_logo_env(term: &str) -> InlineLogoEnvironment {
        InlineLogoEnvironment {
            term: term.to_owned(),
            ..InlineLogoEnvironment::default()
        }
    }

    #[test]
    fn detect_inline_logo_support_prefers_iterm_protocol_in_wezterm() {
        let mut env = inline_logo_env("wezterm");
        env.term_program = Some("WezTerm".to_owned());

        let support =
            RatatuiOnboardRunner::<ScriptedEventSource>::detect_inline_logo_support_for_env(&env)
                .expect("wezterm support should be detected");
        assert_eq!(support.terminal, InlineLogoTerminal::WezTerm);
        assert_eq!(support.protocol, InlineLogoProtocol::Iterm2);
        assert!(!support.tmux_passthrough);
    }

    #[test]
    fn detect_inline_logo_support_uses_kitty_over_tmux_in_wezterm() {
        let mut env = inline_logo_env("tmux-256color");
        env.term_program = Some("WezTerm".to_owned());
        env.inside_tmux = true;
        env.tmux_passthrough_allowed = true;

        let support =
            RatatuiOnboardRunner::<ScriptedEventSource>::detect_inline_logo_support_for_env(&env)
                .expect("wezterm over tmux should still be detected");
        assert_eq!(support.terminal, InlineLogoTerminal::WezTerm);
        assert_eq!(support.protocol, InlineLogoProtocol::Kitty);
        assert!(support.tmux_passthrough);
    }

    #[test]
    fn detect_inline_logo_support_blocks_tmux_without_passthrough() {
        let mut env = inline_logo_env("tmux-256color");
        env.kitty_window_id_present = true;
        env.inside_tmux = true;
        env.tmux_passthrough_allowed = false;

        let support =
            RatatuiOnboardRunner::<ScriptedEventSource>::detect_inline_logo_support_for_env(&env);
        assert!(
            support.is_none(),
            "tmux should fall back without passthrough"
        );
    }

    #[test]
    fn detect_inline_logo_support_recognizes_ghostty() {
        let mut env = inline_logo_env("xterm-ghostty");
        env.term_program = Some("ghostty".to_owned());

        let support =
            RatatuiOnboardRunner::<ScriptedEventSource>::detect_inline_logo_support_for_env(&env)
                .expect("ghostty support should be detected");
        assert_eq!(support.terminal, InlineLogoTerminal::Ghostty);
        assert_eq!(support.protocol, InlineLogoProtocol::Kitty);
    }

    #[test]
    fn badge_lines_wrap_when_width_is_tight() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::badge_lines(
            [
                RatatuiOnboardRunner::<ScriptedEventSource>::filled_badge_span(
                    "FULLSCREEN",
                    Color::Green,
                ),
                RatatuiOnboardRunner::<ScriptedEventSource>::filled_badge_span(
                    "INTERACTIVE",
                    Color::Cyan,
                ),
                RatatuiOnboardRunner::<ScriptedEventSource>::filled_badge_span(
                    "SHELL HANDOFF",
                    Color::Yellow,
                ),
            ],
            18,
        );
        assert!(
            lines.len() >= 2,
            "tight widths should wrap badge rows instead of clipping"
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("FULLSCREEN"));
        assert!(rendered.contains("INTERACTIVE"));
        assert!(rendered.contains("SHELL HANDOFF"));
    }

    #[test]
    fn selection_stage_lines_carry_stage_identity_and_position() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Workspace,
            "File root",
            1,
            3,
            32,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("File root"));
        assert!(rendered.contains("Choice 2 of 3"));
        assert!(!rendered.contains("Boundary definition"));
        assert!(!rendered.contains("Focus:"));
    }

    #[test]
    fn selection_stage_lines_drop_badge_heavy_copy() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Authentication,
            "Provider",
            0,
            2,
            36,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(!rendered.contains("GUIDED"));
        assert!(!rendered.contains("ENTER LOCKS"));
        assert!(rendered.contains("Provider"));
        assert!(rendered.contains("Choice 1 of 2"));
        assert!(!rendered.contains("Access calibration"));
    }

    #[test]
    fn selection_compact_sidebar_lines_keep_choice_context_without_repeating_stage_headline() {
        let item = SelectionItem::new("OpenAI", Some("current provider"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_compact_sidebar_lines(
            OnboardWizardStep::Authentication,
            "Provider",
            &item,
            0,
            2,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Choice 1 of 2"));
        assert!(rendered.contains("Provider"));
        assert!(rendered.contains("OpenAI"));
        assert!(rendered.contains("current provider"));
        assert!(rendered.contains("Live focus: OpenAI"));
        assert!(rendered.contains("Sets the provider family"));
        assert!(!rendered.contains("Enter continues  Esc back"));
        assert!(!rendered.contains("Access calibration"));
    }

    #[test]
    fn input_stage_lines_reduce_badge_heavy_copy() {
        let state = TextInputState::with_default("/tmp/workspace");
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::input_stage_lines(
            OnboardWizardStep::Workspace,
            "File root:",
            &state,
            32,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Boundary definition"));
        assert!(rendered.contains("File root"));
        assert!(rendered.contains("Current draft stays live until you type."));
        assert!(!rendered.contains("DEFAULT LIVE"));
        assert!(!rendered.contains("ENTER COMMITS"));
    }

    #[test]
    fn launch_status_lines_collapse_stack_into_compact_deck_summary() {
        let summary = sample_success_summary();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::launch_status_lines(
            &summary,
            "The operator deck is configured and ready for first launch.",
            Some(0),
            RatatuiOnboardRunner::<ScriptedEventSource>::palette().success,
            56,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("OpenAI · gpt-5 · loongclaw.toml"));
        assert!(rendered.contains("gpt-5"));
        assert!(rendered.contains("loongclaw.toml"));
        assert!(rendered.contains("1 channel surface ready beside local chat."));
        assert!(!rendered.contains("Provider:"));
        assert!(!rendered.contains("Model:"));
        assert!(!rendered.contains("Config:"));
        assert!(!rendered.contains("Deck:"));
        assert!(!rendered.contains("No action is armed"));
    }

    #[test]
    fn saved_setup_lines_keep_launch_summary_focused_on_user_visible_setup() {
        let summary = sample_success_summary();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::saved_setup_lines(
            &summary,
            RatatuiOnboardRunner::<ScriptedEventSource>::palette().success,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("OpenAI · gpt-5"));
        assert!(rendered.contains("Web search · Brave"));
        assert!(rendered.contains("Channels · cli"));
        assert!(rendered.contains("Memory · /tmp/memory.sqlite3"));
        assert!(!rendered.contains("Control plane"));
        assert!(!rendered.contains("Runtime posture"));
        assert!(!rendered.contains("Transport:"));
        assert!(!rendered.contains("Prompt mode:"));
    }

    #[test]
    fn launch_action_detail_lines_reduce_section_stack_noise() {
        let summary = sample_success_summary();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::launch_action_detail_lines(
            summary.next_actions.first(),
            &summary,
            Some(0),
            68,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Action 1 of 1"));
        assert!(rendered.contains("Ask LoongClaw a question"));
        assert!(rendered.contains("loong ask \"hello\""));
        assert!(!rendered.contains("launch command"));
        assert!(!rendered.contains("first move"));
        assert!(!rendered.contains("why now"));
        assert!(!rendered.contains("what it validates"));
    }

    #[test]
    fn showcase_stage_lines_carry_variant_identity_and_commit_cue() {
        let intro = vec![
            "We found a reusable setup on this machine.".to_owned(),
            "Choose the fastest way to first success.".to_owned(),
        ];
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_stage_lines(
            ShowcaseStageVariant::EntryPath,
            "Choose A Path",
            &intro,
            1,
            3,
            42,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Choose A Path"));
        assert!(rendered.contains("We found a reusable setup on this machine."));
        assert!(rendered.contains("Route 2 of 3"));
        assert!(!rendered.contains("Choose the first move"));
        assert!(!rendered.contains("Tab toggles details"));
        assert!(!rendered.contains("Focus:"));
    }

    #[test]
    fn showcase_stage_lines_keep_welcome_slogan_out_of_choice_flow() {
        let intro = vec![
            "We found a reusable setup on this machine.".to_owned(),
            "Choose the fastest way to first success.".to_owned(),
        ];
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_stage_lines(
            ShowcaseStageVariant::EntryPath,
            "Choose A Path",
            &intro,
            1,
            3,
            42,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(!rendered.contains("Originated from the East,"));
        assert!(!rendered.contains("here to benefit the world."));
        assert!(rendered.contains("Choose A Path"));
    }

    #[test]
    fn showcase_signal_lines_switch_between_snapshot_and_detail_modes() {
        let item = SelectionItem::new("Use current setup", Some("skip detailed edits"));
        let snapshot_lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_signal_lines(
            ShowcaseStageVariant::ShortcutChoice,
            "Current Draft",
            false,
            &item,
            0,
            2,
            40,
        );
        let snapshot_rendered = snapshot_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(snapshot_rendered.contains("TEMPO CHECK"));
        assert!(snapshot_rendered.contains("DECK SNAPSHOT"));
        assert!(snapshot_rendered.contains("Use current setup"));

        let detail_lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_signal_lines(
            ShowcaseStageVariant::ShortcutChoice,
            "Choice Detail",
            true,
            &item,
            0,
            2,
            40,
        );
        let detail_rendered = detail_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(detail_rendered.contains("DETAIL LENS"));
        assert!(detail_rendered.contains("fast handoff"));
    }

    #[test]
    fn showcase_focus_lines_explain_stage_specific_consequence() {
        let item = SelectionItem::new("Current setup", Some("use existing machine state"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_focus_lines(
            ShowcaseStageVariant::EntryPath,
            &item,
            0,
            3,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("First move"));
        assert!(rendered.contains("current machine state"));
        assert!(rendered.contains("opening move"));
    }

    #[test]
    fn showcase_compact_sidebar_lines_merge_stage_and_snapshot_context() {
        let item = SelectionItem::new("Use current setup", Some("skip detailed edits"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_compact_sidebar_lines(
            ShowcaseStageVariant::EntryPath,
            "Choose A Path",
            &[
                "We found a reusable setup on this machine.".to_owned(),
                "Choose the fastest way to first success.".to_owned(),
            ],
            &["provider: openai".to_owned()],
            &item,
            0,
            2,
            false,
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Choose A Path"));
        assert!(rendered.contains("Use current setup"));
        assert!(rendered.contains("We found a reusable setup on this machine."));
        assert!(rendered.contains("provider: openai"));
        assert!(rendered.contains("Enter commits this path."));
    }

    #[test]
    fn showcase_compact_sidebar_lines_drop_snapshot_panel_heading_for_entry_path() {
        let item = SelectionItem::new("Current", Some("use existing machine state"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_compact_sidebar_lines(
            ShowcaseStageVariant::EntryPath,
            "Choose A Path",
            &[
                "We found a reusable setup on this machine.".to_owned(),
                "Choose the fastest way to first success.".to_owned(),
            ],
            &["workspace: /tmp/project".to_owned()],
            &item,
            0,
            2,
            false,
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Choose A Path"));
        assert!(rendered.contains("Current"));
        assert!(rendered.contains("workspace: /tmp/project"));
        assert!(!rendered.contains("Starting posture"));
        assert!(!rendered.contains("Press Tab or l"));
    }

    #[test]
    fn showcase_compact_sidebar_lines_drop_detail_panel_heading_for_entry_path() {
        let item = SelectionItem::new("Fresh", Some("start from a clean draft"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_compact_sidebar_lines(
            ShowcaseStageVariant::EntryPath,
            "Choose A Path",
            &[
                "We found a reusable setup on this machine.".to_owned(),
                "Choose the fastest way to first success.".to_owned(),
            ],
            &["workspace: /tmp/project".to_owned()],
            &item,
            1,
            2,
            true,
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Fresh"));
        assert!(!rendered.contains("Route detail"));
        assert!(!rendered.contains("Lens locked"));
        assert!(!rendered.contains("Press h or Tab"));
    }

    #[test]
    fn showcase_compact_sidebar_lines_drop_choice_counter_and_detail_heading() {
        let item = SelectionItem::new("Current", Some("use existing machine state"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_compact_sidebar_lines(
            ShowcaseStageVariant::EntryPath,
            "Choose A Path",
            &[
                "We found a reusable setup on this machine.".to_owned(),
                "Choose the fastest way to first success.".to_owned(),
            ],
            &["workspace: /tmp/project".to_owned()],
            &item,
            0,
            2,
            true,
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(!rendered.contains("Choice 1 of 2"));
        assert!(!rendered.contains("Highlighted path"));
    }

    #[test]
    fn showcase_snapshot_and_footer_lines_report_live_mode() {
        let item = SelectionItem::new("Use current setup", Some("skip detailed edits"));
        let snapshot_lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_snapshot_lines(
            ShowcaseStageVariant::ShortcutChoice,
            &["provider: openai".to_owned()],
            &item,
        );
        let snapshot_rendered = snapshot_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(snapshot_rendered.contains("Draft momentum snapshot"));
        assert!(snapshot_rendered.contains("provider: openai"));

        let footer = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_footer_status_line(
            ShowcaseStageVariant::ShortcutChoice,
            &item,
            0,
            2,
            true,
        );
        let footer_rendered = line_text(&footer);
        assert!(footer_rendered.contains("shortcut"));
        assert!(footer_rendered.contains("1/2"));
        assert!(footer_rendered.contains("Use current setup"));
    }

    #[test]
    fn showcase_control_copy_compacts_long_footer_hints() {
        let compact = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_control_copy(
            "Up/Down move  1..9 jump  Enter confirm  Tab/h/l detail",
            false,
        );
        assert!(compact.contains("j/k move"));
        assert!(compact.contains("Esc cancel"));
        assert!(!compact.contains("Tab/l detail"));

        let detail =
            RatatuiOnboardRunner::<ScriptedEventSource>::showcase_control_copy("hint", true);
        assert!(detail.contains("Esc cancel"));
    }

    #[test]
    fn showcase_cue_lines_report_stage_specific_cadence() {
        let item = SelectionItem::new("Use current setup", Some("skip detailed edits"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_cue_lines(
            ShowcaseStageVariant::ShortcutChoice,
            false,
            &item,
            "Up/Down move  1..9 jump  Enter confirm  Tab/h/l detail",
            40,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Control cadence"));
        assert!(rendered.contains("FAST COMMIT"));
        assert!(rendered.contains("deliberate operator handoff"));
        assert!(rendered.contains("j/k move"));
    }

    #[test]
    fn showcase_stack_lines_describe_stage_specific_lane_family() {
        let item = SelectionItem::new("Import detected setup", Some("seed from local signals"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_stack_lines(
            ShowcaseStageVariant::DetectedStartingPoint,
            &item,
            1,
            3,
            40,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Detected sources"));
        assert!(rendered.contains("SOURCE PICK"));
        assert!(rendered.contains("Current lane: Import detected setup"));
        assert!(rendered.contains("machine signals"));
    }

    #[test]
    fn decision_stage_signal_lines_report_window_and_progress() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::decision_stage_signal_lines(
            4,
            6,
            20,
            "Brand render: fallback banner.",
            36,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Decision telemetry"));
        assert!(rendered.contains("50% READ"));
        assert!(rendered.contains("Briefing window: 5..10 of 20"));
    }

    #[test]
    fn confirm_compact_hero_lines_reduce_opening_noise() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::confirm_compact_hero_lines(
            "security check",
            4,
            6,
            20,
            "Enter/y accept  n/Esc decline",
            "Render path: headless preview.",
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("security check"));
        assert!(rendered.contains("Briefing window: 5..10 of 20"));
        assert!(rendered.contains("Enter/y accept"));
        assert!(!rendered.contains("Render path:"));
        assert!(!rendered.contains("headless preview"));
        assert!(!rendered.contains("Why This Gate"));
        assert!(!rendered.contains("Stage Signals"));
    }

    #[test]
    fn hero_slogan_lines_keep_brand_slogan_copy() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::hero_slogan_lines();
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");

        assert!(rendered.contains("Originated from the East,"));
        assert!(rendered.contains("here to benefit the world."));
    }

    #[test]
    fn hero_slogan_lines_use_italic_modifier() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::hero_slogan_lines();
        let line = &lines[0];
        let span = line.spans.first().expect("slogan line should have a span");

        assert!(
            span.style.add_modifier.contains(Modifier::ITALIC),
            "brand slogan should render in italic when the terminal supports it"
        );
    }

    #[test]
    fn risk_gate_lines_keep_trust_boundary_copy_without_repeating_slogan() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::risk_gate_lines();
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");

        assert!(rendered.contains("confirm the trust boundary on this machine."));
        assert!(rendered.contains("May read local files and call local tools."));
        assert!(!rendered.contains("Security Check"));
        assert!(!rendered.contains("Originated from the East,"));
        assert!(!rendered.contains("Recommended baseline"));
    }

    #[test]
    fn brand_mark_fallback_lines_carry_stage_tags_and_fallback_note() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::brand_mark_fallback_lines(
            &[
                BrandMarkTag {
                    label: "ENTRY",
                    color: Color::Rgb(241, 101, 116),
                },
                BrandMarkTag {
                    label: "CEREMONY",
                    color: Color::Cyan,
                },
            ],
            false,
            42,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("ENTRY"));
        assert!(rendered.contains("CEREMONY"));
        assert!(rendered.contains("Banner fallback active."));
    }

    #[test]
    fn palette_honors_light_theme_override() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let palette = RatatuiOnboardRunner::<ScriptedEventSource>::palette();
        let expected = light_palette();

        assert_eq!(palette.brand, expected.brand);
        assert_eq!(palette.text, expected.text);
        assert_eq!(palette.surface, expected.surface);
    }

    #[test]
    fn palette_falls_back_to_plain_when_no_color_is_set() {
        let mut env = ScopedEnv::new();
        env.remove("LOONGCLAW_ONBOARD_THEME");
        env.set("NO_COLOR", "1");
        env.remove("COLORFGBG");

        let palette = RatatuiOnboardRunner::<ScriptedEventSource>::palette();
        assert_eq!(palette.brand, Color::Reset);
        assert_eq!(palette.text, Color::Reset);
        assert_eq!(palette.surface, Color::Reset);
    }

    #[test]
    fn palette_detects_light_background_from_colorfgbg() {
        let mut env = ScopedEnv::new();
        env.remove("LOONGCLAW_ONBOARD_THEME");
        env.remove("NO_COLOR");
        env.set("COLORFGBG", "0;15");

        let palette = RatatuiOnboardRunner::<ScriptedEventSource>::palette();
        let expected = light_palette();

        assert_eq!(palette.text, expected.text);
        assert_eq!(palette.surface_emphasis, expected.surface_emphasis);
    }

    #[test]
    fn screen_header_line_follows_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let line =
            RatatuiOnboardRunner::<ScriptedEventSource>::screen_header_line("verify", "Esc cancel");
        let palette = light_palette();

        assert_eq!(span_fg(&line, 0), Some(palette.brand));
        assert_eq!(span_fg(&line, 1), Some(palette.text));
        assert_eq!(span_fg(&line, 3), Some(palette.muted_text));
    }

    #[test]
    fn filled_badge_span_uses_contrasting_text_in_light_theme_brand() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let palette = light_palette();
        let badge =
            RatatuiOnboardRunner::<ScriptedEventSource>::filled_badge_span("LIVE", palette.brand);
        assert_eq!(badge.style.fg, Some(Color::White));
        assert_eq!(badge.style.bg, Some(palette.brand));
    }

    #[test]
    fn filled_badge_span_preserves_reset_colors_in_plain_mode() {
        let mut env = ScopedEnv::new();
        env.set("NO_COLOR", "1");
        env.remove("LOONGCLAW_ONBOARD_THEME");
        env.remove("COLORFGBG");

        let palette = RatatuiOnboardRunner::<ScriptedEventSource>::palette();
        let badge =
            RatatuiOnboardRunner::<ScriptedEventSource>::filled_badge_span("PLAIN", palette.brand);
        assert_eq!(badge.style.fg, Some(Color::Reset));
        assert_eq!(badge.style.bg, Some(Color::Reset));
    }

    #[test]
    fn preflight_summary_lines_follow_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::preflight_summary_lines(3, 1, 0);
        let palette = light_palette();

        assert_eq!(span_fg(&lines[0], 0), Some(palette.brand));
        assert_eq!(span_fg(&lines[1], 0), Some(palette.secondary_text));
        assert_eq!(span_fg(&lines[2], 0), Some(palette.muted_text));
    }

    #[test]
    fn brand_ascii_logo_lines_use_wide_wordmark_when_area_allows() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::brand_ascii_logo_lines(84, 6);
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("██╗      ██████╗"));
        assert!(rendered.contains("███████╗╚██████╔╝"));
    }

    #[test]
    fn brand_ascii_logo_lines_keep_block_wordmark_when_height_is_tight() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::brand_ascii_logo_lines(84, 5);
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");

        assert!(
            rendered.contains("██╗      ██████╗"),
            "tight heights should still keep the block wordmark visible: {rendered}"
        );
        assert!(
            !rendered.contains("\nLOONG\nCLAW\n"),
            "plain split fallback should not replace the block wordmark: {rendered}"
        );
    }

    #[test]
    fn welcome_support_lines_do_not_surface_render_transport_copy() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::welcome_support_lines_for_route(
            "Render path: headless preview.",
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");

        assert!(rendered.contains("Nothing is written until Verify & Write."));
        assert!(rendered.contains("Safe to rerun whenever your setup changes."));
        assert!(!rendered.contains("Render path:"));
        assert!(!rendered.contains("headless preview"));
    }

    #[test]
    fn brand_ascii_logo_lines_use_stacked_block_fallback_before_plain_split() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::brand_ascii_logo_lines(45, 13);
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");

        assert!(
            rendered.contains("██╗      ██████╗  ██████╗ ███╗   ██╗ ██████╗"),
            "stacked fallback should keep the first half of the block wordmark: {rendered}"
        );
        assert!(
            rendered.contains("██████╗██╗      █████╗ ██╗    ██╗"),
            "stacked fallback should keep the second half of the block wordmark: {rendered}"
        );
        assert!(
            !rendered.contains("\nLOONG\nCLAW\n"),
            "stacked fallback should win before the plain split wordmark: {rendered}"
        );
    }

    #[test]
    fn inset_rect_adds_horizontal_breathing_room() {
        let area = Rect::new(10, 4, 24, 5);
        let inset = RatatuiOnboardRunner::<ScriptedEventSource>::inset_rect(area, 2, 0);

        assert_eq!(inset.x, 12);
        assert_eq!(inset.width, 20);
        assert_eq!(inset.y, 4);
        assert_eq!(inset.height, 5);
    }

    #[test]
    fn showcase_shell_layout_keeps_a_blank_row_below_header() {
        let area = Rect::new(0, 0, 120, 28);
        let shell = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_shell_sections(area);

        assert_eq!(shell[0].height, 1);
        assert_eq!(shell[1].height, 1);
        assert_eq!(shell[2].y, 2);
    }

    #[test]
    fn guided_shell_content_area_keeps_a_blank_row_below_header() {
        let content_area = Rect::new(12, 3, 92, 20);
        let shell_area =
            RatatuiOnboardRunner::<ScriptedEventSource>::shell_content_area(content_area);

        assert_eq!(shell_area.x, 12);
        assert_eq!(shell_area.y, 4);
        assert_eq!(shell_area.width, 92);
        assert_eq!(shell_area.height, 19);
    }

    #[test]
    fn guided_selection_stage_uses_stacked_wordmark_on_narrow_width() {
        let backend = TestBackend::new(72, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let stage_lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Authentication,
            "OpenAI API model",
            0,
            6,
            0,
        );

        terminal
            .draw(|frame| {
                let shell_area = Rect::new(0, 0, 52, 16);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_guided_brand_stage(
                    frame,
                    shell_area,
                    &stage_lines,
                    &[],
                    Alignment::Center,
                    Alignment::Center,
                    72,
                    0,
                );
            })
            .expect("guided brand stage render should succeed");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(
            rendered.contains("██╗      ██████╗  ██████╗"),
            "narrow guided stage should keep stacked block wordmark: {rendered}"
        );
        assert!(
            rendered.contains("██████╗██╗      █████╗"),
            "narrow guided stage should render stacked second block: {rendered}"
        );
        assert!(
            !rendered.contains("LOONG\n"),
            "narrow guided stage should not fall back to plain split wordmark: {rendered}"
        );
    }

    #[test]
    fn guided_selection_stage_uses_full_wordmark_when_shell_is_wide_enough() {
        let backend = TestBackend::new(112, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let stage_lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Authentication,
            "Providers",
            0,
            12,
            0,
        );
        let full_wordmark_width = HERO_WORDMARK_LINES
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let full_wordmark_width = u16::try_from(full_wordmark_width).ok().unwrap_or(0);

        terminal
            .draw(|frame| {
                let shell_area = Rect::new(0, 0, full_wordmark_width, 11);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_guided_brand_stage(
                    frame,
                    shell_area,
                    &stage_lines,
                    &[],
                    Alignment::Center,
                    Alignment::Center,
                    72,
                    0,
                );
            })
            .expect("guided brand stage render should succeed");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(
            rendered.contains(
                "██╗      ██████╗  ██████╗ ███╗   ██╗ ██████╗  ██████╗██╗      █████╗ ██╗    ██╗"
            ),
            "guided selection stage should keep the full single-line wordmark when the shell is wide enough: {rendered}"
        );
        assert!(
            !rendered.contains("LOONG\n"),
            "guided selection stage should not fall back to the plain split wordmark when the shell is wide enough: {rendered}"
        );
    }

    #[test]
    fn guided_brand_stage_keeps_one_row_of_breathing_room_above_wordmark() {
        let backend = TestBackend::new(96, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let stage_lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Authentication,
            "Providers",
            0,
            4,
            0,
        );

        terminal
            .draw(|frame| {
                let shell_area = Rect::new(0, 0, 96, 10);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_guided_brand_stage(
                    frame,
                    shell_area,
                    &stage_lines,
                    &[],
                    Alignment::Center,
                    Alignment::Center,
                    72,
                    0,
                );
            })
            .expect("guided brand stage render should succeed");

        let rendered = buffer_text(terminal.backend().buffer());
        let logo_row = rendered_line_index(rendered.as_str(), "██╗      ██████╗")
            .expect("wide block wordmark should render");

        assert!(
            logo_row > 0,
            "guided brand stage should keep breathing room above the wordmark: {rendered}"
        );
    }

    #[test]
    fn selection_shell_sections_preserve_stage_room_for_stacked_wordmark() {
        let stage_lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Authentication,
            "Providers",
            0,
            6,
            0,
        );
        let shell = RatatuiOnboardRunner::<ScriptedEventSource>::selection_shell_sections(
            Rect::new(0, 0, 52, 19),
            &stage_lines,
            6,
        );

        assert!(
            shell[0].height >= 16,
            "narrow selection shell should reserve enough room for the stacked block wordmark"
        );
        assert_eq!(shell[1].height, 3);
    }

    #[test]
    fn selection_shell_sections_compact_stage_when_full_wordmark_fits() {
        let full_wordmark_width = HERO_WORDMARK_LINES
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let full_wordmark_width = u16::try_from(full_wordmark_width).ok().unwrap_or(0);
        let stage_lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Authentication,
            "Providers",
            0,
            12,
            0,
        );
        let shell = RatatuiOnboardRunner::<ScriptedEventSource>::selection_shell_sections(
            Rect::new(0, 0, full_wordmark_width, 24),
            &stage_lines,
            12,
        );

        assert_eq!(
            shell[0].height, 10,
            "wide selection shells should keep the compact single-line wordmark stage"
        );
        assert!(
            shell[1].height >= 14,
            "wide selection shells should hand the reclaimed height back to the selection list"
        );
    }

    #[test]
    fn selection_shell_sections_keep_short_selection_block_near_the_stage() {
        let content_area = Rect::new(0, 0, 96, 24);
        let stage_lines = RatatuiOnboardRunner::<ScriptedEventSource>::selection_stage_lines(
            OnboardWizardStep::Authentication,
            "Default Provider",
            0,
            2,
            0,
        );
        let desired_stage_height =
            RatatuiOnboardRunner::<ScriptedEventSource>::guided_brand_stage_desired_height(
                content_area.width,
                &stage_lines,
                &[],
            );
        let shell = RatatuiOnboardRunner::<ScriptedEventSource>::selection_shell_sections(
            content_area,
            &stage_lines,
            2,
        );

        assert_eq!(
            shell[0].height, desired_stage_height,
            "tall layouts should keep the stage compact instead of absorbing extra height"
        );
        assert_eq!(
            shell[1].height, 6,
            "a two-item selection should keep its natural two-card height"
        );
        assert_eq!(
            shell[1].y,
            shell[0].bottom(),
            "the selection block should sit directly below the stage"
        );
        assert!(
            shell[2].height > 0,
            "unused height should remain as filler below the selection block"
        );
    }

    #[test]
    fn focus_selection_panel_height_uses_full_available_height_for_tall_lists() {
        let height =
            RatatuiOnboardRunner::<ScriptedEventSource>::focus_selection_panel_height(24, 42);

        assert_eq!(
            height, 24,
            "tall selection pages should use the full available height instead of stopping at a hard cap"
        );
    }

    #[test]
    fn guided_input_stage_keeps_full_wordmark_at_standard_shell_height() {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let input_state = TextInputState::with_default("OPENAI_API_KEY");
        let stage_lines = RatatuiOnboardRunner::<ScriptedEventSource>::input_stage_lines(
            OnboardWizardStep::Authentication,
            "Provider credential env:",
            &input_state,
            28,
        );

        terminal
            .draw(|frame| {
                let shell_area = Rect::new(0, 0, 105, 11);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_guided_brand_stage(
                    frame,
                    shell_area,
                    &stage_lines,
                    &[],
                    Alignment::Center,
                    Alignment::Center,
                    72,
                    0,
                );
            })
            .expect("guided brand stage render should succeed");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(
            rendered.contains("██╗      ██████╗"),
            "guided input stage should keep the full wordmark visible: {rendered}"
        );
        assert!(
            !rendered.contains("LOONG\n"),
            "guided input stage should not fall back to the split wordmark: {rendered}"
        );
    }

    #[test]
    fn guided_brand_stage_rerender_clears_previous_longer_copy() {
        let backend = TestBackend::new(96, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let first_lines = vec![Line::from("THIS COPY MUST DISAPPEAR AFTER THE NEXT DRAW")];
        let second_lines = vec![Line::from("Short")];
        let stage_area = Rect::new(0, 0, 96, 12);

        terminal
            .draw(|frame| {
                RatatuiOnboardRunner::<ScriptedEventSource>::render_guided_brand_stage(
                    frame,
                    stage_area,
                    &first_lines,
                    &[],
                    Alignment::Center,
                    Alignment::Center,
                    72,
                    0,
                );
            })
            .expect("first draw should succeed");

        terminal
            .draw(|frame| {
                RatatuiOnboardRunner::<ScriptedEventSource>::render_guided_brand_stage(
                    frame,
                    stage_area,
                    &second_lines,
                    &[],
                    Alignment::Center,
                    Alignment::Center,
                    72,
                    0,
                );
            })
            .expect("second draw should succeed");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(
            !rendered.contains("MUST DISAPPEAR"),
            "second render should clear stale stage copy: {rendered}"
        );
        assert!(rendered.contains("Short"));
    }

    #[test]
    fn risk_centerpiece_uses_open_layout_without_outer_frame() {
        let backend = TestBackend::new(120, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::risk_gate_lines();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 120, 28);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_risk_centerpiece(
                    frame,
                    area,
                    "v0.1.0-alpha.2 · branch · abc1234",
                    &lines,
                );
            })
            .expect("risk centerpiece render should succeed");

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Security Check"));
        assert!(!text.contains("Originated from the East,"));
        assert!(!text.contains("╭"));
        assert!(!text.contains("╰"));
    }

    #[test]
    fn risk_centerpiece_keeps_stacked_wordmark_on_narrow_width() {
        let backend = TestBackend::new(72, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::risk_gate_lines();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 72, 28);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_risk_centerpiece(
                    frame,
                    area,
                    "v0.1.0-alpha.2 · branch · abc1234",
                    &lines,
                );
            })
            .expect("risk centerpiece render should succeed");

        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("██╗      ██████╗  ██████╗"),
            "risk centerpiece should keep stacked block header on narrow width: {text}"
        );
        assert!(
            text.contains("██████╗██╗      █████╗"),
            "risk centerpiece should render the stacked lower half too: {text}"
        );
    }

    #[test]
    fn risk_centerpiece_centers_security_copy_instead_of_left_aligning_it() {
        let backend = TestBackend::new(120, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::risk_gate_lines();

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 120, 28);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_risk_centerpiece(
                    frame,
                    area,
                    "v0.1.0-alpha.2 · branch · abc1234",
                    &lines,
                );
            })
            .expect("risk centerpiece render should succeed");

        let rendered = buffer_text(terminal.backend().buffer());
        let security_start = rendered_line_start(rendered.as_str(), "Security Check");
        let bullet_start = rendered_line_start(
            rendered.as_str(),
            "• May read local files and call local tools.",
        );

        assert!(
            security_start > bullet_start,
            "shorter centered lines should start further right than longer bullet lines: {rendered}"
        );
    }

    #[test]
    fn welcome_centerpiece_uses_open_layout_without_outer_frame() {
        let backend = TestBackend::new(120, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        let primary_lines = RatatuiOnboardRunner::<ScriptedEventSource>::welcome_primary_lines();
        let support_lines =
            RatatuiOnboardRunner::<ScriptedEventSource>::welcome_support_lines_for_route(
                "Render path: headless preview.",
            );

        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 120, 28);
                RatatuiOnboardRunner::<ScriptedEventSource>::render_welcome_centerpiece(
                    frame,
                    area,
                    "v0.1.0-alpha.2 · branch · abc1234",
                    &primary_lines,
                    &support_lines,
                    false,
                );
            })
            .expect("welcome centerpiece render should succeed");

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Originated from the East,"));
        assert!(text.contains("Press Enter to begin."));
        assert!(text.contains("██╗      ██████╗"));
        assert!(!text.contains("█      ████   ████"));
        assert!(!text.contains("╭"));
        assert!(!text.contains("╰"));
    }

    #[test]
    fn brand_logo_base64_now_points_to_icon_asset() {
        let expected = BASE64_STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/logo/loongclaw-icon.png"
        )));
        assert_eq!(
            RatatuiOnboardRunner::<ScriptedEventSource>::brand_logo_base64(),
            expected
        );
    }

    #[test]
    fn welcome_brand_media_uses_ascii_wordmark_even_when_inline_logo_is_available() {
        let backend = TestBackend::new(48, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        let mut captured = None;
        terminal
            .draw(|frame| {
                captured = RatatuiOnboardRunner::<ScriptedEventSource>::render_welcome_brand_media(
                    frame,
                    Rect::new(0, 0, 40, 8),
                    true,
                    Style::default(),
                );
            })
            .expect("welcome media render should succeed");

        assert!(captured.is_none());
        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("LOONG"));
        assert!(rendered.contains("CLAW"));
    }

    #[test]
    fn welcome_brand_media_falls_back_to_ascii_wordmark_when_inline_logo_is_unavailable() {
        let backend = TestBackend::new(48, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| {
                let logo_area =
                    RatatuiOnboardRunner::<ScriptedEventSource>::render_welcome_brand_media(
                        frame,
                        Rect::new(0, 0, 40, 8),
                        false,
                        Style::default(),
                    );
                assert!(logo_area.is_none());
            })
            .expect("welcome media render should succeed");

        let rendered = buffer_text(terminal.backend().buffer());
        assert!(rendered.contains("LOONG"));
        assert!(rendered.contains("CLAW"));
    }

    #[test]
    fn verify_gate_hero_keeps_logo_and_target_visible_at_80_columns() {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 80, 12);
                let accent = RatatuiOnboardRunner::<ScriptedEventSource>::palette().warning;
                let hero_block = RatatuiOnboardRunner::<ScriptedEventSource>::rounded_panel(
                    "Write Gate",
                    accent,
                )
                .style(RatatuiOnboardRunner::<ScriptedEventSource>::panel_fill_style(accent));
                let hero_inner = hero_block.inner(area);
                frame.render_widget(hero_block, area);

                let hero_body = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(4), Constraint::Min(6)])
                    .split(hero_inner);
                frame.render_widget(
                    Paragraph::new(
                        RatatuiOnboardRunner::<ScriptedEventSource>::brand_ascii_logo_lines(
                            hero_body[0].width,
                            hero_body[0].height,
                        ),
                    )
                    .alignment(Alignment::Center)
                    .style(RatatuiOnboardRunner::<ScriptedEventSource>::panel_fill_style(accent)),
                    hero_body[0],
                );
                frame.render_widget(
                    Paragraph::new(
                        RatatuiOnboardRunner::<ScriptedEventSource>::verify_gate_narrow_hero_lines(
                            "ready with warnings",
                            3,
                            1,
                            0,
                            "/tmp/loongclaw.toml",
                            hero_body[1].width.saturating_sub(2),
                        ),
                    )
                    .alignment(Alignment::Left)
                    .wrap(Wrap { trim: false })
                    .style(RatatuiOnboardRunner::<ScriptedEventSource>::panel_fill_style(accent)),
                    hero_body[1],
                );
            })
            .expect("hero render should succeed");

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Write Gate"));
        assert!(text.contains("ready with warnings"));
        assert!(text.contains("Write target: loongclaw.toml"), "{text}");
        assert!(
            text.contains("██╗      ██████╗"),
            "verify gate hero should keep the block wordmark visible: {text}"
        );
    }

    #[test]
    fn verify_gate_narrow_hero_lines_preserve_write_target_and_enter_copy() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::verify_gate_narrow_hero_lines(
            "ready with warnings",
            3,
            1,
            0,
            "/tmp/loongclaw.toml",
            72,
        );
        assert_eq!(lines.len(), 6);
        assert_eq!(line_text(&lines[4]), "Write target: loongclaw.toml");
        assert_eq!(
            line_text(&lines[5]),
            "Enter writes with warnings and continues."
        );
    }

    #[test]
    fn verify_gate_compact_hero_lines_follow_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::verify_gate_compact_hero_lines(
            "ready with warnings",
            "Review the verification warnings, then decide whether this draft should be written.",
            3,
            1,
            0,
            "/tmp/loongclaw.toml",
            42,
        );
        let palette = light_palette();

        assert_eq!(span_fg(&lines[0], 0), Some(palette.warning));
        let badge_colors = lines.iter().flat_map(badge_bg_colors).collect::<Vec<_>>();
        assert!(badge_colors.contains(&palette.warning));
        assert!(badge_colors.contains(&palette.info));
        assert_eq!(span_fg(&lines[1], 0), Some(palette.secondary_text));
    }

    #[test]
    fn help_binding_line_follows_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let line = RatatuiOnboardRunner::<ScriptedEventSource>::help_binding_line("Enter", "begin");
        let palette = light_palette();

        assert_eq!(span_fg(&line, 0), Some(palette.brand));
        assert_eq!(span_fg(&line, 1), Some(palette.secondary_text));
    }

    #[test]
    fn info_stage_lines_keep_reading_window_without_guide_panel_copy() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::info_stage_lines(
            "preflight results",
            2,
            5,
            18,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");

        assert!(rendered.contains("preflight results"));
        assert!(rendered.contains("Read 3..7 of 18"));
        assert!(rendered.contains("Enter or Esc returns"));
        assert!(!rendered.contains("Operator guide"));
        assert!(!rendered.contains("audit-only"));
    }

    #[test]
    fn guided_footer_line_follows_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let line =
            RatatuiOnboardRunner::<ScriptedEventSource>::guided_footer_line(4, "Esc back", 48);
        let palette = light_palette();

        assert_eq!(span_fg(&line, 0), Some(palette.secondary_text));
        assert_eq!(span_fg(&line, 2), Some(palette.muted_text));
    }

    #[test]
    fn shell_footer_line_follows_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let line = RatatuiOnboardRunner::<ScriptedEventSource>::shell_footer_line("Enter to exit");
        let palette = light_palette();

        assert_eq!(span_fg(&line, 0), Some(palette.secondary_text));
    }

    #[test]
    fn brand_mark_story_lines_follow_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::brand_mark_story_lines(false, 48);
        let palette = light_palette();

        assert_eq!(span_fg(&lines[0], 0), Some(palette.muted_text));
    }

    #[test]
    fn showcase_signal_lines_follow_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let item = SelectionItem::new("Use current setup", Some("skip detailed edits"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_signal_lines(
            ShowcaseStageVariant::EntryPath,
            "Choose A Path",
            false,
            &item,
            0,
            3,
            48,
        );
        let palette = light_palette();

        assert_eq!(span_fg(&lines[0], 0), Some(palette.brand));
        assert_eq!(
            badge_bg_colors(&lines[2]),
            vec![palette.brand, palette.success, palette.warning]
        );
        assert_eq!(span_fg(&lines[4], 0), Some(palette.text));
        assert_eq!(span_fg(&lines[5], 0), Some(palette.secondary_text));
        assert_eq!(span_fg(&lines[9], 0), Some(palette.muted_text));
    }

    #[test]
    fn showcase_snapshot_lines_follow_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let item = SelectionItem::new("Use current setup", Some("skip detailed edits"));
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::showcase_snapshot_lines(
            ShowcaseStageVariant::ShortcutChoice,
            &["provider: openai".to_owned()],
            &item,
        );
        let palette = light_palette();

        assert_eq!(span_fg(&lines[0], 0), Some(palette.brand));
        assert_eq!(span_fg(&lines[2], 0), Some(palette.secondary_text));
        assert_eq!(span_fg(&lines[4], 0), Some(palette.secondary_text));
    }

    #[test]
    fn render_scrollable_panel_uses_light_palette_for_inactive_border() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let backend = TestBackend::new(48, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| {
                RatatuiOnboardRunner::<ScriptedEventSource>::render_scrollable_panel(
                    frame,
                    Rect::new(2, 2, 30, 8),
                    "Verification",
                    light_palette().brand,
                    false,
                    &[Line::from("provider: openai")],
                    0,
                    Alignment::Left,
                );
            })
            .expect("panel render should succeed");
        let buf = terminal.backend().buffer();
        assert!(buffer_has_fg(buf, light_palette().border));
    }

    #[test]
    fn render_help_overlay_uses_light_warning_palette() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| {
                RatatuiOnboardRunner::<ScriptedEventSource>::render_help_overlay(
                    frame,
                    Rect::new(0, 0, 80, 24),
                    HelpOverlayTopic::Welcome,
                );
            })
            .expect("help overlay render should succeed");
        let buf = terminal.backend().buffer();
        let palette = light_palette();

        assert!(buffer_has_fg(buf, palette.warning));
        assert!(buffer_has_bg(buf, palette.warning_surface));
    }

    #[test]
    fn welcome_hero_lines_follow_light_palette_tokens() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::welcome_hero_lines("v1.2.3");
        let palette = light_palette();

        assert_eq!(span_fg(&lines[0], 0), Some(palette.muted_text));
        assert_eq!(span_fg(&lines[2], 0), Some(palette.text));
        assert_eq!(span_fg(&lines[5], 0), Some(palette.secondary_text));
        assert_eq!(span_fg(&lines[8], 0), Some(palette.brand));
    }

    #[test]
    fn verify_gate_compact_hero_lines_reduce_panel_noise() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::verify_gate_compact_hero_lines(
            "ready with warnings",
            "Review the verification warnings, then decide whether this draft should be written.",
            6,
            1,
            0,
            "/tmp/loongclaw.toml",
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("ready with warnings"));
        assert!(rendered.contains("Checks: 6 pass  1 warn  0 fail"));
        assert!(rendered.contains("Write target: /tmp/loongclaw.toml"));
        assert!(rendered.contains("REVIEW"));
        assert!(rendered.contains("WRITE WITH WARNINGS"));
        assert!(!rendered.contains("Stage Signals"));
        assert!(!rendered.contains("Pane telemetry"));
        assert!(!rendered.contains("VERIFY LIVE"));
    }

    #[test]
    fn launch_stage_signal_lines_show_armed_command() {
        let summary = sample_success_summary();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::launch_stage_signal_lines(
            &summary,
            true,
            summary.next_actions.first(),
            "Brand render: fallback banner.",
            42,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Handoff telemetry"));
        assert!(rendered.contains("COMMAND ARMED"));
        assert!(rendered.contains("loong ask \"hello\""));
    }

    #[test]
    fn verify_gate_compact_hero_lines_switch_write_outcomes() {
        let ready_lines =
            RatatuiOnboardRunner::<ScriptedEventSource>::verify_gate_compact_hero_lines(
                "ready to write",
                "Verification is clear. Review the final draft, then continue when the draft reads cleanly.",
                0,
                0,
                0,
                "/tmp/loongclaw.toml",
                42,
            );
        let ready_rendered = ready_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(ready_rendered.contains("WRITE READY"));
        assert!(!ready_rendered.contains("launch deck"));
        assert!(!ready_rendered.contains("opens launch"));

        let blocked_lines =
            RatatuiOnboardRunner::<ScriptedEventSource>::verify_gate_compact_hero_lines(
                "blocked by verification",
                "Verification found blockers. Review the failing checks before attempting to write this deck.",
                1,
                0,
                1,
                "/tmp/loongclaw.toml",
                42,
            );
        let blocked_rendered = blocked_lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(blocked_rendered.contains("RETURN ONLY"));
        assert!(blocked_rendered.contains(
            "Enter leaves this gate without writing so the draft can be corrected first."
        ));
    }

    #[test]
    fn launch_handoff_lines_show_direct_exit_sequence() {
        let summary = sample_success_summary();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::launch_handoff_lines(
            summary.next_actions.first(),
            &summary,
            true,
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(!rendered.contains("Command handoff"));
        assert!(rendered.contains("loong chat --config '/tmp/loongclaw.toml'"));
        assert!(rendered.contains("Enter opens chat"));
    }

    #[test]
    fn launch_handoff_lines_use_short_direct_exit_copy() {
        let summary = sample_success_summary();
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::launch_handoff_lines(
            summary.next_actions.first(),
            &summary,
            true,
            48,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Enter opens chat"));
        assert!(!rendered.contains("1. keep this action armed"));
        assert!(!rendered.contains("2. press Enter to leave the deck"));
    }

    #[test]
    fn launch_action_color_stays_on_brand_across_paths() {
        let palette = RatatuiOnboardRunner::<ScriptedEventSource>::palette();
        assert_eq!(
            RatatuiOnboardRunner::<ScriptedEventSource>::launch_action_color(
                OnboardingActionKind::Chat,
            ),
            palette.brand,
        );
        assert_eq!(
            RatatuiOnboardRunner::<ScriptedEventSource>::launch_action_color(
                OnboardingActionKind::BrowserPreview,
            ),
            palette.brand,
        );
        assert_eq!(
            RatatuiOnboardRunner::<ScriptedEventSource>::launch_action_color(
                OnboardingActionKind::Doctor,
            ),
            palette.brand,
        );
    }

    #[test]
    fn saved_setup_lines_use_compact_launch_summary_sections() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::saved_setup_lines(
            &sample_success_summary(),
            Color::Green,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Configured now"));
        assert!(rendered.contains("Provider stack"));
        assert!(rendered.contains("Surfaces"));
        assert!(!rendered.contains("Control plane"));
        assert!(!rendered.contains("Runtime posture"));
        assert!(!rendered.contains("Local surfaces"));
    }

    #[test]
    fn welcome_step_returns_next_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert_eq!(result.unwrap(), OnboardFlowStepAction::Next);
    }

    #[test]
    fn welcome_step_can_skip_the_first_guided_visit_once() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        runner.skip_next_guided_welcome_once();

        let skipped = runner.run_welcome_step();
        assert_eq!(skipped.unwrap(), OnboardFlowStepAction::Next);

        let rendered = runner.run_welcome_step();
        assert_eq!(rendered.unwrap(), OnboardFlowStepAction::Next);
    }

    #[test]
    fn welcome_step_returns_error_on_esc() {
        let events = vec![key(KeyCode::Esc)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "onboarding cancelled");
    }

    #[test]
    fn welcome_step_returns_error_on_ctrl_c() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert!(result.is_err());
    }

    #[test]
    fn welcome_step_closes_help_then_continues() {
        let events = vec![
            key(KeyCode::Char('?')),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert_eq!(result.unwrap(), OnboardFlowStepAction::Next);
    }

    #[test]
    fn welcome_hero_lines_stay_focused_without_badge_noise() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::welcome_hero_lines("v1.2.3");
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");

        assert!(rendered.contains("v1.2.3"));
        assert!(rendered.contains("Originated from the East,"));
        assert!(rendered.contains("here to benefit the world."));
        assert!(rendered.contains("Press Enter to begin."));
        assert!(!rendered.contains("FULLSCREEN"));
        assert!(!rendered.contains("INTERACTIVE"));
        assert!(!rendered.contains("NO WRITE YET"));
    }

    #[test]
    fn welcome_compact_lines_merge_support_into_single_hero() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::welcome_compact_lines(
            "v1.2.3",
            "Render path: headless preview.",
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Originated from the East,"));
        assert!(rendered.contains("Nothing is written until Verify & Write."));
        assert!(rendered.contains("Safe to rerun whenever your setup changes."));
        assert!(!rendered.contains("Render path:"));
        assert!(!rendered.contains("Before You Begin"));
    }

    #[test]
    fn selection_loop_returns_selected_index() {
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(matches!(result, SelectionLoopResult::Selected(1)));
    }

    #[test]
    fn provider_selection_loop_starts_from_first_catalog_page() {
        let events = vec![key(KeyCode::Char(' ')), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let draft = sample_draft();

        let selected_options = runner
            .run_provider_selection_loop(&draft.config.provider)
            .expect("provider picker should render")
            .expect("provider picker should submit");
        let first_selected_kind = selected_options
            .first()
            .map(|option| option.kind)
            .expect("at least one provider should remain selected");
        let current_provider_kind = draft.config.provider.kind;
        let includes_current_provider = selected_options
            .iter()
            .any(|option| option.kind == current_provider_kind);

        assert_eq!(
            first_selected_kind,
            mvp::config::ProviderKind::Anthropic,
            "toggling immediately on entry should affect the first visible provider rather than a later preselected one",
        );
        assert!(
            includes_current_provider,
            "the current provider should remain pre-checked until the operator changes it explicitly",
        );
    }

    #[test]
    fn selection_loop_wraps_around() {
        // Start at 0, go up (wraps to last), then enter
        let events = vec![key(KeyCode::Up), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
            SelectionItem::new("C", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(matches!(result, SelectionLoopResult::Selected(2)));
    }

    #[test]
    fn selection_loop_supports_numeric_jump_and_edge_keys() {
        let events = vec![
            key(KeyCode::Char('3')),
            key(KeyCode::Char('g')),
            key(KeyCode::Char('G')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
            SelectionItem::new("C", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(matches!(result, SelectionLoopResult::Selected(2)));
    }

    #[test]
    fn selection_loop_supports_page_navigation() {
        let events = vec![key(KeyCode::PageDown), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
            SelectionItem::new("C", None::<&str>),
            SelectionItem::new("D", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(matches!(result, SelectionLoopResult::Selected(3)));
    }

    #[test]
    fn showcase_selection_loop_supports_detail_lens_shortcuts() {
        let events = vec![
            key(KeyCode::Right),
            key(KeyCode::Char('2')),
            key(KeyCode::Char('h')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", Some("first path")),
            SelectionItem::new("B", Some("second path")),
        ];
        let result = runner
            .run_showcase_selection_loop(
                ShowcaseStageVariant::EntryPath,
                "choose a path",
                "Choices",
                &HERO_INTRO_LINES,
                "Snapshot",
                &["workspace: /tmp/project".to_owned()],
                items,
                0,
                "hint",
            )
            .unwrap();
        assert!(matches!(result, StandaloneSelectionResult::Selected(1)));
    }

    #[test]
    fn standalone_selection_loop_uses_fullscreen_showcase_navigation() {
        let events = vec![
            key(KeyCode::Char('l')),
            key(KeyCode::Down),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", Some("first path")),
            SelectionItem::new("B", Some("second path")),
        ];
        let result = runner
            .run_standalone_selection_loop("compat gate", items, 0, "hint")
            .unwrap();
        assert!(matches!(result, StandaloneSelectionResult::Selected(1)));
    }

    #[test]
    fn input_loop_returns_typed_value() {
        let events = vec![
            key(KeyCode::Char('h')),
            key(KeyCode::Char('i')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "hi"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn input_loop_returns_default_on_immediate_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "/default", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "/default"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn input_loop_handles_backspace() {
        let events = vec![
            key(KeyCode::Char('a')),
            key(KeyCode::Char('b')),
            key(KeyCode::Backspace),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "a"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn input_loop_accepts_pasted_text() {
        let events = vec![
            Event::Paste("hello://world?ok=1".to_owned()),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "hello://world?ok=1"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn input_loop_supports_ctrl_navigation_and_clear() {
        let events = vec![
            key(KeyCode::Char('a')),
            key(KeyCode::Char('b')),
            key(KeyCode::Char('c')),
            ctrl_key('a'),
            key(KeyCode::Char('X')),
            ctrl_key('e'),
            key(KeyCode::Char('Y')),
            ctrl_key('u'),
            key(KeyCode::Char('o')),
            key(KeyCode::Char('k')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "ok"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn runtime_defaults_step_sets_memory_profile() {
        // Down once to select window_plus_summary, then accept personality and surfaces.
        let events = vec![
            key(KeyCode::Down),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let result = runner.run_runtime_defaults_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(
            draft.config.memory.profile,
            mvp::config::MemoryProfile::WindowPlusSummary
        );
    }

    #[test]
    fn workspace_step_sets_paths() {
        // Accept default sqlite path (Enter), then type custom file root + Enter
        let events = vec![
            key(KeyCode::Enter),
            key(KeyCode::Char('/')),
            key(KeyCode::Char('x')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let result = runner.run_workspace_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(draft.workspace.file_root, PathBuf::from("/x"));
    }

    #[test]
    fn protocols_step_disables_acp() {
        // Accept channels as-is, then select "Disabled" (Down once), then Enter.
        let events = vec![key(KeyCode::Enter), key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        draft.set_acp_enabled(true);
        let result = runner.run_protocols_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert!(!draft.protocols.acp_enabled);
    }

    #[test]
    fn protocols_step_selects_backend_when_enabled() {
        // Draft starts with acp_enabled=true so the default selection is "Enabled" (idx 0).
        // Enter accepts channels, Enter confirms Enabled, then Enter accepts the
        // default runnable backend offered by onboarding.
        let events = vec![
            key(KeyCode::Enter),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        draft.set_acp_enabled(true);
        let result = runner.run_protocols_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert!(draft.protocols.acp_enabled);
        assert_eq!(draft.protocols.acp_backend.as_deref(), Some("acpx"));
    }

    #[test]
    fn run_step_dispatches_post_boundary_steps_as_next() {
        let events: Vec<Event> = vec![];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();

        let env_check = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(runner.run_step(OnboardWizardStep::EnvironmentCheck, &mut draft));
        assert_eq!(env_check.unwrap(), OnboardFlowStepAction::Next);
    }

    // -----------------------------------------------------------------------
    // Pre/post-flow screen tests
    // -----------------------------------------------------------------------

    #[test]
    fn risk_screen_accepts_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_risk_screen().unwrap());
    }

    #[test]
    fn risk_screen_accepts_on_y() {
        let events = vec![key(KeyCode::Char('y'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_risk_screen().unwrap());
    }

    #[test]
    fn risk_screen_declines_on_n() {
        let events = vec![key(KeyCode::Char('n'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(!runner.run_risk_screen().unwrap());
    }

    #[test]
    fn risk_screen_declines_on_esc() {
        let events = vec![key(KeyCode::Esc)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(!runner.run_risk_screen().unwrap());
    }

    #[test]
    fn confirm_screen_supports_help_and_page_navigation() {
        let events = vec![
            key(KeyCode::Char('?')),
            key(KeyCode::Enter),
            key(KeyCode::PageDown),
            key(KeyCode::Char('G')),
            key(KeyCode::Char('y')),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let body_lines = (0..40)
            .map(|index| Line::from(format!("line {index}")))
            .collect::<Vec<_>>();
        assert!(
            runner
                .run_confirm_screen("long confirm", body_lines, "Enter/y accept  n/Esc decline")
                .unwrap()
        );
    }

    #[test]
    fn entry_choice_screen_selects_default() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let options = vec![
            ("Current".to_owned(), "use existing".to_owned()),
            ("Fresh".to_owned(), "start fresh".to_owned()),
        ];
        let idx = runner
            .run_entry_choice_screen(&options, 0, &["workspace: /tmp/project".to_owned()])
            .unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn entry_choice_screen_selects_second() {
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let options = vec![
            ("Current".to_owned(), "use existing".to_owned()),
            ("Fresh".to_owned(), "start fresh".to_owned()),
        ];
        let idx = runner
            .run_entry_choice_screen(&options, 0, &["workspace: /tmp/project".to_owned()])
            .unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn shortcut_choice_screen_returns_true_for_primary() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_shortcut_choice_screen("Use current setup", &["provider: openai".to_owned()])
            .unwrap();
        assert!(result);
    }

    #[test]
    fn shortcut_choice_screen_returns_false_for_adjust() {
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_shortcut_choice_screen("Use current setup", &["provider: openai".to_owned()])
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn preflight_screen_passes_all_green() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "provider credentials",
            level: crate::onboard_preflight::OnboardCheckLevel::Pass,
            detail: "env binding found".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_preflight_screen(&checks).unwrap());
    }

    #[test]
    fn preflight_screen_with_warning_accepts_on_enter() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "model probe",
            level: crate::onboard_preflight::OnboardCheckLevel::Warn,
            detail: "model not verified".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_preflight_screen(&checks).unwrap());
    }

    #[test]
    fn preflight_screen_with_warning_declines_on_n() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "model probe",
            level: crate::onboard_preflight::OnboardCheckLevel::Warn,
            detail: "model not verified".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Char('n'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(!runner.run_preflight_screen(&checks).unwrap());
    }

    #[test]
    fn preflight_screen_with_failure_returns_false_after_review() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "config validation",
            level: crate::onboard_preflight::OnboardCheckLevel::Fail,
            detail: "provider route probe failed".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(!runner.run_preflight_screen(&checks).unwrap());
    }

    #[test]
    fn review_screen_continues_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        runner
            .run_review_screen(&["provider: openai".to_owned(), "model: gpt-4".to_owned()])
            .unwrap();
    }

    #[test]
    fn review_screen_supports_page_and_edge_navigation() {
        let events = vec![
            key(KeyCode::PageDown),
            key(KeyCode::Char('G')),
            key(KeyCode::PageUp),
            key(KeyCode::Char('g')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let lines = (0..40)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>();
        runner.run_review_screen(&lines).unwrap();
    }

    #[test]
    fn info_screen_supports_help_overlay_before_exit() {
        let events = vec![
            key(KeyCode::Char('?')),
            key(KeyCode::Enter),
            key(KeyCode::Char('G')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let body_lines = (0..30)
            .map(|index| Line::from(format!("item {index}")))
            .collect::<Vec<_>>();
        runner
            .run_info_screen("review deck", body_lines, "Enter to continue")
            .unwrap();
    }

    #[test]
    fn write_confirmation_screen_accepts_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            runner
                .run_write_confirmation_screen("/tmp/loongclaw.toml", false)
                .unwrap()
        );
    }

    #[test]
    fn write_confirmation_screen_declines_on_n() {
        let events = vec![key(KeyCode::Char('n'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            !runner
                .run_write_confirmation_screen("/tmp/loongclaw.toml", false)
                .unwrap()
        );
    }

    #[test]
    fn verify_and_write_screen_accepts_on_enter() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "provider credentials",
            level: crate::onboard_preflight::OnboardCheckLevel::Pass,
            detail: "env binding found".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            runner
                .run_verify_and_write_screen(
                    &checks,
                    &["provider: openai".to_owned(), "model: gpt-5".to_owned()],
                    "/tmp/loongclaw.toml",
                )
                .unwrap()
        );
    }

    #[test]
    fn verify_and_write_screen_help_overlay_closes_before_accept() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "provider credentials",
            level: crate::onboard_preflight::OnboardCheckLevel::Pass,
            detail: "env binding found".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![
            key(KeyCode::Char('?')),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            runner
                .run_verify_and_write_screen(
                    &checks,
                    &["provider: openai".to_owned(), "model: gpt-5".to_owned()],
                    "/tmp/loongclaw.toml",
                )
                .unwrap()
        );
    }

    #[test]
    fn verify_and_write_screen_supports_page_navigation() {
        let checks = (0..12)
            .map(|index| crate::onboard_preflight::OnboardCheck {
                name: if index % 2 == 0 {
                    "provider credentials"
                } else {
                    "provider transport"
                },
                level: crate::onboard_preflight::OnboardCheckLevel::Pass,
                detail: "ok".to_owned(),
                non_interactive_warning_policy:
                    crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
            })
            .collect::<Vec<_>>();
        let review_lines = (0..30)
            .map(|index| format!("review line {index}"))
            .collect::<Vec<_>>();
        let events = vec![
            key(KeyCode::PageDown),
            key(KeyCode::End),
            key(KeyCode::Tab),
            key(KeyCode::PageDown),
            key(KeyCode::Home),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            runner
                .run_verify_and_write_screen(&checks, &review_lines, "/tmp/loongclaw.toml")
                .unwrap()
        );
    }

    #[test]
    fn verify_and_write_screen_with_failure_returns_false_on_enter() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "config validation",
            level: crate::onboard_preflight::OnboardCheckLevel::Fail,
            detail: "provider route probe failed".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            !runner
                .run_verify_and_write_screen(
                    &checks,
                    &["provider: openai".to_owned()],
                    "/tmp/loongclaw.toml"
                )
                .unwrap()
        );
    }

    #[test]
    fn launch_screen_exits_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_launch_screen(&sample_success_summary()).unwrap();
        assert_eq!(result.focused_action, None);
        assert!(result.open_chat);
    }

    #[test]
    fn launch_screen_handles_help_and_scroll_navigation() {
        let events = vec![
            key(KeyCode::Char('?')),
            key(KeyCode::Enter),
            key(KeyCode::Down),
            key(KeyCode::PageDown),
            key(KeyCode::Char('G')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut summary = sample_success_summary();
        summary.next_actions.push(OnboardingAction {
            kind: OnboardingActionKind::Chat,
            label: "interactive chat".to_owned(),
            command: "loong chat".to_owned(),
        });
        let result = runner.run_launch_screen(&summary).unwrap();
        assert_eq!(result.focused_action, None);
        assert!(result.open_chat);
    }

    #[test]
    fn launch_screen_finishes_on_escape() {
        let events = vec![key(KeyCode::Esc)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_launch_screen(&sample_success_summary()).unwrap();
        assert_eq!(result.focused_action, None);
        assert!(!result.open_chat);
    }

    #[test]
    fn launch_screen_blocks_chat_open_for_blocked_outcome() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut summary = sample_success_summary();
        summary.outcome = OnboardOutcome::Blocked;
        let result = runner.run_launch_screen(&summary).unwrap();
        assert_eq!(result.focused_action, None);
        assert!(!result.open_chat);
    }

    #[test]
    fn success_screen_exits_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        runner
            .run_success_screen(&["config written".to_owned()])
            .unwrap();
    }

    #[test]
    fn import_candidate_screen_selects_first() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let candidates = vec![("codex config".to_owned(), "~/.codex/config.json".to_owned())];
        let result = runner
            .run_import_candidate_screen(&candidates, 0, &["provider: openai".to_owned()])
            .unwrap();
        assert_eq!(result, Some(0));
    }

    #[test]
    fn import_candidate_screen_selects_start_fresh() {
        // Navigate past all candidates to the "Start fresh" item
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let candidates = vec![("codex config".to_owned(), "~/.codex/config.json".to_owned())];
        let result = runner
            .run_import_candidate_screen(&candidates, 0, &["provider: openai".to_owned()])
            .unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn entry_choice_screen_supports_showcase_edge_and_numeric_jump() {
        let events = vec![
            key(KeyCode::Char('2')),
            key(KeyCode::Char('g')),
            key(KeyCode::Char('G')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let options = vec![
            ("Current".to_owned(), "use existing".to_owned()),
            ("Import".to_owned(), "reuse detected setup".to_owned()),
            ("Fresh".to_owned(), "start fresh".to_owned()),
        ];
        let idx = runner
            .run_entry_choice_screen(&options, 0, &["workspace: /tmp/project".to_owned()])
            .unwrap();
        assert_eq!(idx, 2);
    }

    // -----------------------------------------------------------------------
    // Integration-level tests: end-to-end guided flow through multiple screens
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "current_thread")]
    async fn guided_flow_completes_with_scripted_events() {
        use crate::onboard_flow::{OnboardFlowController, run_guided_onboard_flow};

        // Event sequence per step:
        //   Welcome:         Enter
        //   Auth sub0-3:     Enter x4  (provider, model, credential, web search)
        //   RuntimeDefaults: Enter x3  (memory, personality, surfaces)
        //   Workspace sub0:  Enter  (sqlite path, accept default)
        //   Workspace sub1:  Enter  (file root, accept default)
        //   Protocols sub0:  Enter  (keep service channels empty)
        //   Protocols sub1:  Enter  (ACP Disabled selected by default)
        let events: Vec<Event> = vec![key(KeyCode::Enter); 13];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let draft = sample_draft();
        let controller = OnboardFlowController::new(draft);
        let controller = run_guided_onboard_flow(controller, &mut runner)
            .await
            .expect("guided flow should complete");

        assert_eq!(
            controller.current_step(),
            OnboardWizardStep::EnvironmentCheck,
            "flow should stop at EnvironmentCheck boundary"
        );
        // Protocols was accepted as Disabled
        assert!(
            !controller.draft().protocols.acp_enabled,
            "ACP should remain disabled when user accepted default"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn guided_flow_with_acp_enabled_completes() {
        use crate::onboard_flow::{OnboardFlowController, run_guided_onboard_flow};

        // Same as above, but start with acp.enabled = true so the Protocols
        // step requires three guided actions (channels + toggle + backend).
        //
        //   Welcome:         Enter
        //   Auth sub0-3:     Enter x4
        //   RuntimeDefaults: Enter x3
        //   Workspace sub0-1:Enter x2
        //   Protocols sub0:  Enter  (keep service channels empty)
        //   Protocols sub1:  Enter  (Enabled, pre-selected)
        //   Protocols sub2:  Enter  (accept the default runnable backend)
        let events: Vec<Event> = vec![key(KeyCode::Enter); 13];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let mut draft = sample_draft();
        draft.set_acp_enabled(true);
        let controller = OnboardFlowController::new(draft);
        let controller = run_guided_onboard_flow(controller, &mut runner)
            .await
            .expect("guided flow with ACP enabled should complete");

        assert_eq!(
            controller.current_step(),
            OnboardWizardStep::EnvironmentCheck
        );
        assert!(controller.draft().protocols.acp_enabled);
        assert_eq!(
            controller.draft().protocols.acp_backend.as_deref(),
            Some("acpx"),
            "guided onboarding should select a registered runnable ACP backend"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn guided_flow_back_from_auth_returns_to_welcome() {
        use crate::onboard_flow::{OnboardFlowController, run_guided_onboard_flow};

        // Press Enter on Welcome, then Esc on Auth sub-step 0 (provider),
        // which makes Auth return Back.  The flow should revisit Welcome,
        // then proceed normally with Enter through all remaining steps.
        let events = vec![
            key(KeyCode::Enter), // Welcome -> Next
            key(KeyCode::Esc),   // Auth sub0 -> Back -> revisit Welcome
            key(KeyCode::Enter), // Welcome (replay) -> Next
            key(KeyCode::Enter), // Auth sub0
            key(KeyCode::Enter), // Auth sub1
            key(KeyCode::Enter), // Auth sub2
            key(KeyCode::Enter), // Auth sub3
            key(KeyCode::Enter), // RuntimeDefaults sub0
            key(KeyCode::Enter), // RuntimeDefaults sub1
            key(KeyCode::Enter), // RuntimeDefaults sub2
            key(KeyCode::Enter), // Workspace sub0
            key(KeyCode::Enter), // Workspace sub1
            key(KeyCode::Enter), // Protocols sub0
            key(KeyCode::Enter), // Protocols sub1 (Disabled)
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let controller = OnboardFlowController::new(sample_draft());
        let controller = run_guided_onboard_flow(controller, &mut runner)
            .await
            .expect("flow with back from auth should complete");

        assert_eq!(
            controller.current_step(),
            OnboardWizardStep::EnvironmentCheck,
            "flow should still reach EnvironmentCheck after back-navigation"
        );
    }

    #[test]
    fn auth_step_back_from_model_returns_to_provider() {
        // Within the Authentication step, navigate into model strategy,
        // press Esc to go back to provider, then proceed through the full flow.
        let events = vec![
            key(KeyCode::Enter), // sub0: provider -> sub1
            key(KeyCode::Esc),   // sub1: model -> back to sub0
            key(KeyCode::Enter), // sub0: provider (replay) -> sub1
            key(KeyCode::Enter), // sub1: model -> sub2
            key(KeyCode::Enter), // sub2: provider credential env -> sub3
            key(KeyCode::Enter), // sub3: web search -> Next
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let result = runner.run_authentication_step(&mut draft).unwrap();
        assert_eq!(
            result,
            OnboardFlowStepAction::Next,
            "auth step should complete successfully after sub-step back-nav"
        );
    }

    #[test]
    fn auth_step_can_switch_provider_before_model_and_credential_entry() {
        let source = ScriptedEventSource::new(Vec::new());
        let runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let provider_options = runner.provider_picker_options(&draft.config.provider);
        let current_route_is_oauth =
            RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                &draft.config.provider,
            );
        let default_provider_index = provider_options
            .iter()
            .position(|option| {
                if option.kind != draft.config.provider.kind {
                    return false;
                }
                let option_route_is_oauth =
                    RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                        &option.preview,
                    );
                option_route_is_oauth == current_route_is_oauth
            })
            .expect("current provider index");
        let anthropic_index = provider_options
            .iter()
            .position(|option| option.kind == mvp::config::ProviderKind::Anthropic)
            .expect("anthropic provider option");
        let current_web_search_provider =
            crate::onboard_web_search::current_web_search_provider(&draft.config);
        let default_web_search_requires_credential = mvp::config::web_search_provider_descriptors()
            .iter()
            .find(|descriptor| descriptor.id == current_web_search_provider)
            .is_some_and(|descriptor| descriptor.requires_api_key);

        let mut events =
            replace_checked_option_from_first_page(default_provider_index, anthropic_index);
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        if default_web_search_requires_credential {
            events.push(key(KeyCode::Enter));
        }

        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_authentication_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(
            draft.config.provider.kind,
            mvp::config::ProviderKind::Anthropic,
            "the first provider move should switch away from the current OpenAI draft"
        );
    }

    #[test]
    fn default_openai_draft_prefers_api_route_instead_of_oauth_route() {
        let draft = sample_draft();

        let prefers_oauth =
            RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                &draft.config.provider,
            );

        assert!(
            !prefers_oauth,
            "plain OpenAI should default to the API-key route unless OAuth was explicitly selected"
        );
    }

    #[test]
    fn openai_codex_oauth_preview_prefers_oauth_route() {
        let draft = sample_draft();
        let oauth_preview =
            RatatuiOnboardRunner::<ScriptedEventSource>::openai_codex_oauth_picker_preview(
                draft.config.provider,
            );

        let prefers_oauth =
            RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                &oauth_preview,
            );

        assert!(
            prefers_oauth,
            "the dedicated OpenAI Codex OAuth option should remain on the OAuth route"
        );
    }

    #[test]
    fn auth_step_can_accept_reviewed_model_and_choose_web_search_provider() {
        let mut draft = sample_draft();
        let source = ScriptedEventSource::new(Vec::new());
        let runner = RatatuiOnboardRunner::headless(source).unwrap();
        let provider_options = runner.provider_picker_options(&draft.config.provider);
        let current_route_is_oauth =
            RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                &draft.config.provider,
            );
        let default_provider_index = provider_options
            .iter()
            .position(|option| {
                if option.kind != draft.config.provider.kind {
                    return false;
                }
                let option_route_is_oauth =
                    RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                        &option.preview,
                    );
                option_route_is_oauth == current_route_is_oauth
            })
            .expect("current provider index");
        let deepseek_index = provider_options
            .iter()
            .position(|option| option.kind == mvp::config::ProviderKind::Deepseek)
            .expect("deepseek provider option");
        let web_search_options =
            RatatuiOnboardRunner::<ScriptedEventSource>::web_search_picker_options(&draft.config);
        let default_web_search_index = web_search_options
            .iter()
            .position(|option| option.id == mvp::config::DEFAULT_WEB_SEARCH_PROVIDER)
            .expect("default web search option");
        let brave_index = web_search_options
            .iter()
            .position(|option| option.id == mvp::config::WEB_SEARCH_PROVIDER_BRAVE)
            .expect("brave web search option");

        let mut events =
            replace_checked_option_from_first_page(default_provider_index, deepseek_index);
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.extend(replace_checked_option_from_first_page(
            default_web_search_index,
            brave_index,
        ));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));

        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let result = runner.run_authentication_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(
            draft.config.provider.kind,
            mvp::config::ProviderKind::Deepseek
        );
        assert_eq!(draft.config.provider.model, "deepseek-chat");
        assert_eq!(
            draft.config.tools.web_search.default_provider,
            mvp::config::WEB_SEARCH_PROVIDER_BRAVE
        );
    }

    #[test]
    fn auth_step_can_select_multiple_web_search_providers_and_choose_a_default() {
        let mut draft = sample_draft();
        let picker_options =
            RatatuiOnboardRunner::<ScriptedEventSource>::web_search_picker_options(&draft.config);
        let default_index = picker_options
            .iter()
            .position(|option| option.id == mvp::config::DEFAULT_WEB_SEARCH_PROVIDER)
            .expect("default web search option");
        let brave_index = picker_options
            .iter()
            .position(|option| option.id == mvp::config::WEB_SEARCH_PROVIDER_BRAVE)
            .expect("brave web search option");
        let tavily_index = picker_options
            .iter()
            .position(|option| option.id == mvp::config::WEB_SEARCH_PROVIDER_TAVILY)
            .expect("tavily web search option");

        let mut events = vec![
            key(KeyCode::Enter),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
        ];
        events.extend(replace_checked_option_from_first_page(
            default_index,
            brave_index,
        ));
        events.extend(move_events_between_indices(default_index, tavily_index));
        events.push(key(KeyCode::Char(' ')));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Down));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));

        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let result = runner.run_authentication_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(
            draft.config.tools.web_search.default_provider,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY
        );
        assert_eq!(
            draft.config.tools.web_search.brave_api_key,
            Some("${BRAVE_API_KEY}".to_owned())
        );
        assert_eq!(
            draft.config.tools.web_search.tavily_api_key,
            Some("${TAVILY_API_KEY}".to_owned())
        );
    }

    #[test]
    fn auth_step_can_keep_multiple_providers_and_choose_active_default() {
        let source = ScriptedEventSource::new(Vec::new());
        let runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let provider_options = runner.provider_picker_options(&draft.config.provider);
        let current_route_is_oauth =
            RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                &draft.config.provider,
            );
        let default_provider_index = provider_options
            .iter()
            .position(|option| {
                if option.kind != draft.config.provider.kind {
                    return false;
                }
                let option_route_is_oauth =
                    RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                        &option.preview,
                    );
                option_route_is_oauth == current_route_is_oauth
            })
            .expect("current provider index");
        let anthropic_index = provider_options
            .iter()
            .position(|option| option.kind == mvp::config::ProviderKind::Anthropic)
            .expect("anthropic provider option");
        let current_web_search_provider =
            crate::onboard_web_search::current_web_search_provider(&draft.config);
        let default_web_search_requires_credential = mvp::config::web_search_provider_descriptors()
            .iter()
            .find(|descriptor| descriptor.id == current_web_search_provider)
            .is_some_and(|descriptor| descriptor.requires_api_key);
        let current_active_index = if default_provider_index < anthropic_index {
            0
        } else {
            1
        };
        let anthropic_active_index = if anthropic_index < default_provider_index {
            0
        } else {
            1
        };

        let mut events = add_checked_option_from_first_page(anthropic_index);
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.extend(move_events_between_indices(
            current_active_index,
            anthropic_active_index,
        ));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        if default_web_search_requires_credential {
            events.push(key(KeyCode::Enter));
        }

        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let result = runner.run_authentication_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(draft.config.active_provider_id(), Some("anthropic"));
        assert_eq!(
            draft.config.provider.kind,
            mvp::config::ProviderKind::Anthropic
        );
        assert!(draft.config.providers.contains_key("anthropic"));
        assert!(draft.config.providers.contains_key("openai"));
    }

    #[test]
    fn auth_step_openai_codex_oauth_route_runs_browser_authorization_and_stores_inline_token() {
        let _guard = openai_codex_oauth_test_guard();
        OPENAI_CODEX_OAUTH_START_CALLS.store(0, Ordering::SeqCst);
        OPENAI_CODEX_OAUTH_BROWSER_CALLS.store(0, Ordering::SeqCst);
        OPENAI_CODEX_OAUTH_MANUAL_CALLS.store(0, Ordering::SeqCst);

        let source = ScriptedEventSource::new(Vec::new());
        let runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let provider_options = runner.provider_picker_options(&draft.config.provider);
        let current_route_is_oauth =
            RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                &draft.config.provider,
            );
        let default_provider_index = provider_options
            .iter()
            .position(|option| {
                if option.kind != draft.config.provider.kind {
                    return false;
                }
                let option_route_is_oauth =
                    RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                        &option.preview,
                    );
                option_route_is_oauth == current_route_is_oauth
            })
            .expect("current provider index");
        let oauth_provider_index = provider_options
            .iter()
            .position(|option| {
                let is_openai = option.kind == mvp::config::ProviderKind::Openai;
                let is_oauth_route =
                    RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                        &option.preview,
                    );
                is_openai && is_oauth_route
            })
            .expect("openai codex oauth provider option");
        let current_web_search_provider =
            crate::onboard_web_search::current_web_search_provider(&draft.config);
        let default_web_search_requires_credential = mvp::config::web_search_provider_descriptors()
            .iter()
            .find(|descriptor| descriptor.id == current_web_search_provider)
            .is_some_and(|descriptor| descriptor.requires_api_key);

        let mut events =
            replace_checked_option_from_first_page(default_provider_index, oauth_provider_index);
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        if default_web_search_requires_credential {
            events.push(key(KeyCode::Enter));
        }

        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        runner.set_openai_codex_oauth_start(fake_openai_codex_oauth_start);

        let result = runner.run_authentication_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(OPENAI_CODEX_OAUTH_START_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(OPENAI_CODEX_OAUTH_BROWSER_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(OPENAI_CODEX_OAUTH_MANUAL_CALLS.load(Ordering::SeqCst), 0);
        assert_eq!(draft.config.provider.api_key, None);
        assert_eq!(draft.config.provider.api_key_env, None);
        assert_eq!(draft.config.provider.oauth_access_token_env, None);
        assert_eq!(
            draft.config.provider.oauth_access_token,
            Some(SecretRef::Inline("oauth-derived-token".to_owned()))
        );
    }

    #[test]
    fn openai_codex_oauth_support_lines_explain_manual_redirect_fallback() {
        let lines = RatatuiOnboardRunner::<ScriptedEventSource>::openai_codex_oauth_support_lines(
            "http://localhost:1455/auth/callback",
            "https://auth.openai.test/oauth/authorize?client_id=test-client",
            None,
        );
        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");

        assert!(
            rendered.contains("paste"),
            "oauth support copy should mention manual paste fallback: {rendered}"
        );
        assert!(
            rendered.contains("redirect URL") || rendered.contains("authorization code"),
            "oauth support copy should mention the redirect URL or authorization code fallback: {rendered}"
        );
    }

    #[test]
    fn auth_step_openai_codex_oauth_route_accepts_manual_paste_fallback() {
        let _guard = openai_codex_oauth_test_guard();
        OPENAI_CODEX_OAUTH_START_CALLS.store(0, Ordering::SeqCst);
        OPENAI_CODEX_OAUTH_BROWSER_CALLS.store(0, Ordering::SeqCst);
        OPENAI_CODEX_OAUTH_MANUAL_CALLS.store(0, Ordering::SeqCst);

        let source = ScriptedEventSource::new(Vec::new());
        let runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let provider_options = runner.provider_picker_options(&draft.config.provider);
        let current_route_is_oauth =
            RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                &draft.config.provider,
            );
        let default_provider_index = provider_options
            .iter()
            .position(|option| {
                if option.kind != draft.config.provider.kind {
                    return false;
                }
                let option_route_is_oauth =
                    RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                        &option.preview,
                    );
                option_route_is_oauth == current_route_is_oauth
            })
            .expect("current provider index");
        let oauth_provider_index = provider_options
            .iter()
            .position(|option| {
                let is_openai = option.kind == mvp::config::ProviderKind::Openai;
                let is_oauth_route =
                    RatatuiOnboardRunner::<ScriptedEventSource>::provider_prefers_oauth_route(
                        &option.preview,
                    );
                is_openai && is_oauth_route
            })
            .expect("openai codex oauth provider option");
        let current_web_search_provider =
            crate::onboard_web_search::current_web_search_provider(&draft.config);
        let default_web_search_requires_credential = mvp::config::web_search_provider_descriptors()
            .iter()
            .find(|descriptor| descriptor.id == current_web_search_provider)
            .is_some_and(|descriptor| descriptor.requires_api_key);

        let mut events =
            replace_checked_option_from_first_page(default_provider_index, oauth_provider_index);
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Char('p')));
        events.push(Event::Paste(
            "http://localhost:1455/auth/callback?code=manual-code-123&state=state-123".to_owned(),
        ));
        events.push(key(KeyCode::Enter));
        events.push(key(KeyCode::Enter));
        if default_web_search_requires_credential {
            events.push(key(KeyCode::Enter));
        }

        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        runner.set_openai_codex_oauth_start(fake_openai_codex_oauth_start_manual_only);

        let result = runner.run_authentication_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(OPENAI_CODEX_OAUTH_START_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(OPENAI_CODEX_OAUTH_BROWSER_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(OPENAI_CODEX_OAUTH_MANUAL_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(
            draft.config.provider.oauth_access_token,
            Some(SecretRef::Inline("oauth-derived-token".to_owned()))
        );
    }

    #[test]
    fn runtime_defaults_step_can_toggle_cli_and_external_skills_surfaces() {
        let events = vec![
            key(KeyCode::Enter),
            key(KeyCode::Enter),
            key(KeyCode::Char(' ')),
            key(KeyCode::Down),
            key(KeyCode::Char(' ')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();

        let result = runner.run_runtime_defaults_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert!(!draft.config.cli.enabled);
        assert!(draft.config.external_skills.enabled);
        assert!(draft.config.external_skills.require_download_approval);
        assert!(!draft.config.external_skills.auto_expose_installed);
    }

    #[test]
    fn protocols_step_can_select_channels_before_protocol_choice() {
        let mut draft = sample_draft();
        let entries =
            RatatuiOnboardRunner::<ScriptedEventSource>::sorted_service_channel_catalog_entries(
                &draft.config,
            );
        let selected_entry = entries.first().expect("at least one service channel");
        let selected_channel_id = selected_entry.id.to_owned();
        let pairing_prompts = RatatuiOnboardRunner::<ScriptedEventSource>::channel_pairing_prompts(
            &draft.config,
            selected_entry,
        );
        let mut events = vec![key(KeyCode::Char(' ')), key(KeyCode::Enter)];
        for _ in pairing_prompts {
            events.push(key(KeyCode::Enter));
        }
        events.push(key(KeyCode::Enter));
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let result = runner.run_protocols_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(
            draft.config.enabled_service_channel_ids(),
            vec![selected_channel_id]
        );
        assert!(!draft.protocols.acp_enabled);
    }

    #[test]
    fn protocols_step_offers_all_registered_channel_surfaces_sorted_by_label() {
        let draft = sample_draft();
        let entries =
            RatatuiOnboardRunner::<ScriptedEventSource>::sorted_service_channel_catalog_entries(
                &draft.config,
            );
        let actual_ids = entries.iter().map(|entry| entry.id).collect::<Vec<_>>();
        let actual_labels = entries
            .iter()
            .map(|entry| entry.label.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let mut sorted_labels = actual_labels.clone();
        sorted_labels.sort();

        assert!(
            actual_ids.len() > 10,
            "protocols step should surface the broader channel catalog instead of a tiny curated subset: {actual_ids:#?}"
        );
        assert!(
            actual_ids.contains(&"matrix"),
            "matrix should remain available in the onboarding channel picker: {actual_ids:#?}"
        );
        assert!(
            actual_ids.contains(&"wecom"),
            "wecom should remain available in the onboarding channel picker: {actual_ids:#?}"
        );
        assert!(
            actual_ids.contains(&"slack"),
            "slack should remain available in the onboarding channel picker: {actual_ids:#?}"
        );
        assert_eq!(
            actual_labels, sorted_labels,
            "channel picker entries should stay alphabetically sorted by label"
        );
    }

    #[test]
    fn protocols_step_sorts_channels_and_collects_pairing_input_for_selected_channel() {
        let mut draft = sample_draft();
        let entries =
            RatatuiOnboardRunner::<ScriptedEventSource>::sorted_service_channel_catalog_entries(
                &draft.config,
            );
        let telegram_index = entries
            .iter()
            .position(|entry| entry.id == "telegram")
            .expect("telegram channel should be available");
        let telegram_entry = &entries[telegram_index];
        let pairing_prompts = RatatuiOnboardRunner::<ScriptedEventSource>::channel_pairing_prompts(
            &draft.config,
            telegram_entry,
        );

        let mut events = down_events_to_index(telegram_index);
        events.push(key(KeyCode::Char(' ')));
        events.push(key(KeyCode::Enter));
        for prompt in pairing_prompts {
            if prompt.field_key == "telegram.bot_token_env" {
                for character in "LC_TELEGRAM_TOKEN".chars() {
                    events.push(key(KeyCode::Char(character)));
                }
            }
            events.push(key(KeyCode::Enter));
        }
        events.push(key(KeyCode::Enter));

        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let result = runner.run_protocols_step(&mut draft).unwrap();

        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(
            draft.config.enabled_service_channel_ids(),
            vec!["telegram".to_owned()]
        );
        assert_eq!(
            draft.config.telegram.bot_token_env.as_deref(),
            Some("LC_TELEGRAM_TOKEN")
        );
    }

    #[test]
    fn ctrl_c_on_welcome_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let err = runner.run_welcome_step().unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_auth_returns_interrupted_error() {
        // Ctrl-C on provider selection sub-step
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_authentication_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_runtime_defaults_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_runtime_defaults_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_workspace_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_workspace_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_protocols_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_protocols_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_risk_screen_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let err = runner.run_risk_screen().unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_entry_choice_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let options = vec![("A".to_owned(), "option a".to_owned())];
        let err = runner
            .run_entry_choice_screen(&options, 0, &["workspace: /tmp/project".to_owned()])
            .unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn selection_card_theme_for_step_stays_stable_across_stage() {
        let palette = RatatuiOnboardRunner::<ScriptedEventSource>::palette();
        let auth = RatatuiOnboardRunner::<ScriptedEventSource>::selection_card_theme_for_step(
            OnboardWizardStep::Authentication,
        );
        let runtime = RatatuiOnboardRunner::<ScriptedEventSource>::selection_card_theme_for_step(
            OnboardWizardStep::RuntimeDefaults,
        );
        let workspace = RatatuiOnboardRunner::<ScriptedEventSource>::selection_card_theme_for_step(
            OnboardWizardStep::Workspace,
        );

        assert_eq!(auth.frame_color, palette.brand);
        assert_eq!(runtime.frame_color, palette.brand);
        assert_eq!(workspace.frame_color, palette.brand);
        assert_eq!(auth.active_bg, palette.surface_emphasis);
        assert_eq!(auth, runtime);
        assert_eq!(runtime, workspace);
    }

    #[test]
    fn launch_action_card_theme_uses_handoff_palette() {
        let palette = RatatuiOnboardRunner::<ScriptedEventSource>::palette();
        let theme = RatatuiOnboardRunner::<ScriptedEventSource>::launch_action_card_theme();
        assert_eq!(theme.frame_color, palette.brand);
        assert_eq!(theme.accent_color, palette.brand);
        assert_eq!(theme.active_bg, palette.surface_emphasis);
    }

    #[test]
    fn resize_events_are_handled_gracefully() {
        // Inject a resize event before the Enter that completes the welcome step.
        // The resize should be silently consumed (triggering a redraw) without panic.
        let events = vec![Event::Resize(120, 40), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert_eq!(
            result.unwrap(),
            OnboardFlowStepAction::Next,
            "resize events should be consumed gracefully"
        );
    }

    #[test]
    fn resize_during_selection_loop_is_handled() {
        let events = vec![
            Event::Resize(100, 50),
            key(KeyCode::Down),
            Event::Resize(80, 24),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(
            matches!(result, SelectionLoopResult::Selected(1)),
            "selection should complete normally despite resize events"
        );
    }

    #[test]
    fn selection_card_state_with_zero_items_does_not_panic() {
        // SelectionCardState with zero items: next/previous should be no-ops.
        let mut state = SelectionCardState::new(0);
        state.next();
        state.previous();
        assert_eq!(state.selected(), 0, "selected index should remain 0");

        // SelectionCardWidget with empty items renders without panic.
        let widget = SelectionCardWidget::new(vec![]);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &mut state);
    }
}
