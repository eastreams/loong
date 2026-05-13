use super::pending_motion;
use super::utils::*;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingBandGeometry {
    pub width: u16,
    pub height: u16,
    pub composer_height: u16,
    pub palette_height: u16,
}

impl PendingBandGeometry {
    pub(super) fn max_pending_height(self) -> u16 {
        let reserved_without_pending = 1
            + self.composer_height
            + if self.palette_height > 0 {
                1 + self.palette_height
            } else {
                0
            }
            + 1
            + 1
            + 1;
        self.height.saturating_sub(reserved_without_pending).max(3)
    }

    pub(super) fn preview_budget(self) -> usize {
        self.max_pending_height().saturating_sub(2).max(1) as usize
    }
}

pub(super) fn pending_signature_preview_budget_for_geometry(geometry: PendingBandGeometry) -> usize {
    geometry.preview_budget()
}

pub(super) fn max_pending_height(geometry: PendingBandGeometry) -> u16 {
    geometry.max_pending_height()
}

pub(super) fn pending_render_signature_for_geometry(
    pending_turn: bool,
    turn_start: Option<std::time::Instant>,
    spinner_seed: u64,
    live_lines: &[String],
    pending_steers: &VecDeque<String>,
    pending_queue: &VecDeque<String>,
    geometry: PendingBandGeometry,
) -> Option<u64> {
    if !pending_turn {
        return None;
    }
    let start = turn_start?;
    let max_pending_preview_lines = geometry.preview_budget();
    let visible_lines = live_lines
        .iter()
        .filter(|line| pending_line_is_tool_activity(line))
        .take(max_pending_preview_lines)
        .cloned()
        .collect::<Vec<_>>();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    crate::chat::chat_surface::utils::focus_ring_frame(start).hash(&mut hasher);
    crate::chat::chat_surface::utils::get_spinner_verb_with_seed(start, spinner_seed)
        .hash(&mut hasher);
    geometry.width.hash(&mut hasher);
    geometry.height.hash(&mut hasher);
    visible_lines.hash(&mut hasher);
    pending_steers.iter().for_each(|message| message.hash(&mut hasher));
    pending_queue.iter().for_each(|message| message.hash(&mut hasher));
    Some(hasher.finish())
}

pub(super) fn build_pending_lines(
    turn_start: Option<std::time::Instant>,
    live_lines: &[String],
    spinner_seed: u64,
    pending_steers: &VecDeque<String>,
    pending_queue: &VecDeque<String>,
    width: u16,
) -> Vec<Line<'static>> {
    let start = turn_start.unwrap_or_else(std::time::Instant::now);
    let has_tool_activity = live_lines.iter().any(|line| pending_line_is_tool_activity(line));
    let spinner_spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("{} ", crate::chat::chat_surface::utils::focus_ring_frame(start)),
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "{}...",
                crate::chat::chat_surface::utils::get_spinner_verb_with_seed(start, spinner_seed)
            ),
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let content_width = width.saturating_sub(2).max(1) as usize;
    let mut lines = Vec::new();
    if has_tool_activity {
        lines.push(Line::from(vec![Span::styled(
            super::utils::divider_rule_text(content_width),
            Style::default()
                .fg(SURFACE_DIM_GRAY)
                .add_modifier(Modifier::DIM),
        )]));
    }
    let has_visible_reply_after_blank = live_lines
        .iter()
        .position(|line| line.trim().is_empty())
        .is_some_and(|blank_idx| {
            live_lines
                .iter()
                .skip(blank_idx + 1)
                .any(|line| !line.trim().is_empty())
        });
    let mut in_reasoning_block = has_visible_reply_after_blank;

    for line in live_lines {
        if line.trim().is_empty() {
            if has_visible_reply_after_blank && !lines.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    super::utils::divider_rule_text(content_width),
                    Style::default()
                        .fg(SURFACE_DIM_GRAY)
                        .add_modifier(Modifier::DIM),
                )]));
            } else {
                lines.push(Line::from(""));
            }
            if has_visible_reply_after_blank {
                in_reasoning_block = false;
            }
            continue;
        }

        let style = if in_reasoning_block {
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM | Modifier::ITALIC)
        } else {
            Style::default().fg(ratatui::style::Color::White)
        };
        lines.extend(render_pending_live_line(
            line.as_str(),
            content_width,
            style,
            start,
        ));
    }

    append_pending_input_preview_lines(
        &mut lines,
        pending_steers,
        pending_queue,
        width,
        !live_lines.is_empty(),
    );
    lines.push(Line::from(""));
    lines.push(Line::from(spinner_spans));
    lines
}

fn render_pending_live_line(
    line: &str,
    content_width: usize,
    default_style: Style,
    start: std::time::Instant,
) -> Vec<Line<'static>> {
    if let Some(lines) = render_pending_tool_headline_line(line, content_width, start) {
        return lines;
    }

    if let Some(lines) = render_pending_tool_child_line(line, content_width) {
        return lines;
    }

    if let Some(lines) = render_pending_tool_sample_line(line, content_width) {
        return lines;
    }

    crate::presentation::render_wrapped_plain_display_line(line, content_width)
        .into_iter()
        .map(|wrapped| Line::from(vec![Span::raw("  "), Span::styled(wrapped, default_style)]))
        .collect()
}

