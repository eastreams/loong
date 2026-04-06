use std::borrow::Cow;

use chrono::Datelike;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap, block::Position as TitlePosition,
    },
};

use crate::session::presentation::{SessionPresentationLocale, localized_root_thread_label};

use super::commands;
use super::dialog::ClarifyDialog;
use super::focus::{FocusLayer, FocusStack};
use super::history::{self, PaneView};
use super::input::{self, InputView};
use super::layout;
use super::message::{ToolStatus, format_tool_args_preview};
use super::spinner::{self, SpinnerView};
use super::state;
use super::stats;
use super::status_bar::{self, StatusBarView};
use super::theme::Palette;

#[derive(Debug, Clone, Copy)]
pub(super) struct ToolInspectorView<'a> {
    pub(super) tool_id: &'a str,
    pub(super) tool_name: &'a str,
    pub(super) args_preview: &'a str,
    pub(super) status: &'a ToolStatus,
    pub(super) scroll_offset: u16,
    pub(super) position: usize,
    pub(super) total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SlashPaletteEntry {
    pub(super) replacement: String,
    pub(super) label: String,
    pub(super) meta: String,
    pub(super) detail: String,
    pub(super) immediate: bool,
    pub(super) submit_on_select: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StatsOverlayView<'a> {
    pub(super) snapshot: &'a stats::StatsSnapshot,
    pub(super) active_tab: stats::StatsTab,
    pub(super) date_range: stats::StatsDateRange,
    pub(super) list_scroll_offset: usize,
    pub(super) copy_status: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SessionPickerView<'a> {
    pub(super) picker: &'a state::SessionPickerState,
    pub(super) current_session_id: &'a str,
}

// ---------------------------------------------------------------------------
// Composite view trait for Shell
// ---------------------------------------------------------------------------

/// Unifies all the sub-view traits so `render::draw` can accept a single
/// state reference.  The consumer (typically `shell.rs`) implements this on
/// the concrete `Shell` type.
pub(super) trait ShellView {
    type Pane: PaneView + SpinnerView + StatusBarView + InputView;

    fn pane(&self) -> &Self::Pane;
    fn show_thinking(&self) -> bool;
    fn focus(&self) -> &FocusStack;
    fn clarify_dialog(&self) -> Option<&ClarifyDialog>;
    fn tool_inspector(&self) -> Option<ToolInspectorView<'_>>;
    fn stats_overlay(&self) -> Option<StatsOverlayView<'_>>;
    fn session_picker(&self) -> Option<SessionPickerView<'_>>;
    fn slash_command_selection(&self) -> usize;
    fn slash_palette_entries(&self, draft_prefix: &str) -> Vec<SlashPaletteEntry>;
}

// ---------------------------------------------------------------------------
// Top-level draw dispatcher
// ---------------------------------------------------------------------------

pub(super) fn draw(
    frame: &mut Frame<'_>,
    state: &impl ShellView,
    textarea: &tui_textarea::TextArea<'_>,
    palette: &Palette,
) {
    let area = frame.area();
    if should_use_compact_shell(area) {
        render_compact_shell(frame, state, palette);
        return;
    }

    let input_height = input::preferred_input_height(textarea, area.width);
    let areas = resolve_shell_areas(frame, state, textarea, input_height);

    // 1. History (message transcript)
    history::render_history(
        frame,
        areas.history,
        state.pane(),
        palette,
        state.show_thinking(),
        state.focus().has(FocusLayer::Transcript),
    );

    // 2. First separator
    render_separator(frame, areas.separator1, palette);

    // 3. Spinner / phase line
    spinner::render_spinner(frame, areas.spinner, state.pane(), palette);

    // 4. Second separator
    render_separator(frame, areas.separator2, palette);

    // 5. Input area
    input::render_input(
        frame,
        areas.input,
        textarea,
        state.pane(),
        state.focus().top(),
        palette,
    );

    render_command_palette(
        frame,
        state,
        areas.input,
        textarea,
        state.focus().top(),
        state.slash_command_selection(),
        palette,
    );

    // 6. Status bar
    status_bar::render_status_bar(
        frame,
        areas.status_bar,
        state.pane(),
        state.focus().top(),
        palette,
    );

    // 7. Overlays
    if let Some(dialog) = state.clarify_dialog()
        && state.focus().has(FocusLayer::ClarifyDialog)
    {
        render_clarify_dialog(dialog, frame, area, palette);
    }

    if let Some(tool_inspector) = state.tool_inspector()
        && state.focus().has(FocusLayer::ToolInspector)
    {
        render_tool_inspector(tool_inspector, frame, area, palette);
    }

    if let Some(stats_overlay) = state.stats_overlay()
        && state.focus().has(FocusLayer::StatsOverlay)
    {
        render_stats_overlay(stats_overlay, frame, area, palette);
    }

    if let Some(session_picker) = state.session_picker()
        && state.focus().has(FocusLayer::SessionPicker)
    {
        render_session_picker(session_picker, frame, area, palette);
    }

    if state.focus().has(FocusLayer::Help) {
        render_help_overlay(frame, area, palette);
    }
}

fn resolve_shell_areas(
    frame: &Frame<'_>,
    state: &impl ShellView,
    textarea: &tui_textarea::TextArea<'_>,
    input_height: u16,
) -> layout::ShellAreas {
    let area = frame.area();
    let maybe_intro = intro_layout_config(state, textarea, area);
    let Some(intro) = maybe_intro else {
        return layout::compute(area, input_height);
    };

    layout::compute_intro(area, input_height, intro)
}

fn intro_layout_config(
    state: &impl ShellView,
    textarea: &tui_textarea::TextArea<'_>,
    area: Rect,
) -> Option<layout::IntroLayoutConfig> {
    if area.height < 18 {
        return None;
    }

    let pane = state.pane();
    if PaneView::scroll_offset(pane) > 0 {
        return None;
    }
    if InputView::agent_running(pane) {
        return None;
    }
    if state.focus().top() != FocusLayer::Composer {
        return None;
    }

    let transcript_width = usize::from(area.width.saturating_sub(4).max(1));
    let transcript_lines =
        history::transcript_plain_lines(pane, transcript_width, state.show_thinking());
    let transcript_line_count = transcript_lines.len();
    if transcript_line_count > 8 {
        return None;
    }
    if !PaneView::messages(pane).is_empty() {
        return None;
    }

    let input_height = input::preferred_input_height(textarea, area.width);
    let clamped_input_height = input_height.clamp(3, 12);
    let minimum_history_height = 5_u16;
    let history_height = u16::try_from(transcript_line_count)
        .ok()
        .map(|count| count.saturating_add(1))
        .unwrap_or(minimum_history_height)
        .max(minimum_history_height);
    let reserved_height = history_height
        .saturating_add(clamped_input_height)
        .saturating_add(4);
    let remaining_height = area.height.saturating_sub(reserved_height);
    if remaining_height < 4 {
        return None;
    }

    let top_padding = remaining_height / 3;
    let intro = layout::IntroLayoutConfig {
        top_padding,
        history_height,
    };

    Some(intro)
}

const COMPACT_CHAT_MIN_WIDTH: u16 = 32;
const COMPACT_CHAT_MIN_HEIGHT: u16 = 10;

fn should_use_compact_shell(area: Rect) -> bool {
    area.width < COMPACT_CHAT_MIN_WIDTH || area.height < COMPACT_CHAT_MIN_HEIGHT
}

fn render_compact_shell(frame: &mut Frame<'_>, state: &impl ShellView, palette: &Palette) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let pane = state.pane();
    let scroll_offset = PaneView::scroll_offset(pane);
    let transcript_width = usize::from(area.width.saturating_sub(2).max(1));
    let transcript_lines =
        history::transcript_plain_lines(pane, transcript_width, state.show_thinking());
    let visible_body_lines = usize::from(area.height.saturating_sub(2).max(1));
    let show_compact_welcome = pane.messages().is_empty()
        || (pane.messages().len() == 1
            && pane
                .messages()
                .first()
                .is_some_and(|message| matches!(message.role, super::message::Role::User)));

    let body_lines = if show_compact_welcome {
        vec!["Type a message to begin.".to_owned()]
    } else if transcript_lines.is_empty() {
        vec!["Ready for chat.".to_owned()]
    } else {
        let start_index = transcript_lines.len().saturating_sub(visible_body_lines);
        transcript_lines
            .into_iter()
            .skip(start_index)
            .collect::<Vec<_>>()
    };

    let header_line = Line::from(vec![
        Span::styled(
            " LOONGCLAW ",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            compact_focus_label(state.focus().top()),
            Style::default().fg(palette.info),
        ),
        Span::styled(" | ", Style::default().fg(palette.separator)),
        Span::styled(
            compact_scroll_label(scroll_offset),
            Style::default().fg(if scroll_offset == 0 {
                palette.success
            } else {
                palette.warning
            }),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(header_line),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let footer_hint = pane
        .input_hint()
        .unwrap_or("Keep resizing for the full layout.");
    let footer_line = Line::styled(footer_hint.to_owned(), Style::default().fg(palette.dim));
    let footer_area = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    frame.render_widget(Paragraph::new(footer_line), footer_area);

    let body_height = area.height.saturating_sub(2);
    if body_height == 0 {
        return;
    }

    let body_area = Rect::new(area.x, area.y + 1, area.width, body_height);
    let body_widget = Paragraph::new(body_lines.join("\n")).wrap(Wrap { trim: false });
    frame.render_widget(body_widget, body_area);
}

fn compact_focus_label(focus: FocusLayer) -> &'static str {
    match focus {
        FocusLayer::Composer => "COMPOSE",
        FocusLayer::Transcript => "REVIEW",
        FocusLayer::Help => "HELP",
        FocusLayer::SessionPicker => "PICKER",
        FocusLayer::StatsOverlay => "STATS",
        FocusLayer::ToolInspector => "TOOL",
        FocusLayer::ClarifyDialog => "QUESTION",
    }
}

fn compact_scroll_label(scroll_offset: u16) -> &'static str {
    if scroll_offset == 0 {
        "LIVE"
    } else {
        "SCROLLED"
    }
}

fn render_stats_overlay(
    stats_overlay: StatsOverlayView<'_>,
    frame: &mut Frame<'_>,
    area: Rect,
    palette: &Palette,
) {
    if area.width < 60 || area.height < 18 {
        return;
    }

    let max_width = area.width.saturating_sub(4);
    let preferred_width = area.width.saturating_mul(4) / 5;
    let popup_width = preferred_width.max(72).min(max_width);

    let max_height = area.height.saturating_sub(2);
    let preferred_height = area.height.saturating_mul(4) / 5;
    let popup_height = preferred_height.max(18).min(max_height);

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let footer_hint = match stats_overlay.copy_status {
        Some(status) => {
            format!(" Esc close · Tab switch · r cycle range · Ctrl+S copy · {status} ")
        }
        None => " Esc close · Tab switch · r cycle range · Ctrl+S copy ".to_owned(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.separator))
        .title(Span::styled(
            " Stats ",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ))
        .title_position(TitlePosition::Top)
        .title(Span::styled(
            footer_hint,
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ))
        .title_position(TitlePosition::Bottom)
        .style(Style::default().bg(palette.surface));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(8),
        ])
        .split(inner);

    let [tabs_area, date_range_area, body_area] = sections.as_ref() else {
        return;
    };

    render_stats_tab_row(frame, *tabs_area, stats_overlay, palette);
    render_stats_date_range_row(frame, *date_range_area, stats_overlay, palette);

    match stats_overlay.active_tab {
        stats::StatsTab::Overview => {
            render_stats_overview_body(frame, *body_area, stats_overlay, palette);
        }
        stats::StatsTab::Models => {
            render_stats_models_body(frame, *body_area, stats_overlay, palette);
        }
        stats::StatsTab::Sessions => {
            render_stats_sessions_body(frame, *body_area, stats_overlay, palette);
        }
    }
}

fn render_session_picker(
    session_picker: SessionPickerView<'_>,
    frame: &mut Frame<'_>,
    area: Rect,
    palette: &Palette,
) {
    if area.width < 56 || area.height < 14 {
        return;
    }

    let max_width = area.width.saturating_sub(4);
    let preferred_width = area.width.saturating_mul(3) / 4;
    let popup_width = preferred_width.max(56).min(max_width);

    let max_height = area.height.saturating_sub(2);
    let preferred_height = area.height.saturating_mul(3) / 4;
    let popup_height = preferred_height.max(14).min(max_height);

    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.separator))
        .title(Span::styled(
            format!(" {} ", session_picker.picker.mode.title()),
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ))
        .title_position(TitlePosition::Top)
        .title(Span::styled(
            session_picker.picker.mode.footer_hint(),
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ))
        .title_position(TitlePosition::Bottom)
        .style(Style::default().bg(palette.surface));

    let inner_area = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(6)])
        .split(inner_area);
    let [search_area, list_area] = sections.as_ref() else {
        return;
    };

    let query = session_picker.picker.query.trim();
    let query_text = if query.is_empty() {
        " Search…".to_owned()
    } else {
        format!(" Search: {query}")
    };
    let query_style = if query.is_empty() {
        Style::default().fg(palette.dim)
    } else {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    };
    let query_line = Line::from(Span::styled(query_text, query_style));
    frame.render_widget(Paragraph::new(vec![query_line]), *search_area);

    let filtered_indices = session_picker.picker.filtered_indices();
    let visible_rows = usize::from(list_area.height.max(1));
    let scroll_offset = session_picker.picker.list_scroll_offset;
    let visible_indices = filtered_indices
        .into_iter()
        .skip(scroll_offset)
        .take(visible_rows)
        .collect::<Vec<_>>();

    if visible_indices.is_empty() {
        let empty_text = Paragraph::new(vec![Line::from(Span::styled(
            session_picker.picker.mode.empty_message(),
            Style::default().fg(palette.dim),
        ))]);
        frame.render_widget(empty_text, *list_area);
        return;
    }

    let picker_mode = session_picker.picker.mode;
    let mut lines = Vec::new();
    let locale = SessionPresentationLocale::detect_from_env();
    for (visible_row, session_index) in visible_indices.into_iter().enumerate() {
        let Some(session) = session_picker.picker.sessions.get(session_index) else {
            continue;
        };
        let absolute_index = scroll_offset.saturating_add(visible_row);
        let is_selected = absolute_index == session_picker.picker.selected_index;
        let is_current = session.session_id == session_picker.current_session_id;

        let primary = session_picker_primary_label(session, picker_mode, locale);
        let mut detail_parts = vec![session_picker_detail_label(session)];
        let maybe_provider_label = session
            .agent_presentation
            .as_ref()
            .and_then(|presentation| presentation.provider_label(locale));
        if let Some(provider_label) = maybe_provider_label {
            detail_parts.push(provider_label);
        }
        if picker_mode == state::SessionPickerMode::Subagents
            && session.kind == "root"
            && session.label.is_some()
        {
            detail_parts.push(localized_root_thread_label(locale).to_owned());
        }
        if let Some(label) = session.label.as_deref() {
            let label_text = label.to_owned();
            if primary != label_text {
                detail_parts.push(label_text);
            }
        }
        if session.attention_approval_count > 0 {
            detail_parts.push(format!("APR! {}", session.attention_approval_count));
        }
        let remaining_pending = session
            .pending_approval_count
            .saturating_sub(session.attention_approval_count);
        if remaining_pending > 0 {
            detail_parts.push(format!("APR {remaining_pending}"));
        }
        if is_current {
            detail_parts.push("current".to_owned());
        }
        let detail = detail_parts.join(" · ");

        let marker = if is_selected { "›" } else { " " };
        let accent_color = session
            .agent_presentation
            .as_ref()
            .map(|presentation| palette.subagent_accent(presentation.persona_id.as_str()))
            .unwrap_or_else(|| {
                if session.kind == "root" {
                    palette.brand
                } else {
                    palette.text
                }
            });
        let primary_style = Style::default()
            .fg(accent_color)
            .add_modifier(Modifier::BOLD);
        let detail_style = if is_selected {
            Style::default().fg(palette.info)
        } else {
            Style::default().fg(palette.dim)
        };

        let line = Line::from(vec![
            Span::styled(format!("{marker} "), Style::default().fg(palette.brand)),
            Span::styled(primary, primary_style),
            Span::styled("  ", Style::default().fg(palette.separator)),
            Span::styled(detail, detail_style),
        ]);
        lines.push(line);
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, *list_area);
}

fn session_picker_primary_label(
    session: &state::VisibleSessionSuggestion,
    picker_mode: state::SessionPickerMode,
    locale: SessionPresentationLocale,
) -> String {
    if picker_mode == state::SessionPickerMode::Subagents && session.kind == "root" {
        if let Some(label) = session.label.as_deref() {
            let trimmed_label = label.trim();
            if !trimmed_label.is_empty() {
                return trimmed_label.to_owned();
            }
        }
        return localized_root_thread_label(locale).to_owned();
    }

    if let Some(presentation) = session.agent_presentation.as_ref() {
        return presentation.primary_label(locale);
    }

    session
        .label
        .clone()
        .unwrap_or_else(|| session.session_id.clone())
}

fn session_picker_detail_label(session: &state::VisibleSessionSuggestion) -> String {
    let status_label = session
        .task_phase
        .as_deref()
        .unwrap_or(session.state.as_str());
    let kind_label = match session.kind.as_str() {
        "root" => "thread",
        "delegate_child" => "subagent",
        _ => session.kind.as_str(),
    };

    format!("{status_label} · {kind_label}")
}