fn render_pending_tool_headline_line(
    line: &str,
    content_width: usize,
    start: std::time::Instant,
) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("• ").unwrap_or(trimmed);
    let (label, rest, label_style, body_style) = pending_tool_headline_parts(trimmed, start)?;
    let label_text = format!("{label} ");
    let prefix_width = 2 + crate::presentation::display_width(label_text.as_str());
    let mut wrapped =
        crate::presentation::render_wrapped_literal_display_line(rest.trim(), content_width.saturating_sub(prefix_width).max(1));
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled("• ", Style::default().fg(SURFACE_GRAY)),
                        Span::styled(label_text.clone(), label_style),
                        Span::styled(wrapped_line, body_style),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(prefix_width)),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect(),
    )
}

fn pending_tool_headline_parts(
    trimmed: &str,
    start: std::time::Instant,
) -> Option<(&'static str, &str, Style, Style)> {
    if let Some(rest) = trimmed.strip_prefix("Called ") {
        return Some((
            "Called",
            rest,
            Style::default()
                .fg(pending_motion::pending_tool_label_color(start))
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(pending_motion::pending_tool_body_color(start))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(rest) = trimmed.strip_prefix("Closed ") {
        return Some((
            "Closed",
            rest,
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_GRAY),
        ));
    }
    if let Some(rest) = trimmed.strip_prefix("Approval ") {
        return Some((
            "Approval",
            rest,
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::White),
        ));
    }
    if let Some(rest) = trimmed.strip_prefix("Denied ") {
        return Some((
            "Denied",
            rest,
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED),
        ));
    }
    None
}

fn render_pending_tool_child_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let body = trimmed.strip_prefix("↳ ")?;
    let (label, rest) = body.split_once(' ').unwrap_or((body, ""));
    let label_text = if rest.is_empty() {
        String::new()
    } else {
        format!("{label} ")
    };
    let (label_style, body_style) = pending_tool_child_styles(label);
    let prefix_width = 2 + crate::presentation::display_width(label_text.as_str());
    let mut wrapped =
        crate::presentation::render_wrapped_literal_display_line(rest.trim(), content_width.saturating_sub(prefix_width).max(1));
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled("↳ ", Style::default().fg(SURFACE_ACCENT)),
                    ];
                    if !label_text.is_empty() {
                        spans.push(Span::styled(label_text.clone(), label_style));
                    }
                    spans.push(Span::styled(wrapped_line, body_style));
                    Line::from(spans)
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(prefix_width)),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect(),
    )
}