fn render_stats_tab_row(
    frame: &mut Frame<'_>,
    area: Rect,
    stats_overlay: StatsOverlayView<'_>,
    palette: &Palette,
) {
    let tabs = [
        stats::StatsTab::Overview,
        stats::StatsTab::Models,
        stats::StatsTab::Sessions,
    ];
    let mut spans = Vec::new();

    for (index, tab) in tabs.iter().enumerate() {
        let is_active = *tab == stats_overlay.active_tab;
        let label = tab.label();
        let style = if is_active {
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(palette.dim)
        };
        spans.push(Span::styled(format!(" {label} "), style));

        if index + 1 < tabs.len() {
            spans.push(Span::styled("  ", Style::default().fg(palette.dim)));
        }
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_stats_date_range_row(
    frame: &mut Frame<'_>,
    area: Rect,
    stats_overlay: StatsOverlayView<'_>,
    palette: &Palette,
) {
    let ranges = [
        stats::StatsDateRange::All,
        stats::StatsDateRange::Last7Days,
        stats::StatsDateRange::Last30Days,
    ];
    let mut spans = Vec::new();

    for (index, date_range) in ranges.iter().enumerate() {
        let is_active = *date_range == stats_overlay.date_range;
        let label = date_range.label();
        let style = if is_active {
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim)
        };
        spans.push(Span::styled(label.to_owned(), style));

        if index + 1 < ranges.len() {
            spans.push(Span::styled(" · ", Style::default().fg(palette.dim)));
        }
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_stats_overview_body(
    frame: &mut Frame<'_>,
    area: Rect,
    stats_overlay: StatsOverlayView<'_>,
    palette: &Palette,
) {
    let range_view = stats_overlay.snapshot.range_view(stats_overlay.date_range);
    let total_tokens_label = stats::format_compact_tokens(range_view.total_tokens);
    let total_input_label = stats::format_compact_tokens(range_view.total_input_tokens);
    let total_output_label = stats::format_compact_tokens(range_view.total_output_tokens);
    let usage_event_label = stats_overlay.snapshot.usage_event_count.to_string();
    let top_model_label = range_view
        .top_model
        .as_ref()
        .map(|entry| entry.model.clone())
        .unwrap_or_else(|| "(none)".to_owned());
    let longest_session_label = stats_overlay
        .snapshot
        .longest_session
        .as_ref()
        .map(render_stats_duration)
        .unwrap_or_else(|| "(none)".to_owned());
    let first_activity_label = stats_overlay
        .snapshot
        .first_activity_date
        .map(stats::short_date_label)
        .unwrap_or_else(|| "(none)".to_owned());
    let last_activity_label = stats_overlay
        .snapshot
        .last_activity_date
        .map(stats::short_date_label)
        .unwrap_or_else(|| "(none)".to_owned());
    let peak_day_label = range_view
        .daily_points
        .iter()
        .max_by_key(|point| point.total_tokens)
        .map(|point| stats::short_date_label(point.date))
        .unwrap_or_else(|| "(none)".to_owned());

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(6)])
        .split(area);
    let [heatmap_area, metrics_area] = sections.as_ref() else {
        return;
    };

    render_stats_activity_heatmap(frame, *heatmap_area, &range_view, palette);

    let left_lines = vec![
        stats_metric_line(
            "Visible sessions",
            stats_overlay.snapshot.visible_sessions.to_string(),
            palette,
        ),
        stats_metric_line(
            "Delegate sessions",
            stats_overlay.snapshot.delegate_sessions.to_string(),
            palette,
        ),
        stats_metric_line(
            "Pending approvals",
            stats_overlay.snapshot.pending_approvals.to_string(),
            palette,
        ),
        stats_metric_line(
            "Running tasks",
            stats_overlay.snapshot.running_delegate_sessions.to_string(),
            palette,
        ),
        stats_metric_line("Active days", range_view.active_days.to_string(), palette),
        stats_metric_line(
            "Current streak",
            range_view.current_streak.to_string(),
            palette,
        ),
        stats_metric_line("Usage events", usage_event_label, palette),
    ];
    let right_lines = vec![
        stats_metric_line("Total tokens", total_tokens_label, palette),
        stats_metric_line("Input tokens", total_input_label, palette),
        stats_metric_line("Output tokens", total_output_label, palette),
        stats_metric_line("Top model", top_model_label, palette),
        stats_metric_line("Peak day", peak_day_label, palette),
        stats_metric_line("Longest session", longest_session_label, palette),
        stats_metric_line(
            "Activity window",
            format!("{first_activity_label} → {last_activity_label}"),
            palette,
        ),
    ];

    let body_sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(*metrics_area);
    let [left_area, right_area] = body_sections.as_ref() else {
        return;
    };

    frame.render_widget(Paragraph::new(left_lines), *left_area);
    frame.render_widget(Paragraph::new(right_lines), *right_area);

    if stats_overlay.snapshot.usage_event_count == 0 {
        let note_line = Line::from(vec![
            Span::styled(
                " No persisted provider usage events yet. ",
                Style::default()
                    .fg(palette.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "New turns will populate the models view.",
                Style::default().fg(palette.dim),
            ),
        ]);
        let note_area = Rect::new(
            metrics_area.x,
            metrics_area.y + metrics_area.height.saturating_sub(1),
            metrics_area.width,
            1,
        );
        frame.render_widget(Paragraph::new(note_line), note_area);
    }
}

fn render_stats_activity_heatmap(
    frame: &mut Frame<'_>,
    area: Rect,
    range_view: &stats::StatsRangeView,
    palette: &Palette,
) {
    if area.width < 18 || area.height < 6 {
        return;
    }

    let heatmap_start_date = range_view
        .daily_points
        .last()
        .map(|point| point.date)
        .unwrap_or_else(|| chrono::Utc::now().date_naive());
    let heatmap_start_date = heatmap_start_date - chrono::Duration::days(34);
    let heatmap_start_offset = i64::from(heatmap_start_date.weekday().num_days_from_monday());
    let grid_start_date = heatmap_start_date - chrono::Duration::days(heatmap_start_offset);
    let grid_end_date = range_view
        .daily_points
        .last()
        .map(|point| point.date)
        .unwrap_or_else(|| chrono::Utc::now().date_naive());
    let max_tokens = range_view
        .daily_points
        .iter()
        .map(|point| point.total_tokens)
        .max()
        .unwrap_or(0);
    let mut by_date = std::collections::BTreeMap::new();

    for point in &range_view.daily_points {
        by_date.insert(point.date, point.total_tokens);
    }

    let mut lines = Vec::new();
    let title = Line::styled(
        " Recent activity",
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD),
    );
    lines.push(title);

    let weekday_labels = ["M", "T", "W", "T", "F", "S", "S"];

    for (weekday_index, weekday_label) in weekday_labels.iter().enumerate() {
        let mut spans = Vec::new();
        let label_text = format!(" {weekday_label} ");
        let label_span = Span::styled(label_text, Style::default().fg(palette.dim));
        spans.push(label_span);

        let mut cell_date = grid_start_date + chrono::Duration::days(weekday_index as i64);
        while cell_date <= grid_end_date {
            let token_count = by_date.get(&cell_date).copied().unwrap_or(0);
            let cell_char = stats_heatmap_char(token_count, max_tokens);
            let cell_color = stats_heatmap_color(token_count, max_tokens, palette);
            let cell_span = Span::styled(cell_char.to_string(), Style::default().fg(cell_color));
            spans.push(cell_span);
            spans.push(Span::raw(" "));
            cell_date += chrono::Duration::days(7);
        }

        let line = Line::from(spans);
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn stats_heatmap_char(token_count: u64, max_tokens: u64) -> char {
    if token_count == 0 || max_tokens == 0 {
        return '·';
    }

    let ratio = token_count as f64 / max_tokens as f64;
    if ratio >= 0.75 {
        return '█';
    }
    if ratio >= 0.5 {
        return '▓';
    }
    if ratio >= 0.25 {
        return '▒';
    }
    '░'
}

fn stats_heatmap_color(token_count: u64, max_tokens: u64, palette: &Palette) -> Color {
    if token_count == 0 || max_tokens == 0 {
        return palette.dim;
    }

    let ratio = token_count as f64 / max_tokens as f64;
    if ratio >= 0.75 {
        return palette.brand;
    }
    if ratio >= 0.5 {
        return palette.warning;
    }
    if ratio >= 0.25 {
        return palette.success;
    }
    palette.info
}

fn render_stats_models_body(
    frame: &mut Frame<'_>,
    area: Rect,
    stats_overlay: StatsOverlayView<'_>,
    palette: &Palette,
) {
    let range_view = stats_overlay.snapshot.range_view(stats_overlay.date_range);
    let total_models = range_view.model_totals.len();
    let visible_count = 6_usize;
    let max_offset = total_models.saturating_sub(visible_count);
    let scroll_offset = stats_overlay.list_scroll_offset.min(max_offset);
    let body_sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(10),
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);
    let [title_area, chart_area, legend_area, list_area, hint_area] = body_sections.as_ref() else {
        return;
    };

    let title = Line::styled(
        " Tokens per Day",
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(Paragraph::new(title), *title_area);

    let maybe_chart = range_view.chart_view(3);
    if let Some(chart_view) = maybe_chart {
        render_stats_chart(frame, *chart_area, &chart_view, palette);
        render_stats_chart_legend(frame, *legend_area, &chart_view, palette);
    } else {
        let empty_line = Line::styled(
            "No persisted model usage yet.",
            Style::default().fg(palette.dim),
        );
        frame.render_widget(Paragraph::new(empty_line), *chart_area);
    }

    let model_lines = range_view
        .model_totals
        .iter()
        .skip(scroll_offset)
        .take(6)
        .map(|entry| render_stats_model_line(entry, palette))
        .collect::<Vec<_>>();

    let content = if model_lines.is_empty() {
        vec![Line::styled(
            "No model totals available for this range.",
            Style::default().fg(palette.dim),
        )]
    } else {
        model_lines
    };

    frame.render_widget(Paragraph::new(content), *list_area);

    let hint_line = if total_models > visible_count {
        let start = scroll_offset.saturating_add(1);
        let end = (scroll_offset + visible_count).min(total_models);
        Line::styled(
            format!(" ↑↓ scroll models · showing {start}-{end} of {total_models} "),
            Style::default().fg(palette.dim),
        )
    } else {
        Line::styled(" top models ".to_owned(), Style::default().fg(palette.dim))
    };
    frame.render_widget(Paragraph::new(hint_line), *hint_area);
}

fn render_stats_sessions_body(
    frame: &mut Frame<'_>,
    area: Rect,
    stats_overlay: StatsOverlayView<'_>,
    palette: &Palette,
) {
    let range_view = stats_overlay.snapshot.range_view(stats_overlay.date_range);
    let total_sessions = range_view.session_rows.len();
    let visible_count = 8_usize;
    let max_offset = total_sessions.saturating_sub(visible_count);
    let scroll_offset = stats_overlay.list_scroll_offset.min(max_offset);
    let body_sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);
    let [title_area, list_area, hint_area] = body_sections.as_ref() else {
        return;
    };

    let title = Line::styled(
        " Session activity",
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(Paragraph::new(title), *title_area);

    let session_lines = range_view
        .session_rows
        .iter()
        .skip(scroll_offset)
        .take(visible_count)
        .map(|row| render_stats_session_line(row, palette))
        .collect::<Vec<_>>();
    let content = if session_lines.is_empty() {
        vec![Line::styled(
            "No session rows available for this range.",
            Style::default().fg(palette.dim),
        )]
    } else {
        session_lines
    };
    frame.render_widget(Paragraph::new(content), *list_area);

    let hint_line = if total_sessions > visible_count {
        let start = scroll_offset.saturating_add(1);
        let end = (scroll_offset + visible_count).min(total_sessions);
        Line::styled(
            format!(" ↑↓ scroll sessions · showing {start}-{end} of {total_sessions} "),
            Style::default().fg(palette.dim),
        )
    } else {
        Line::styled(
            " visible sessions ".to_owned(),
            Style::default().fg(palette.dim),
        )
    };
    frame.render_widget(Paragraph::new(hint_line), *hint_area);
}

fn render_stats_chart(
    frame: &mut Frame<'_>,
    area: Rect,
    chart_view: &stats::StatsChartView,
    palette: &Palette,
) {
    let series_colors = [palette.info, palette.success, palette.warning];
    let mut datasets = Vec::new();

    for (index, series) in chart_view.series.iter().enumerate() {
        let color = series_colors
            .get(index % series_colors.len())
            .copied()
            .unwrap_or(palette.text);
        let dataset = Dataset::default()
            .name(series.label.clone())
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(color))
            .data(series.points.as_slice());
        datasets.push(dataset);
    }

    let x_end = chart_view
        .series
        .first()
        .map(|series| series.points.len().saturating_sub(1) as f64)
        .unwrap_or(0.0);
    let y_max = chart_view.max_tokens as f64;
    let mid_y = (chart_view.max_tokens / 2).max(1);

    let x_labels = vec![
        Span::styled(
            chart_view.start_label.clone(),
            Style::default().fg(palette.dim),
        ),
        Span::styled(
            chart_view.middle_label.clone(),
            Style::default().fg(palette.dim),
        ),
        Span::styled(
            chart_view.end_label.clone(),
            Style::default().fg(palette.dim),
        ),
    ];
    let y_labels = vec![
        Span::styled("0".to_owned(), Style::default().fg(palette.dim)),
        Span::styled(
            stats::format_compact_tokens(mid_y),
            Style::default().fg(palette.dim),
        ),
        Span::styled(
            stats::format_compact_tokens(chart_view.max_tokens),
            Style::default().fg(palette.dim),
        ),
    ];
    let chart = Chart::new(datasets)
        .x_axis(
            Axis::default()
                .bounds([0.0, x_end.max(1.0)])
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, y_max.max(1.0)])
                .labels(y_labels),
        );

    frame.render_widget(chart, area);
}

fn render_stats_chart_legend(
    frame: &mut Frame<'_>,
    area: Rect,
    chart_view: &stats::StatsChartView,
    palette: &Palette,
) {
    let series_colors = [palette.info, palette.success, palette.warning];
    let mut spans = Vec::new();

    for (index, series) in chart_view.series.iter().enumerate() {
        let color = series_colors
            .get(index % series_colors.len())
            .copied()
            .unwrap_or(palette.text);
        spans.push(Span::styled("● ", Style::default().fg(color)));
        spans.push(Span::styled(
            series.label.clone(),
            Style::default().fg(palette.text),
        ));

        if index + 1 < chart_view.series.len() {
            spans.push(Span::styled("  ", Style::default().fg(palette.dim)));
        }
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_stats_model_line(entry: &stats::ModelTokenTotal, palette: &Palette) -> Line<'static> {
    let total_label = stats::format_compact_tokens(entry.total_tokens);
    let input_label = stats::format_compact_tokens(entry.input_tokens);
    let output_label = stats::format_compact_tokens(entry.output_tokens);
    let model_label = format!(" {} ", entry.model);
    let usage_label = format!(
        "{} total · in {} · out {}",
        total_label, input_label, output_label,
    );

    Line::from(vec![
        Span::styled(model_label, Style::default().fg(palette.text)),
        Span::styled("· ", Style::default().fg(palette.dim)),
        Span::styled(usage_label, Style::default().fg(palette.dim)),
    ])
}

fn render_stats_session_line(row: &stats::StatsSessionRow, palette: &Palette) -> Line<'static> {
    let locale = SessionPresentationLocale::detect_from_env();
    let session_label = if row.current {
        format!(" {} (current) ", row.session_id)
    } else {
        format!(" {} ", row.session_id)
    };
    let duration_label = stats::format_duration_compact(row.duration_seconds);
    let date_label = row
        .last_activity_date
        .map(stats::short_date_label)
        .unwrap_or_else(|| "(no turns)".to_owned());
    let label_text = match row.agent_presentation.as_ref() {
        Some(presentation) => presentation.primary_label(locale),
        None => row
            .label
            .clone()
            .unwrap_or_else(|| "(unlabeled)".to_owned()),
    };
    let provider_text = row
        .agent_presentation
        .as_ref()
        .and_then(|presentation| presentation.provider_label(locale));
    let provider_suffix = provider_text
        .as_deref()
        .map(|value| format!(" · {value}"))
        .unwrap_or_default();
    let meta_label = format!(
        "{} · {} · {} turns · {} · {} · {}{}",
        row.state,
        row.kind,
        row.turn_count,
        duration_label,
        date_label,
        label_text,
        provider_suffix,
    );

    Line::from(vec![
        Span::styled(session_label, Style::default().fg(palette.text)),
        Span::styled("· ", Style::default().fg(palette.dim)),
        Span::styled(meta_label, Style::default().fg(palette.dim)),
    ])
}