fn pending_tool_child_styles(label: &str) -> (Style, Style) {
    match label {
        "stdout" => (
            Style::default()
                .fg(SURFACE_GREEN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "stderr" => (
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
        ),
        "file" => (
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "metrics" => (
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "request" | "args" => (
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        _ => (
            Style::default().fg(SURFACE_ACCENT),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
    }
}

fn render_pending_tool_sample_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    if !line.starts_with("    ") {
        return None;
    }
    let sample = line.trim_start();
    if sample.is_empty() {
        return None;
    }
    let sample_style = if sample.starts_with('+') {
        Style::default().fg(SURFACE_GREEN)
    } else if sample.starts_with('-') {
        Style::default().fg(SURFACE_RED)
    } else {
        Style::default().fg(SURFACE_DARK_GRAY)
    };
    let sample_width = content_width.saturating_sub(4).max(1);
    Some(
        crate::presentation::render_wrapped_literal_display_line(sample, sample_width)
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                let guide = if index == 0 { "    " } else { "      " };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(guide, Style::default().fg(SURFACE_DARK_GRAY)),
                    Span::styled(wrapped_line, sample_style),
                ])
            })
            .collect(),
    )
}

fn append_pending_input_preview_lines(
    lines: &mut Vec<Line<'static>>,
    pending_steers: &VecDeque<String>,
    pending_queue: &VecDeque<String>,
    width: u16,
    has_live_preview: bool,
) {
    const MAX_PENDING_PREVIEW_MESSAGES: usize = 3;

    if pending_steers.is_empty() && pending_queue.is_empty() {
        return;
    }
    if has_live_preview || lines.last().is_some_and(|line| !line.spans.is_empty()) {
        lines.push(Line::from(""));
    }

    let content_width = width.saturating_sub(6).max(1) as usize;
    let mut remaining_preview_budget = MAX_PENDING_PREVIEW_MESSAGES;
    if !pending_steers.is_empty() {
        push_pending_input_header(
            lines,
            content_width,
            "Messages to be submitted after next tool call",
            Some("Esc"),
            "to interrupt and send immediately",
        );
        let preview_items = pending_steers
            .iter()
            .map(|message| {
                (
                    message.as_str(),
                    Style::default()
                        .fg(SURFACE_CYAN)
                        .add_modifier(Modifier::DIM),
                )
            })
            .collect::<Vec<_>>();
        let displayed = push_pending_input_lines(
            lines,
            &preview_items,
            content_width,
            "    ↳ ",
            remaining_preview_budget,
        );
        remaining_preview_budget = remaining_preview_budget.saturating_sub(displayed);
    }

    if !pending_queue.is_empty() {
        if !pending_steers.is_empty() {
            lines.push(Line::from(""));
        }
        push_pending_input_header(lines, content_width, "Queued follow-up messages", None, "");
        let preview_items = pending_queue
            .iter()
            .map(|message| {
                (
                    message.as_str(),
                    Style::default()
                        .fg(SURFACE_GRAY)
                        .add_modifier(Modifier::DIM | Modifier::ITALIC),
                )
            })
            .collect::<Vec<_>>();
        push_pending_input_lines(
            lines,
            &preview_items,
            content_width,
            "    ↳ ",
            remaining_preview_budget,
        );
    }
}

fn push_pending_input_header(
    lines: &mut Vec<Line<'static>>,
    content_width: usize,
    title: &str,
    key_hint: Option<&str>,
    suffix: &str,
) {
    let mut spans = vec![
        Span::styled(
            "• ",
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ),
        Span::styled(title.to_owned(), Style::default().fg(SURFACE_GRAY)),
    ];
    if let Some(key_hint) = key_hint {
        spans.push(Span::styled(
            " (press ".to_owned(),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ));
        spans.push(Span::styled(
            key_hint.to_owned(),
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {suffix})"),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ));
    }
    for (line_index, wrapped) in crate::presentation::render_wrapped_text_line(
        "",
        &spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        content_width + 2,
    )
    .into_iter()
    .enumerate()
    {
        let prefix = if line_index == 0 { "" } else { "  " };
        lines.push(Line::from(vec![Span::styled(
            format!("{prefix}{wrapped}"),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        )]));
    }
}

fn push_pending_input_lines(
    lines: &mut Vec<Line<'static>>,
    messages: &[(&str, Style)],
    content_width: usize,
    first_prefix: &str,
    max_preview_messages: usize,
) -> usize {
    let displayed_messages = messages.len().min(max_preview_messages);
    for (message, message_style) in messages.iter().take(max_preview_messages) {
        let wrapped_lines =
            crate::presentation::render_wrapped_literal_display_line(message, content_width);
        let wrapped_count = wrapped_lines.len();
        for (line_index, wrapped) in wrapped_lines.into_iter().take(3).enumerate() {
            let prefix = if line_index == 0 {
                first_prefix.to_owned()
            } else {
                "      ".to_owned()
            };
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(wrapped, *message_style),
            ]));
        }

        if wrapped_count > 3 {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled("…".to_owned(), *message_style),
            ]));
        }
    }

    let remaining_messages = messages.len().saturating_sub(displayed_messages);
    if remaining_messages > 0 {
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("… +{remaining_messages} more"),
                Style::default()
                    .fg(SURFACE_GRAY)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
    }

    displayed_messages
}

pub(super) fn compact_pending_lines(
    mut lines: Vec<Line<'static>>,
    max_height: u16,
) -> Vec<Line<'static>> {
    let max_height = max_height.max(1) as usize;
    if lines.len() <= max_height {
        return lines;
    }

    let removable_blank_indices = [0usize, lines.len().saturating_sub(1), 2usize];
    for index in removable_blank_indices {
        if lines.len() <= max_height {
            break;
        }
        if lines
            .get(index)
            .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
        {
            lines.remove(index);
        }
    }

    while lines.len() > max_height {
        if let Some(index) = lines.iter().enumerate().skip(2).find_map(|(idx, line)| {
            line.spans
                .iter()
                .all(|span| span.content.trim().is_empty())
                .then_some(idx)
        }) {
            lines.remove(index);
        } else if let Some(index) = lines.iter().enumerate().find_map(|(idx, line)| {
            let text = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            (text.contains("Queued follow-up messages")
                || text.contains("Messages to be submitted after next tool call"))
            .then_some(idx)
        }) {
            lines.remove(index);
        } else {
            break;
        }
    }

    lines.truncate(max_height);
    lines
}

pub(super) fn pending_line_is_tool_activity(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('•')
        || trimmed.starts_with("[running]")
        || trimmed.starts_with("[pending]")
        || trimmed.starts_with("[completed]")
        || trimmed.starts_with("[failed]")
        || trimmed.starts_with("[interrupted]")
        || trimmed.starts_with("[needs_approval]")
        || trimmed.starts_with("[denied]")
        || trimmed.starts_with("request:")
        || trimmed.starts_with("args:")
        || trimmed.starts_with("stdout:")
        || trimmed.starts_with("stderr:")
        || trimmed.starts_with("file:")
        || trimmed.starts_with("metrics:")
        || trimmed.starts_with("↳ ")
}