fn stats_metric_line(label: &str, value: String, palette: &Palette) -> Line<'static> {
    let label_text = format!(" {:<18}", label);
    let value_text = format!(" {value}");

    Line::from(vec![
        Span::styled(label_text, Style::default().fg(palette.dim)),
        Span::styled(
            value_text,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn render_stats_duration(duration: &stats::SessionDurationStat) -> String {
    let hours = duration.duration_seconds / 3600;
    let minutes = (duration.duration_seconds % 3600) / 60;

    if hours > 0 {
        return format!("{hours}h {minutes}m");
    }

    format!("{minutes}m")
}

fn render_command_palette(
    frame: &mut Frame<'_>,
    state: &impl ShellView,
    input_area: Rect,
    textarea: &tui_textarea::TextArea<'_>,
    focus: FocusLayer,
    slash_command_selection: usize,
    palette: &Palette,
) {
    if focus != FocusLayer::Composer {
        return;
    }

    let draft_text = textarea.lines().join("\n");
    let draft_prefix = draft_text.trim();
    if !draft_prefix.starts_with('/') {
        return;
    }

    let matches = state.slash_palette_entries(draft_prefix);
    if matches.is_empty() {
        return;
    }

    let area = frame.area();
    let selected_index = slash_command_selection % matches.len();
    let max_visible_matches = slash_palette_max_visible_matches(area, input_area);
    let window_start =
        slash_palette_window_start(matches.len(), selected_index, max_visible_matches);
    let visible_matches = matches
        .iter()
        .skip(window_start)
        .take(max_visible_matches)
        .collect::<Vec<_>>();
    let visible_selected_index = selected_index.saturating_sub(window_start);
    let popup_area = slash_palette_area(area, input_area, visible_matches.len());

    frame.render_widget(Clear, popup_area);

    let block = Block::default().style(Style::default().bg(palette.surface_alt));
    let inner_area = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let mut lines = vec![Line::from(Span::styled(
        " Commands",
        Style::default()
            .fg(palette.dim)
            .add_modifier(Modifier::BOLD),
    ))];
    for (index, entry) in visible_matches.into_iter().enumerate() {
        let is_selected = index == visible_selected_index;
        let command_style = if is_selected {
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD)
                .bg(palette.surface)
        } else {
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD)
        };
        let help_style = if is_selected {
            Style::default().fg(palette.info)
        } else {
            Style::default().fg(palette.dim)
        };
        let prefix = if is_selected { "› " } else { "  " };
        let prefix_style = if is_selected {
            Style::default().fg(palette.brand).bg(palette.surface)
        } else {
            Style::default().fg(palette.separator)
        };
        let command_span = Span::styled(format!("{:<26}", entry.label), command_style);
        let separator_span = Span::styled(" ", Style::default().fg(palette.separator));
        let category_span = Span::styled(
            format!("[{}] ", entry.meta),
            Style::default().fg(palette.dim),
        );
        let help_span = Span::styled(entry.detail.clone(), help_style);
        let line = Line::from(vec![
            Span::styled(prefix, prefix_style),
            command_span,
            separator_span,
            category_span,
            help_span,
        ]);
        lines.push(line);
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner_area);
}

pub(super) fn slash_palette_max_visible_matches(area: Rect, input_area: Rect) -> usize {
    let available_above = input_area.y.saturating_sub(area.y);
    let visible_rows = available_above.saturating_sub(2).clamp(1, 12);

    usize::from(visible_rows)
}

pub(super) fn slash_palette_window_start(
    total_matches: usize,
    selected_index: usize,
    max_visible_matches: usize,
) -> usize {
    if total_matches <= max_visible_matches {
        return 0;
    }

    let centered_start = selected_index.saturating_sub(max_visible_matches / 2);
    let max_start = total_matches.saturating_sub(max_visible_matches);

    centered_start.min(max_start)
}

pub(super) fn slash_palette_area(area: Rect, input_area: Rect, visible_count: usize) -> Rect {
    let popup_height = u16::try_from(visible_count.saturating_add(1))
        .unwrap_or(u16::MAX)
        .saturating_add(1);
    let popup_width = input_area.width.clamp(28, 90);
    let popup_x = input_area.x;
    let popup_y = input_area.y.saturating_sub(popup_height.saturating_sub(1));
    let clamped_popup_y = popup_y.max(area.y);

    Rect::new(popup_x, clamped_popup_y, popup_width, popup_height)
}

// ---------------------------------------------------------------------------
// Separator
// ---------------------------------------------------------------------------

fn render_separator(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let dots = "·  ·  ·".to_string();
    let line = format!("{dots:^width$}", width = usize::from(area.width));
    let sep = Paragraph::new(Line::styled(line, Style::default().fg(palette.separator)));
    frame.render_widget(sep, area);
}

// ---------------------------------------------------------------------------
// Clarify dialog overlay
// ---------------------------------------------------------------------------

fn render_clarify_dialog(
    dialog: &ClarifyDialog,
    frame: &mut Frame<'_>,
    area: Rect,
    palette: &Palette,
) {
    let popup_width = (area.width * 3 / 5)
        .max(40)
        .min(area.width.saturating_sub(4));
    let question_lines = dialog.question.lines().count() as u16;
    let choices_lines = if dialog.choices.is_empty() {
        0
    } else {
        dialog.choices.len() as u16 + 1
    };
    let input_lines = 3u16;
    let inner_height = question_lines + choices_lines + input_lines + 1;
    let popup_height = (inner_height + 2).min(area.height.saturating_sub(2));

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.separator))
        .title(Span::styled(
            " Agent Question ",
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ))
        .title_position(TitlePosition::Top)
        .title(Span::styled(
            " Enter to submit | Esc to dismiss ",
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ))
        .title_position(TitlePosition::Bottom)
        .style(Style::default().bg(palette.surface));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let mut lines: Vec<Line<'_>> = Vec::new();

    for qline in dialog.question.lines() {
        lines.push(Line::styled(
            format!(" {qline}"),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::default());

    if !dialog.choices.is_empty() {
        lines.push(Line::styled(
            " Choices (Up/Down to select):",
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ));
        for (i, choice) in dialog.choices.iter().enumerate() {
            let is_selected = dialog.selected_choice == Some(i);
            let (prefix, style) = if is_selected {
                (
                    "  > ",
                    Style::default()
                        .fg(palette.brand)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("    ", Style::default().fg(palette.text))
            };
            lines.push(Line::styled(format!("{prefix}{choice}"), style));
        }
        lines.push(Line::default());
    }

    let input_label = if dialog.choices.is_empty() {
        " Your response:"
    } else {
        " Or type a response:"
    };
    lines.push(Line::styled(
        input_label,
        Style::default()
            .fg(palette.dim)
            .add_modifier(Modifier::ITALIC),
    ));

    if dialog.selected_choice.is_some() && dialog.input.is_empty() {
        lines.push(Line::styled(
            " (press Enter to confirm selection)",
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ));
    } else {
        lines.push(Line::styled(
            format!(" {}", &dialog.input),
            Style::default().fg(palette.text),
        ));
    }

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(para, inner);
}

// ---------------------------------------------------------------------------
// Help overlay
// ---------------------------------------------------------------------------

fn render_help_overlay(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let mut content_lines: Vec<Line<'_>> = Vec::new();
    content_lines.push(Line::styled(
        " Shortcuts".to_owned(),
        Style::default()
            .fg(palette.brand)
            .add_modifier(Modifier::BOLD),
    ));
    for (key, desc) in [
        ("Enter / Shift+Enter", "Send message / new line"),
        ("Up/Down PgUp/Dn", "Scroll transcript"),
        ("Home/End", "Jump top or latest"),
        ("Ctrl+G", "Cycle queue / steer mode"),
        ("Ctrl+R", "Toggle transcript review"),
        ("Ctrl+O", "Open latest tool details"),
        ("Ctrl+C", "Interrupt, cancel pending, or quit"),
        ("Esc", "Close dialogs"),
    ] {
        content_lines.push(Line::from(vec![
            Span::styled(format!("  {key:<16}"), Style::default().fg(palette.text)),
            Span::styled(format!(" {desc}"), Style::default().fg(palette.dim)),
        ]));
    }

    content_lines.push(Line::styled(
        " Commands".to_owned(),
        Style::default()
            .fg(palette.brand)
            .add_modifier(Modifier::BOLD),
    ));
    for spec in commands::discoverable_command_specs() {
        let command_label = match spec.argument_hint {
            Some(argument_hint) => format!("{} {}", spec.name, argument_hint),
            None => spec.name.to_owned(),
        };
        content_lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<20}", command_label),
                Style::default().fg(palette.text),
            ),
            Span::styled(
                format!("[{}] ", spec.category),
                Style::default().fg(palette.info),
            ),
            Span::styled(spec.help.to_owned(), Style::default().fg(palette.dim)),
        ]));
    }

    content_lines.push(Line::styled(
        " Transcript".to_owned(),
        Style::default()
            .fg(palette.brand)
            .add_modifier(Modifier::BOLD),
    ));
    for (key, desc) in [
        ("Mouse wheel / drag", "Scroll or update line selection"),
        ("Enter on tool", "Open selected tool details"),
    ] {
        content_lines.push(Line::from(vec![
            Span::styled(format!("  {key:<16}"), Style::default().fg(palette.text)),
            Span::styled(format!(" {desc}"), Style::default().fg(palette.dim)),
        ]));
    }

    let popup_width = 60u16.min(area.width.saturating_sub(4));
    let popup_height = (content_lines.len() as u16 + 2).min(area.height.saturating_sub(2));

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.separator))
        .title(Span::styled(
            " Help ",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ))
        .title_position(TitlePosition::Top)
        .title(Span::styled(
            " Esc or /help to close ",
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ))
        .title_position(TitlePosition::Bottom)
        .style(Style::default().bg(palette.surface));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    frame.render_widget(
        Paragraph::new(content_lines).wrap(Wrap { trim: false }),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Tool inspector overlay
// ---------------------------------------------------------------------------

fn render_tool_inspector(
    tool_inspector: ToolInspectorView<'_>,
    frame: &mut Frame<'_>,
    area: Rect,
    palette: &Palette,
) {
    if area.width < 24 || area.height < 10 {
        return;
    }

    let max_width = area.width.saturating_sub(4);
    let preferred_width = area.width.saturating_mul(4) / 5;
    let popup_width = preferred_width.max(60).min(max_width);

    let max_height = area.height.saturating_sub(2);
    let preferred_height = area.height.saturating_mul(4) / 5;
    let popup_height = preferred_height.max(12).min(max_height);

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.separator))
        .title(Span::styled(
            " Tool Details ",
            Style::default()
                .fg(palette.info)
                .add_modifier(Modifier::BOLD),
        ))
        .title_position(TitlePosition::Top)
        .title(Span::styled(
            " Up/Down tool | PgUp/PgDn output | Esc close ",
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ))
        .title_position(TitlePosition::Bottom)
        .style(Style::default().bg(palette.surface));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let status_summary = render_tool_inspector_status(&tool_inspector, palette);
    let output_text = tool_inspector_output(tool_inspector.status);
    let raw_args = tool_inspector.args_preview.trim();
    let summarized_args = format_tool_args_preview(tool_inspector.tool_name, raw_args);
    let args_display = if summarized_args.is_empty() {
        "(awaiting tool input)".to_owned()
    } else {
        summarized_args.clone()
    };
    let show_raw_args = !raw_args.is_empty() && raw_args != summarized_args;

    let mut content_lines: Vec<Line<'_>> = Vec::new();
    let tool_position = tool_inspector.position + 1;
    let position_label = format!("{tool_position}/{}", tool_inspector.total);

    content_lines.push(Line::from(vec![
        Span::styled(" Tool ".to_string(), Style::default().fg(palette.dim)),
        Span::styled(
            position_label,
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    content_lines.push(Line::from(vec![
        Span::styled(" Name: ".to_string(), Style::default().fg(palette.dim)),
        Span::styled(
            tool_inspector.tool_name.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    content_lines.push(Line::from(vec![
        Span::styled(" Id: ".to_string(), Style::default().fg(palette.dim)),
        Span::styled(
            tool_inspector.tool_id.to_string(),
            Style::default().fg(palette.text),
        ),
    ]));
    content_lines.push(Line::from(vec![
        Span::styled(" Args: ".to_string(), Style::default().fg(palette.dim)),
        Span::styled(args_display, Style::default().fg(palette.text)),
    ]));
    content_lines.push(Line::from(vec![
        Span::styled(" Status: ".to_string(), Style::default().fg(palette.dim)),
        status_summary,
    ]));
    content_lines.push(Line::default());

    if show_raw_args {
        content_lines.push(Line::styled(
            " Raw args",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ));
        for raw_arg_line in raw_args.lines() {
            let prefixed_line = format!(" {raw_arg_line}");
            let line = Line::styled(prefixed_line, Style::default().fg(palette.dim));
            content_lines.push(line);
        }
        content_lines.push(Line::default());
    }

    content_lines.push(Line::styled(
        " Output",
        Style::default()
            .fg(palette.brand)
            .add_modifier(Modifier::BOLD),
    ));

    let rendered_output_lines = render_tool_output_lines(output_text.as_ref(), palette);
    for rendered_output_line in rendered_output_lines {
        content_lines.push(rendered_output_line);
    }

    let paragraph = Paragraph::new(content_lines).wrap(Wrap { trim: false });
    let total_lines = paragraph.line_count(inner.width) as u16;
    let visible_height = inner.height;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_offset = tool_inspector.scroll_offset.min(max_scroll);
    let paragraph = paragraph.scroll((scroll_offset, 0));

    frame.render_widget(paragraph, inner);

    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(total_lines as usize);
        scrollbar_state = scrollbar_state.position(scroll_offset as usize);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(palette.dim));

        frame.render_stateful_widget(
            scrollbar,
            inner.inner(Margin {
                horizontal: 0,
                vertical: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct DiffOutputCursor {
    old_line: usize,
    new_line: usize,
}

#[derive(Debug, Clone, Copy)]
struct DiffOutputMetrics {
    line_number_width: usize,
}

fn render_tool_output_lines(output_text: &str, palette: &Palette) -> Vec<Line<'static>> {
    if output_text.is_empty() {
        let empty_line = Line::styled(" ", Style::default().fg(palette.text));
        return vec![empty_line];
    }

    let metrics = diff_output_metrics(output_text);
    let Some(metrics) = metrics else {
        return output_text
            .lines()
            .map(|output_line| render_plain_tool_output_line(output_line, palette))
            .collect();
    };

    render_diff_tool_output_lines(output_text, palette, metrics)
}

fn render_diff_tool_output_lines(
    output_text: &str,
    palette: &Palette,
    metrics: DiffOutputMetrics,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut cursor: Option<DiffOutputCursor> = None;

    for output_line in output_text.lines() {
        let parsed_hunk_cursor = parse_diff_hunk_cursor(output_line);
        if let Some(parsed_hunk_cursor) = parsed_hunk_cursor {
            cursor = Some(parsed_hunk_cursor);
            let metadata_line = render_diff_metadata_line(output_line, palette);
            lines.push(metadata_line);
            continue;
        }

        let is_metadata_line = is_diff_file_header_line(output_line);
        if is_metadata_line {
            let metadata_line = render_diff_metadata_line(output_line, palette);
            lines.push(metadata_line);
            continue;
        }

        let is_no_newline_marker = output_line.starts_with("\\ No newline at end of file");
        if is_no_newline_marker {
            let marker_line =
                Line::styled(format!(" {output_line}"), Style::default().fg(palette.dim));
            lines.push(marker_line);
            continue;
        }

        let diff_line = render_numbered_diff_output_line(
            output_line,
            palette,
            &mut cursor,
            metrics.line_number_width,
        );
        if let Some(diff_line) = diff_line {
            lines.push(diff_line);
            continue;
        }

        let fallback_line = render_plain_tool_output_line(output_line, palette);
        lines.push(fallback_line);
    }

    lines
}

fn render_plain_tool_output_line(output_line: &str, palette: &Palette) -> Line<'static> {
    let prefixed_line = format!(" {output_line}");
    let style = tool_output_style(output_line, palette);

    Line::styled(prefixed_line, style)
}

fn tool_output_style(output_line: &str, palette: &Palette) -> Style {
    if is_diff_hunk_line(output_line) {
        return Style::default()
            .fg(palette.info)
            .add_modifier(Modifier::BOLD);
    }
    if is_diff_addition_line(output_line) {
        return Style::default().fg(palette.success);
    }
    if is_diff_removal_line(output_line) {
        return Style::default().fg(palette.error);
    }
    if is_diff_file_header_line(output_line) {
        return Style::default().fg(palette.brand);
    }

    Style::default().fg(palette.text)
}

fn render_diff_metadata_line(output_line: &str, palette: &Palette) -> Line<'static> {
    let prefixed_line = format!("   {output_line}");
    let style = tool_output_style(output_line, palette);

    Line::styled(prefixed_line, style)
}

fn render_numbered_diff_output_line(
    output_line: &str,
    palette: &Palette,
    cursor: &mut Option<DiffOutputCursor>,
    line_number_width: usize,
) -> Option<Line<'static>> {
    let line_kind = diff_content_line_kind(output_line)?;
    let cursor = cursor.as_mut()?;

    let (old_number, new_number, sign, content_style) = match line_kind {
        DiffContentLineKind::Context => {
            let old_number = Some(cursor.old_line);
            let new_number = Some(cursor.new_line);
            cursor.old_line = cursor.old_line.saturating_add(1);
            cursor.new_line = cursor.new_line.saturating_add(1);
            let sign = " ";
            let content_style = Style::default().fg(palette.text);
            (old_number, new_number, sign, content_style)
        }
        DiffContentLineKind::Addition => {
            let old_number = None;
            let new_number = Some(cursor.new_line);
            cursor.new_line = cursor.new_line.saturating_add(1);
            let sign = "+";
            let content_style = Style::default().fg(palette.success);
            (old_number, new_number, sign, content_style)
        }
        DiffContentLineKind::Removal => {
            let old_number = Some(cursor.old_line);
            let new_number = None;
            cursor.old_line = cursor.old_line.saturating_add(1);
            let sign = "-";
            let content_style = Style::default().fg(palette.error);
            (old_number, new_number, sign, content_style)
        }
    };

    let old_label = format_diff_line_number(old_number, line_number_width);
    let new_label = format_diff_line_number(new_number, line_number_width);
    let number_style = Style::default().fg(palette.dim);
    let sign_style = content_style.add_modifier(Modifier::BOLD);
    let content = if output_line.len() > 1 {
        output_line[1..].to_owned()
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(format!(" {old_label}"), number_style),
        Span::styled(" ", number_style),
        Span::styled(new_label, number_style),
        Span::styled(" │ ", Style::default().fg(palette.separator)),
        Span::styled(sign.to_owned(), sign_style),
        Span::styled(" ", Style::default().fg(palette.separator)),
        Span::styled(content, content_style),
    ]);

    Some(line)
}

fn format_diff_line_number(value: Option<usize>, width: usize) -> String {
    match value {
        Some(value) => format!("{value:>width$}"),
        None => " ".repeat(width),
    }
}

fn diff_output_metrics(output_text: &str) -> Option<DiffOutputMetrics> {
    let looks_like_diff = output_text.lines().any(is_diff_hunk_line);
    if !looks_like_diff {
        return None;
    }

    let mut cursor: Option<DiffOutputCursor> = None;
    let mut max_line_number = 0_usize;

    for output_line in output_text.lines() {
        let parsed_hunk_cursor = parse_diff_hunk_cursor(output_line);
        if let Some(parsed_hunk_cursor) = parsed_hunk_cursor {
            max_line_number = max_line_number.max(parsed_hunk_cursor.old_line);
            max_line_number = max_line_number.max(parsed_hunk_cursor.new_line);
            cursor = Some(parsed_hunk_cursor);
            continue;
        }

        let Some(cursor) = cursor.as_mut() else {
            continue;
        };

        let line_kind = diff_content_line_kind(output_line);
        let Some(line_kind) = line_kind else {
            continue;
        };

        match line_kind {
            DiffContentLineKind::Context => {
                max_line_number = max_line_number.max(cursor.old_line);
                max_line_number = max_line_number.max(cursor.new_line);
                cursor.old_line = cursor.old_line.saturating_add(1);
                cursor.new_line = cursor.new_line.saturating_add(1);
            }
            DiffContentLineKind::Addition => {
                max_line_number = max_line_number.max(cursor.new_line);
                cursor.new_line = cursor.new_line.saturating_add(1);
            }
            DiffContentLineKind::Removal => {
                max_line_number = max_line_number.max(cursor.old_line);
                cursor.old_line = cursor.old_line.saturating_add(1);
            }
        }
    }

    let line_number_width = decimal_width(max_line_number.max(1));
    let metrics = DiffOutputMetrics { line_number_width };

    Some(metrics)
}

fn decimal_width(value: usize) -> usize {
    value.to_string().len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffContentLineKind {
    Context,
    Addition,
    Removal,
}

fn diff_content_line_kind(output_line: &str) -> Option<DiffContentLineKind> {
    if is_diff_context_line(output_line) {
        return Some(DiffContentLineKind::Context);
    }
    if is_diff_addition_line(output_line) {
        return Some(DiffContentLineKind::Addition);
    }
    if is_diff_removal_line(output_line) {
        return Some(DiffContentLineKind::Removal);
    }

    None
}

fn parse_diff_hunk_cursor(output_line: &str) -> Option<DiffOutputCursor> {
    if !is_diff_hunk_line(output_line) {
        return None;
    }

    let tokens = output_line.split_whitespace().collect::<Vec<_>>();
    let old_token = tokens.get(1).copied()?;
    let new_token = tokens.get(2).copied()?;
    let old_line = parse_diff_range_start(old_token, '-')?;
    let new_line = parse_diff_range_start(new_token, '+')?;
    let cursor = DiffOutputCursor { old_line, new_line };

    Some(cursor)
}

fn parse_diff_range_start(token: &str, prefix: char) -> Option<usize> {
    let trimmed = token.strip_prefix(prefix)?;
    let start = trimmed.split(',').next()?;
    let line_number = start.parse::<usize>().ok()?;

    Some(line_number)
}

fn is_diff_context_line(output_line: &str) -> bool {
    output_line.starts_with(' ')
}

fn is_diff_hunk_line(output_line: &str) -> bool {
    output_line.starts_with("@@")
}

fn is_diff_addition_line(output_line: &str) -> bool {
    output_line.starts_with('+') && !output_line.starts_with("+++")
}

fn is_diff_removal_line(output_line: &str) -> bool {
    output_line.starts_with('-') && !output_line.starts_with("---")
}

fn is_diff_file_header_line(output_line: &str) -> bool {
    output_line.starts_with("diff --")
        || output_line.starts_with("index ")
        || output_line.starts_with("--- ")
        || output_line.starts_with("+++ ")
}

fn render_tool_inspector_status(
    tool_inspector: &ToolInspectorView<'_>,
    palette: &Palette,
) -> Span<'static> {
    match tool_inspector.status {
        ToolStatus::Running { started } => {
            let elapsed_seconds = started.elapsed().as_secs_f32();
            let summary = format!("running | {elapsed_seconds:.1}s elapsed");

            Span::styled(summary, Style::default().fg(palette.tool_running))
        }
        ToolStatus::Done {
            success,
            duration_ms,
            ..
        } => {
            let status_label = if *success { "success" } else { "failed" };
            let duration_label = format!("{duration_ms}ms");
            let summary = format!("{status_label} | {duration_label}");
            let color = if *success {
                palette.tool_done
            } else {
                palette.tool_fail
            };

            Span::styled(summary, Style::default().fg(color))
        }
    }
}

fn tool_inspector_output<'a>(status: &'a ToolStatus) -> Cow<'a, str> {
    match status {
        ToolStatus::Running { .. } => Cow::Borrowed("Waiting for tool output..."),
        ToolStatus::Done { output, .. } => Cow::Borrowed(output.as_str()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::tui::commands;
    use crate::chat::tui::dialog::ClarifyDialog;
    use crate::chat::tui::message::{Message, ToolStatus};
    use crate::chat::tui::state::BusyInputMode;
    use ratatui::{Terminal, backend::TestBackend};
    use std::time::Instant;

    struct TestToolInspector {
        tool_id: String,
        tool_name: String,
        args_preview: String,
        status: ToolStatus,
        scroll_offset: u16,
        position: usize,
        total: usize,
    }

    // Unified test pane implementing all view traits.
    struct TestPane {
        messages: Vec<Message>,
        scroll_offset: u16,
        streaming_active: bool,
        agent_running: bool,
        spinner_frame: usize,
        dots_frame: usize,
        loop_state: String,
        loop_action: String,
        loop_iteration: u32,
        status_message: Option<(String, Instant)>,
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        context_length: u32,
        session_id: String,
    }

    impl TestPane {
        fn default_idle() -> Self {
            Self {
                messages: Vec::new(),
                scroll_offset: 0,
                streaming_active: false,
                agent_running: false,
                spinner_frame: 0,
                dots_frame: 0,
                loop_state: String::new(),
                loop_action: String::new(),
                loop_iteration: 0,
                status_message: None,
                model: "test-model".into(),
                input_tokens: 100,
                output_tokens: 50,
                context_length: 10000,
                session_id: "test-sess".into(),
            }
        }
    }

    impl PaneView for TestPane {
        fn messages(&self) -> &[Message] {
            &self.messages
        }
        fn scroll_offset(&self) -> u16 {
            self.scroll_offset
        }
        fn streaming_active(&self) -> bool {
            self.streaming_active
        }
    }

    impl SpinnerView for TestPane {
        fn agent_running(&self) -> bool {
            self.agent_running
        }
        fn spinner_frame(&self) -> usize {
            self.spinner_frame
        }
        fn dots_frame(&self) -> usize {
            self.dots_frame
        }
        fn loop_state(&self) -> &str {
            &self.loop_state
        }
        fn loop_action(&self) -> &str {
            &self.loop_action
        }
        fn loop_iteration(&self) -> u32 {
            self.loop_iteration
        }
        fn status_message(&self) -> Option<(&str, &Instant)> {
            self.status_message.as_ref().map(|(s, i)| (s.as_str(), i))
        }
    }

    impl StatusBarView for TestPane {
        fn model(&self) -> &str {
            &self.model
        }
        fn input_tokens(&self) -> u32 {
            self.input_tokens
        }
        fn output_tokens(&self) -> u32 {
            self.output_tokens
        }
        fn context_length(&self) -> u32 {
            self.context_length
        }
        fn session_id(&self) -> &str {
            &self.session_id
        }
        fn busy_input_mode(&self) -> BusyInputMode {
            BusyInputMode::Queue
        }
    }

    impl InputView for TestPane {
        fn agent_running(&self) -> bool {
            self.agent_running
        }
        fn pending_submission_count(&self) -> usize {
            0
        }
        fn busy_input_mode(&self) -> BusyInputMode {
            BusyInputMode::Queue
        }
    }

    struct TestShell {
        pane: TestPane,
        show_thinking: bool,
        focus: FocusStack,
        clarify_dialog: Option<ClarifyDialog>,
        stats_overlay: Option<StatsOverlayView<'static>>,
        session_picker: Option<state::SessionPickerState>,
        slash_palette_entries_override: Option<Vec<SlashPaletteEntry>>,
        tool_inspector: Option<TestToolInspector>,
        slash_command_selection: usize,
    }

    impl TestShell {
        fn idle() -> Self {
            Self {
                pane: TestPane::default_idle(),
                show_thinking: false,
                focus: FocusStack::new(),
                clarify_dialog: None,
                stats_overlay: None,
                session_picker: None,
                slash_palette_entries_override: None,
                tool_inspector: None,
                slash_command_selection: 0,
            }
        }
    }

    impl ShellView for TestShell {
        type Pane = TestPane;

        fn pane(&self) -> &TestPane {
            &self.pane
        }
        fn show_thinking(&self) -> bool {
            self.show_thinking
        }
        fn focus(&self) -> &FocusStack {
            &self.focus
        }
        fn clarify_dialog(&self) -> Option<&ClarifyDialog> {
            self.clarify_dialog.as_ref()
        }
        fn tool_inspector(&self) -> Option<ToolInspectorView<'_>> {
            let inspector = self.tool_inspector.as_ref()?;

            Some(ToolInspectorView {
                tool_id: inspector.tool_id.as_str(),
                tool_name: inspector.tool_name.as_str(),
                args_preview: inspector.args_preview.as_str(),
                status: &inspector.status,
                scroll_offset: inspector.scroll_offset,
                position: inspector.position,
                total: inspector.total,
            })
        }
        fn stats_overlay(&self) -> Option<StatsOverlayView<'_>> {
            self.stats_overlay
        }
        fn session_picker(&self) -> Option<SessionPickerView<'_>> {
            let picker = self.session_picker.as_ref()?;

            Some(SessionPickerView {
                picker,
                current_session_id: self.pane.session_id.as_str(),
            })
        }
        fn slash_command_selection(&self) -> usize {
            self.slash_command_selection
        }
        fn slash_palette_entries(&self, draft_prefix: &str) -> Vec<SlashPaletteEntry> {
            if let Some(entries) = self.slash_palette_entries_override.as_ref() {
                return entries.clone();
            }

            commands::completions(draft_prefix)
                .into_iter()
                .map(|spec| SlashPaletteEntry {
                    replacement: spec.name.to_owned(),
                    label: match spec.argument_hint {
                        Some(argument_hint) => format!("{} {}", spec.name, argument_hint),
                        None => spec.name.to_owned(),
                    },
                    meta: spec.category.to_owned(),
                    detail: spec.help.to_owned(),
                    immediate: false,
                    submit_on_select: false,
                })
                .collect()
        }
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push(
                    buf.cell((x, y))
                        .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' ')),
                );
            }
        }
        out
    }

    fn sample_stats_snapshot() -> stats::StatsSnapshot {
        let today = chrono::Utc::now().date_naive();
        let first_date = today - chrono::Duration::days(2);
        let second_date = today - chrono::Duration::days(1);
        let mut first_models = std::collections::BTreeMap::new();
        first_models.insert(
            "gpt-5".to_owned(),
            stats::ModelTokenAccumulator {
                input_tokens: 120,
                output_tokens: 80,
            },
        );
        let mut second_models = std::collections::BTreeMap::new();
        second_models.insert(
            "o4-mini".to_owned(),
            stats::ModelTokenAccumulator {
                input_tokens: 180,
                output_tokens: 140,
            },
        );

        stats::StatsSnapshot {
            visible_sessions: 2,
            root_sessions: 1,
            delegate_sessions: 1,
            running_delegate_sessions: 1,
            pending_approvals: 1,
            usage_event_count: 2,
            first_activity_date: Some(first_date),
            last_activity_date: Some(second_date),
            longest_session: Some(stats::SessionDurationStat {
                session_id: "sess-1".to_owned(),
                label: Some("Root".to_owned()),
                duration_seconds: 5400,
            }),
            session_rows: vec![stats::StatsSessionRow {
                session_id: "sess-1".to_owned(),
                label: Some("Root".to_owned()),
                agent_presentation: None,
                kind: "root".to_owned(),
                state: "ready".to_owned(),
                turn_count: 2,
                duration_seconds: 5400,
                last_activity_date: Some(second_date),
                current: true,
            }],
            active_dates: vec![first_date, second_date],
            daily_points: vec![
                stats::DailyTokenPoint {
                    date: first_date,
                    total_input_tokens: 120,
                    total_output_tokens: 80,
                    total_tokens: 200,
                    model_tokens: first_models,
                },
                stats::DailyTokenPoint {
                    date: second_date,
                    total_input_tokens: 180,
                    total_output_tokens: 140,
                    total_tokens: 320,
                    model_tokens: second_models,
                },
            ],
        }
    }

    #[test]
    fn full_draw_does_not_panic() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let shell = TestShell::idle();
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw should not panic");
    }

    #[test]
    fn draw_with_help_overlay() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut focus = FocusStack::new();
        focus.push(FocusLayer::Help);
        let shell = TestShell {
            focus,
            ..TestShell::idle()
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("Help"), "help overlay should be visible");
        assert!(
            text.contains("Ctrl+O"),
            "help overlay should advertise tool inspection shortcut"
        );
        assert!(
            text.contains("Ctrl+R"),
            "help overlay should advertise transcript review shortcut"
        );
    }

    #[test]
    fn draw_with_stats_overlay() {
        let backend = TestBackend::new(100, 34);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut focus = FocusStack::new();
        focus.push(FocusLayer::StatsOverlay);
        let shell = TestShell {
            focus,
            stats_overlay: Some(StatsOverlayView {
                snapshot: Box::leak(Box::new(sample_stats_snapshot())),
                active_tab: stats::StatsTab::Models,
                date_range: stats::StatsDateRange::All,
                list_scroll_offset: 0,
                copy_status: Some("copied"),
            }),
            ..TestShell::idle()
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("Stats"), "stats overlay should be visible");
        assert!(text.contains("Overview"), "stats tabs should render");
        assert!(text.contains("Models"), "stats tabs should render");
        assert!(
            text.contains("Tokens per Day"),
            "stats chart title should render"
        );
    }

    #[test]
    fn draw_with_clarify_dialog() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut focus = FocusStack::new();
        focus.push(FocusLayer::ClarifyDialog);
        let shell = TestShell {
            focus,
            clarify_dialog: Some(ClarifyDialog::new(
                "Pick a tool".into(),
                vec!["bash".into(), "read".into()],
            )),
            ..TestShell::idle()
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Agent Question"),
            "clarify dialog should be visible"
        );
    }

    #[test]
    fn draw_with_tool_inspector_overlay() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut focus = FocusStack::new();
        focus.push(FocusLayer::ToolInspector);
        let tool_inspector = TestToolInspector {
            tool_id: "tool-2".into(),
            tool_name: "shell".into(),
            args_preview: "ls -la".into(),
            status: ToolStatus::Done {
                success: true,
                output: "line 1\nline 2".into(),
                duration_ms: 24,
            },
            scroll_offset: 0,
            position: 1,
            total: 2,
        };
        let shell = TestShell {
            focus,
            tool_inspector: Some(tool_inspector),
            ..TestShell::idle()
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Tool Details"),
            "tool inspector overlay should be visible"
        );
        assert!(
            text.contains("line 2"),
            "tool inspector should render tool output"
        );
    }

    #[test]
    fn draw_with_tool_inspector_overlay_renders_multiline_args() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut focus = FocusStack::new();
        focus.push(FocusLayer::ToolInspector);
        let tool_inspector = TestToolInspector {
            tool_id: "tool-3".into(),
            tool_name: "file.edit".into(),
            args_preview: "path: docs/notes.md\nreplace: draft -> final".into(),
            status: ToolStatus::Done {
                success: true,
                output: "edited docs/notes.md".into(),
                duration_ms: 24,
            },
            scroll_offset: 0,
            position: 0,
            total: 1,
        };
        let shell = TestShell {
            focus,
            tool_inspector: Some(tool_inspector),
            ..TestShell::idle()
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("replace: draft -> final"),
            "tool inspector should render multiline args content"
        );
    }

    #[test]
    fn draw_uses_compact_shell_on_narrow_terminal() {
        let backend = TestBackend::new(28, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let shell = TestShell::idle();
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("LOONGCLAW"),
            "compact shell should keep the product identity visible: {text:?}"
        );
        assert!(
            text.contains("Type a message"),
            "compact shell should still render body content on narrow terminals: {text:?}"
        );
    }

    #[test]
    fn draw_shows_slash_command_palette_for_matching_prefix() {
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let shell = TestShell::idle();
        let palette = Palette::dark();
        let mut textarea = tui_textarea::TextArea::default();
        textarea.insert_str("/rev");

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Commands"),
            "slash palette should be visible: {text:?}"
        );
        assert!(
            text.contains("/review"),
            "matching command should be rendered: {text:?}"
        );
    }

    #[test]
    fn draw_scrolls_slash_command_palette_with_selection() {
        let backend = TestBackend::new(96, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let entries = (0..16)
            .map(|index| SlashPaletteEntry {
                replacement: format!("/cmd-{index}"),
                label: format!("/cmd-{index}"),
                meta: "Test".to_owned(),
                detail: format!("detail {index}"),
                immediate: false,
                submit_on_select: false,
            })
            .collect::<Vec<_>>();
        let shell = TestShell {
            slash_palette_entries_override: Some(entries),
            slash_command_selection: 12,
            ..TestShell::idle()
        };
        let palette = Palette::dark();
        let mut textarea = tui_textarea::TextArea::default();
        textarea.insert_str("/");

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("/cmd-12"),
            "selected command should stay visible when scrolled: {text:?}"
        );
        assert!(
            !text.contains("/cmd-0"),
            "palette should scroll past the earliest commands: {text:?}"
        );
    }

    #[test]
    fn draw_command_palette_shows_more_than_three_commands() {
        let backend = TestBackend::new(100, 22);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let shell = TestShell::idle();
        let palette = Palette::dark();
        let mut textarea = tui_textarea::TextArea::default();
        textarea.insert_str("/");

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("/help"),
            "palette should show /help: {text:?}"
        );
        assert!(
            text.contains("/commands"),
            "palette should show /commands: {text:?}"
        );
        assert!(text.contains("/new"), "palette should show /new: {text:?}");
        assert!(
            text.contains("/rename"),
            "palette should show /rename: {text:?}"
        );
        assert!(
            text.contains("/clear"),
            "palette should show /clear: {text:?}"
        );
    }

    #[test]
    fn draw_renders_session_picker_with_human_thread_details() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut focus = FocusStack::new();
        focus.push(FocusLayer::SessionPicker);
        let session_picker = state::SessionPickerState {
            mode: state::SessionPickerMode::Subagents,
            sessions: vec![
                state::VisibleSessionSuggestion {
                    session_id: "root-session".to_owned(),
                    label: Some("Main thread".to_owned()),
                    agent_presentation: None,
                    state: "running".to_owned(),
                    kind: "root".to_owned(),
                    task_phase: None,
                    overdue: false,
                    pending_approval_count: 0,
                    attention_approval_count: 0,
                },
                state::VisibleSessionSuggestion {
                    session_id: "delegate:1".to_owned(),
                    label: Some("Reference pass".to_owned()),
                    agent_presentation: Some(
                        crate::session::presentation::DelegateAgentPresentation {
                            persona_id: "xu-xiake".to_owned(),
                            role_id: "explorer".to_owned(),
                            names: crate::session::presentation::LocalizedSubagentText {
                                zh_hans: "徐霞客".to_owned(),
                                zh_hant: "徐霞客".to_owned(),
                                en: "Xu Xiake".to_owned(),
                                ja: "徐霞客".to_owned(),
                            },
                            roles: crate::session::presentation::LocalizedSubagentText {
                                zh_hans: "行者".to_owned(),
                                zh_hant: "行者".to_owned(),
                                en: "Explorer".to_owned(),
                                ja: "探索者".to_owned(),
                            },
                            model: Some("gpt-5".to_owned()),
                            reasoning_effort: Some("high".to_owned()),
                        },
                    ),
                    state: "running".to_owned(),
                    kind: "delegate_child".to_owned(),
                    task_phase: Some("running".to_owned()),
                    overdue: false,
                    pending_approval_count: 1,
                    attention_approval_count: 0,
                },
            ],
            query: String::new(),
            selected_index: 0,
            list_scroll_offset: 0,
        };
        let shell = TestShell {
            focus,
            session_picker: Some(session_picker),
            ..TestShell::idle()
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                draw(f, &shell, &textarea, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("Subagents"),
            "picker title should render: {text:?}"
        );
        assert!(
            text.contains("Main thread"),
            "custom root thread label should render: {text:?}"
        );
        assert!(
            text.contains("running · thread"),
            "root detail should use human thread wording: {text:?}"
        );
        assert!(
            text.contains("Xu Xiake · Explorer"),
            "subagent primary label should render: {text:?}"
        );
        assert!(
            text.contains("gpt-5 · high"),
            "subagent provider label should render: {text:?}"
        );
    }

    #[test]
    fn separator_renders_centered_soft_divider() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_separator(f, f.area(), &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("·"), "separator should render a soft divider");
    }
}
