use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

use super::message::{Message, MessagePart, Role, ToolStatus, format_tool_args_preview};
use super::theme::Palette;

// ---------------------------------------------------------------------------
// View trait — decouples rendering from the concrete `Pane` struct
// ---------------------------------------------------------------------------

pub(super) trait PaneView {
    fn messages(&self) -> &[Message];
    fn scroll_offset(&self) -> u16;
    fn streaming_active(&self) -> bool;
    fn transcript_cursor_line(&self, _total_lines: usize) -> Option<usize> {
        None
    }
    fn transcript_selection_range(&self, _total_lines: usize) -> Option<(usize, usize)> {
        None
    }
}

const JUMP_TO_LATEST_HINT: &str = " End latest ";

#[derive(Debug, Clone)]
pub(super) struct TranscriptDocument {
    pub(super) styled_lines: Vec<Line<'static>>,
    pub(super) plain_lines: Vec<String>,
}

// ---------------------------------------------------------------------------
// Top-level render entry point
// ---------------------------------------------------------------------------

pub(super) fn render_history(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &impl PaneView,
    palette: &Palette,
    show_thinking: bool,
    show_transcript_cursor: bool,
) {
    let width = area.width as usize;
    let document = build_transcript_document(pane, width, show_thinking, palette);
    let total_document_lines = document.plain_lines.len();
    let transcript_cursor_line = if show_transcript_cursor {
        pane.transcript_cursor_line(total_document_lines)
    } else {
        None
    };
    let transcript_selection_range = if show_transcript_cursor {
        pane.transcript_selection_range(total_document_lines)
    } else {
        None
    };
    let lines = decorate_transcript_lines(
        document.styled_lines,
        transcript_cursor_line,
        transcript_selection_range,
        palette,
    );

    // Ask ratatui for the exact wrapped line count (requires the
    // `unstable-rendered-line-info` feature on ratatui 0.29).  Manual
    // width÷viewport math diverges from Paragraph's internal wrapping
    // and causes auto-scroll to stop short of the true bottom.
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total_lines = para.line_count(area.width) as u16;

    let visible = area.height;
    let max_scroll = total_lines.saturating_sub(visible);

    // scroll_offset == 0 means "follow tail" (auto-scroll to bottom).
    let scroll = if pane.scroll_offset() == 0 {
        max_scroll
    } else {
        max_scroll.saturating_sub(pane.scroll_offset())
    };

    let para = para.scroll((scroll, 0));

    frame.render_widget(para, area);

    // Scrollbar when content exceeds viewport.
    if total_lines > visible {
        let mut sb_state = ScrollbarState::new(total_lines as usize).position(scroll as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(palette.dim));
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin {
                horizontal: 0,
                vertical: 0,
            }),
            &mut sb_state,
        );
    }

    let show_jump_to_latest_hint =
        total_lines > visible && pane.scroll_offset() > 0 && area.height > 0;
    if show_jump_to_latest_hint {
        render_jump_to_latest_hint(frame, area, palette);
    }
}

pub(super) fn transcript_plain_lines(
    pane: &impl PaneView,
    width: usize,
    show_thinking: bool,
) -> Vec<String> {
    let palette = Palette::plain();
    let document = build_transcript_document(pane, width, show_thinking, &palette);

    document.plain_lines
}

pub(super) fn viewport_plain_line_at(
    pane: &impl PaneView,
    width: u16,
    height: u16,
    viewport_row: u16,
    show_thinking: bool,
) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }

    let plain_lines = transcript_plain_lines(pane, usize::from(width), show_thinking);
    let wrapped_line_to_plain_line =
        wrapped_line_to_plain_line_map(plain_lines.as_slice(), usize::from(width));
    if wrapped_line_to_plain_line.is_empty() {
        return None;
    }

    let total_wrapped_lines = wrapped_line_to_plain_line.len() as u16;
    let max_scroll = total_wrapped_lines.saturating_sub(height);
    let scroll = if pane.scroll_offset() == 0 {
        max_scroll
    } else {
        max_scroll.saturating_sub(pane.scroll_offset())
    };
    let clamped_row = viewport_row.min(height.saturating_sub(1));
    let absolute_row = usize::from(scroll.saturating_add(clamped_row));

    wrapped_line_to_plain_line.get(absolute_row).copied()
}

fn wrapped_line_to_plain_line_map(plain_lines: &[String], width: usize) -> Vec<usize> {
    let effective_width = width.max(1);
    let mut wrapped_lines = Vec::new();

    for (plain_line_index, plain_line) in plain_lines.iter().enumerate() {
        let char_count = plain_line.chars().count();
        let wrapped_line_count = wrapped_line_count(char_count, effective_width);

        for _ in 0..wrapped_line_count {
            wrapped_lines.push(plain_line_index);
        }
    }

    wrapped_lines
}

fn wrapped_line_count(char_count: usize, width: usize) -> usize {
    if char_count == 0 {
        return 1;
    }

    let full_rows = char_count / width;
    let has_partial_row = !char_count.is_multiple_of(width);
    let partial_row_count = usize::from(has_partial_row);

    full_rows + partial_row_count
}

fn render_jump_to_latest_hint(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let hint_width = JUMP_TO_LATEST_HINT.chars().count() as u16;
    let required_width = hint_width.saturating_add(2);
    if area.width < required_width {
        return;
    }

    let hint_x = area.x + area.width.saturating_sub(required_width);
    let hint_y = area.y + area.height.saturating_sub(1);
    let hint_area = Rect::new(hint_x, hint_y, hint_width, 1);
    let hint_line = Line::styled(
        JUMP_TO_LATEST_HINT.to_owned(),
        Style::default()
            .fg(palette.warning)
            .add_modifier(Modifier::BOLD),
    );
    let hint_widget = Paragraph::new(hint_line);

    frame.render_widget(hint_widget, hint_area);
}

fn build_transcript_document(
    pane: &impl PaneView,
    width: usize,
    show_thinking: bool,
    palette: &Palette,
) -> TranscriptDocument {
    let mut styled_lines: Vec<Line<'static>> = Vec::new();

    let show_welcome = pane.messages().is_empty()
        || (pane.messages().len() == 1
            && pane
                .messages()
                .first()
                .is_some_and(|m| m.role == Role::User));
    if show_welcome {
        styled_lines.extend(render_welcome(width, palette));
    }

    for msg in pane.messages() {
        styled_lines.extend(render_message(msg, width, show_thinking, palette));
        styled_lines.push(Line::default());
    }

    if pane.streaming_active()
        && let Some(last_msg) = pane.messages().last()
        && last_msg.role == Role::Assistant
        && let Some(last_line) = styled_lines.last_mut()
    {
        last_line.spans.push(Span::styled(
            "\u{2588}",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
    }

    let plain_lines = styled_lines
        .iter()
        .map(transcript_plain_text_for_line)
        .collect();

    TranscriptDocument {
        styled_lines,
        plain_lines,
    }
}

fn decorate_transcript_lines(
    mut styled_lines: Vec<Line<'static>>,
    transcript_cursor_line: Option<usize>,
    transcript_selection_range: Option<(usize, usize)>,
    palette: &Palette,
) -> Vec<Line<'static>> {
    for (line_index, line) in styled_lines.iter_mut().enumerate() {
        let is_selected = transcript_selection_range.is_some_and(|(range_start, range_end)| {
            line_index >= range_start && line_index <= range_end
        });
        if is_selected {
            prepend_transcript_marker(
                line,
                "\u{258c} ",
                Style::default()
                    .fg(palette.warning)
                    .add_modifier(Modifier::BOLD),
            );
            continue;
        }

        let is_cursor_line = transcript_cursor_line == Some(line_index);
        if is_cursor_line {
            prepend_transcript_marker(
                line,
                "\u{258e} ",
                Style::default()
                    .fg(palette.info)
                    .add_modifier(Modifier::BOLD),
            );
        }
    }

    styled_lines
}

fn prepend_transcript_marker(line: &mut Line<'static>, marker: &str, marker_style: Style) {
    if let Some(first_span) = line.spans.first_mut() {
        let original_content = first_span.content.to_string();
        if let Some(stripped_content) = original_content.strip_prefix("  ") {
            first_span.content = stripped_content.to_owned().into();
        }
    }

    line.spans
        .insert(0, Span::styled(marker.to_owned(), marker_style));
}

fn transcript_plain_text_for_line(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Per-message rendering
// ---------------------------------------------------------------------------

fn render_message(
    msg: &Message,
    width: usize,
    show_thinking: bool,
    palette: &Palette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    match msg.role {
        Role::User => {
            // " You " badge -- white text on user_msg background
            lines.push(Line::styled(
                " You ".to_string(),
                Style::default()
                    .fg(ratatui::style::Color::White)
                    .bg(palette.user_msg)
                    .add_modifier(Modifier::BOLD),
            ));
            for part in &msg.parts {
                if let MessagePart::Text(text) = part {
                    for line_str in text.lines() {
                        lines.push(Line::styled(
                            format!("  {line_str}"),
                            Style::default().fg(palette.text),
                        ));
                    }
                }
            }
        }
        Role::Assistant => {
            // Top divider: "── LoongClaw ──…"
            let label = " LoongClaw ";
            let remaining = width.saturating_sub(label.len() + 4);
            lines.push(Line::from(vec![
                Span::styled(
                    "\u{2500}\u{2500}".to_string(),
                    Style::default().fg(palette.brand),
                ),
                Span::styled(
                    label.to_string(),
                    Style::default()
                        .fg(palette.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "\u{2500}".repeat(remaining),
                    Style::default().fg(palette.brand),
                ),
            ]));

            for part in &msg.parts {
                match part {
                    MessagePart::Text(text) => {
                        lines.extend(render_markdown(text, palette));
                    }
                    MessagePart::ThinkBlock(text) => {
                        if show_thinking {
                            lines.push(Line::styled(
                                "  ~ thinking ~".to_string(),
                                Style::default()
                                    .fg(palette.think_block)
                                    .add_modifier(Modifier::ITALIC),
                            ));
                            for line_str in text.lines() {
                                lines.push(Line::styled(
                                    format!("    {line_str}"),
                                    Style::default()
                                        .fg(palette.think_block)
                                        .add_modifier(Modifier::DIM | Modifier::ITALIC),
                                ));
                            }
                        } else {
                            lines.push(Line::styled(
                                "  [... thinking ...]".to_string(),
                                Style::default()
                                    .fg(palette.think_block)
                                    .add_modifier(Modifier::DIM),
                            ));
                        }
                    }
                    MessagePart::ToolCall {
                        tool_name,
                        args_preview,
                        status,
                        ..
                    } => {
                        lines.push(render_tool_call_line(
                            tool_name,
                            args_preview,
                            status,
                            palette,
                        ));
                    }
                }
            }

            // Bottom divider
            lines.push(Line::styled(
                "\u{2500}".repeat(width),
                Style::default().fg(palette.brand),
            ));
        }
        Role::System => {
            for part in &msg.parts {
                if let MessagePart::Text(text) = part {
                    for line_str in text.lines() {
                        lines.push(Line::styled(
                            format!("  {line_str}"),
                            Style::default()
                                .fg(palette.dim)
                                .add_modifier(Modifier::ITALIC),
                        ));
                    }
                }
            }
        }
        Role::Surface => {
            for part in &msg.parts {
                if let MessagePart::Text(text) = part {
                    for line_str in text.lines() {
                        lines.push(Line::styled(
                            line_str.to_owned(),
                            Style::default().fg(palette.text),
                        ));
                    }
                }
            }
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Tool call status line (hermes-lite style)
// ---------------------------------------------------------------------------

fn render_tool_call_line<'a>(
    tool_name: &str,
    args_preview: &str,
    status: &ToolStatus,
    palette: &Palette,
) -> Line<'a> {
    let summarized_args_preview = format_tool_args_preview(tool_name, args_preview);

    match status {
        ToolStatus::Running { started } => {
            let elapsed = started.elapsed().as_secs_f32();
            Line::from(vec![
                Span::styled("  | ".to_string(), Style::default().fg(palette.dim)),
                Span::styled("* ".to_string(), Style::default().fg(palette.tool_running)),
                Span::styled(
                    format!("{tool_name:<12}"),
                    Style::default()
                        .fg(palette.tool_running)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {summarized_args_preview}"),
                    Style::default().fg(palette.dim),
                ),
                Span::styled(
                    format!("  ({elapsed:.1}s)"),
                    Style::default().fg(palette.dim),
                ),
            ])
        }
        ToolStatus::Done {
            success,
            output,
            duration_ms,
        } => {
            let (icon, color) = if *success {
                ("v", palette.tool_done)
            } else {
                ("x", palette.tool_fail)
            };
            let dur = *duration_ms as f32 / 1000.0;
            let preview = truncate_output(output, 40);
            let truncated = preview.ends_with('\u{2026}');
            let mut spans = vec![
                Span::styled("  | ".to_string(), Style::default().fg(palette.dim)),
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(
                    format!("{tool_name:<12}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {preview}"), Style::default().fg(palette.dim)),
            ];
            if truncated {
                spans.push(Span::styled(
                    " [...]".to_string(),
                    Style::default().fg(palette.dim).add_modifier(Modifier::DIM),
                ));
            }
            spans.push(Span::styled(
                format!("  ({dur:.1}s)"),
                Style::default().fg(palette.dim),
            ));
            Line::from(spans)
        }
    }
}

/// Truncate output to `max_chars`, appending an ellipsis if shortened.
fn truncate_output(text: &str, max_chars: usize) -> String {
    let first_line = text.lines().next().unwrap_or("");
    let char_count = first_line.chars().count();
    if char_count <= max_chars {
        first_line.to_string()
    } else {
        let end = first_line
            .char_indices()
            .nth(max_chars.saturating_sub(1))
            .map_or(first_line.len(), |(i, _)| i);
        let truncated = first_line.get(..end).unwrap_or(first_line);
        format!("{truncated}\u{2026}")
    }
}

// ---------------------------------------------------------------------------
// Welcome screen
// ---------------------------------------------------------------------------

fn render_welcome(width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::default());

    let bar_len = 47usize.min(width.saturating_sub(2));
    let bar: String = "\u{2500}".repeat(bar_len);
    let title = "LoongClaw  -  AI Agent";
    let padded_title = format!("| {:^w$} |", title, w = bar_len.saturating_sub(4));

    for bl in [format!("+{bar}+"), padded_title, format!("+{bar}+")] {
        let centered = format!("{bl:^width$}", width = width);
        lines.push(Line::styled(
            centered,
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ));
    }

    lines.push(Line::default());
    lines.push(Line::styled(
        "  Type a message to begin, or /help for commands.".to_string(),
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::DIM),
    ));
    lines.push(Line::default());
    lines
}

// ---------------------------------------------------------------------------
// Markdown → ratatui Lines
// ---------------------------------------------------------------------------

/// Render a markdown string into indented, styled `Line`s using `pulldown-cmark`.
///
/// Strips raw markdown syntax and applies terminal-friendly formatting:
/// - `# Header` → bold + brand colour (no `#` prefix)
/// - `**bold**` → `Modifier::BOLD`
/// - `*italic*` → `Modifier::ITALIC`
/// - `` `code` `` → dim colour
/// - `- item` → `  • item` (bullet)
/// - `> quote` → `  │ ` prefix + italic
/// - Code blocks → dim + indent
#[allow(clippy::wildcard_enum_match_arm)]
fn render_markdown(src: &str, palette: &Palette) -> Vec<Line<'static>> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

    let parser = Parser::new_ext(src, Options::ENABLE_STRIKETHROUGH);

    let text_style = Style::default().fg(palette.text);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![text_style];
    let mut in_code_block = false;
    let mut list_depth: usize = 0;
    let mut in_blockquote = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { .. } => {
                    style_stack.push(
                        Style::default()
                            .fg(palette.brand)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                Tag::Emphasis => {
                    let base = *style_stack.last().unwrap_or(&text_style);
                    style_stack.push(base.add_modifier(Modifier::ITALIC));
                }
                Tag::Strong => {
                    let base = *style_stack.last().unwrap_or(&text_style);
                    style_stack.push(base.add_modifier(Modifier::BOLD));
                }
                Tag::Strikethrough => {
                    let base = *style_stack.last().unwrap_or(&text_style);
                    style_stack.push(base.add_modifier(Modifier::CROSSED_OUT));
                }
                Tag::CodeBlock(_) => {
                    in_code_block = true;
                    md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                }
                Tag::List(_) => {
                    list_depth += 1;
                }
                Tag::Item => {
                    md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                    spans.push(Span::styled("• ", Style::default().fg(palette.dim)));
                }
                Tag::BlockQuote(_) => {
                    in_blockquote = true;
                }
                Tag::Link { .. } => {
                    style_stack.push(
                        Style::default()
                            .fg(palette.info)
                            .add_modifier(Modifier::UNDERLINED),
                    );
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    style_stack.pop();
                    md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                    out.push(Line::default());
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    if list_depth == 0 {
                        out.push(Line::default());
                    }
                }
                TagEnd::Item => {
                    md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                }
                TagEnd::BlockQuote(_) => {
                    in_blockquote = false;
                }
                TagEnd::Paragraph => {
                    md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                    out.push(Line::default());
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    let s = Style::default().fg(palette.dim);
                    for line_str in text.as_ref().lines() {
                        spans.push(Span::styled(format!("    {line_str}"), s));
                        md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                    }
                } else {
                    let s = *style_stack.last().unwrap_or(&text_style);
                    spans.push(Span::styled(text.into_string(), s));
                }
            }
            Event::Code(code) => {
                spans.push(Span::styled(
                    format!("`{code}`"),
                    Style::default().fg(palette.dim),
                ));
            }
            Event::SoftBreak => spans.push(Span::raw(" ")),
            Event::HardBreak => md_flush(&mut spans, &mut out, in_blockquote, list_depth),
            Event::Rule => {
                md_flush(&mut spans, &mut out, in_blockquote, list_depth);
                out.push(Line::styled(
                    "  ────────────────────────────────────────",
                    Style::default().fg(palette.separator),
                ));
            }
            _ => {}
        }
    }

    md_flush(&mut spans, &mut out, in_blockquote, list_depth);
    out
}

/// Flush accumulated spans into a single indented `Line`.
fn md_flush(
    spans: &mut Vec<Span<'static>>,
    out: &mut Vec<Line<'static>>,
    in_blockquote: bool,
    list_depth: usize,
) {
    if spans.is_empty() {
        return;
    }
    let indent = if in_blockquote {
        "  │ ".to_string()
    } else if list_depth > 0 {
        format!("{}  ", "  ".repeat(list_depth.saturating_sub(1)))
    } else {
        "  ".to_string()
    };
    let mut line_spans = vec![Span::raw(indent)];
    line_spans.append(spans);
    out.push(Line::from(line_spans));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::time::Instant;

    // Minimal PaneView impl for testing.
    struct TestPane {
        messages: Vec<Message>,
        scroll_offset: u16,
        streaming_active: bool,
    }

    impl TestPane {
        fn empty() -> Self {
            Self {
                messages: Vec::new(),
                scroll_offset: 0,
                streaming_active: false,
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

    #[test]
    fn empty_history_shows_welcome() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("terminal creation failed");
        let pane = TestPane::empty();
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_history(f, f.area(), &pane, &palette, false, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("LoongClaw"),
            "welcome banner should contain LoongClaw"
        );
    }

    #[test]
    fn transcript_document_plain_lines_preserve_visible_text() {
        let pane = TestPane {
            messages: vec![Message::user("hello world")],
            ..TestPane::empty()
        };
        let lines = transcript_plain_lines(&pane, 60, false);

        assert!(
            lines.iter().any(|line| line.contains("hello world")),
            "plain transcript lines should preserve rendered user text"
        );
    }

    #[test]
    fn selected_transcript_lines_show_selection_marker() {
        let pane = TestPane {
            messages: vec![Message::user("line one\nline two")],
            scroll_offset: 0,
            streaming_active: false,
        };
        let palette = Palette::dark();

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).expect("terminal creation failed");

        struct SelectionPane<'a> {
            inner: &'a TestPane,
        }

        impl PaneView for SelectionPane<'_> {
            fn messages(&self) -> &[Message] {
                self.inner.messages()
            }
            fn scroll_offset(&self) -> u16 {
                self.inner.scroll_offset()
            }
            fn streaming_active(&self) -> bool {
                self.inner.streaming_active()
            }
            fn transcript_cursor_line(&self, _total_lines: usize) -> Option<usize> {
                Some(1)
            }
            fn transcript_selection_range(&self, _total_lines: usize) -> Option<(usize, usize)> {
                Some((1, 2))
            }
        }

        let selection_pane = SelectionPane { inner: &pane };

        terminal
            .draw(|f| {
                render_history(f, f.area(), &selection_pane, &palette, false, true);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("\u{258c}"),
            "selected transcript lines should render a selection marker: {text:?}"
        );
    }

    #[test]
    fn viewport_plain_line_at_maps_scrolled_rows_back_to_plain_lines() {
        let pane = TestPane {
            messages: vec![Message::user(
                "first line\nsecond line that wraps a bit\nthird line",
            )],
            scroll_offset: 1,
            streaming_active: false,
        };

        let mapped_line = viewport_plain_line_at(&pane, 18, 4, 1, false);

        assert!(mapped_line.is_some());
    }

    #[test]
    fn user_message_renders_badge() {
        let pane = TestPane {
            messages: vec![Message::user("hello world")],
            ..TestPane::empty()
        };
        let palette = Palette::dark();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("terminal creation failed");

        terminal
            .draw(|f| {
                render_history(f, f.area(), &pane, &palette, false, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);
        assert!(text.contains("You"), "should show You badge");
        assert!(text.contains("hello world"), "should show message text");
    }

    #[test]
    fn transcript_cursor_line_shows_cursor_marker() {
        let pane = TestPane {
            messages: vec![Message::user("cursor line")],
            ..TestPane::empty()
        };
        let palette = Palette::dark();

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).expect("terminal creation failed");

        struct CursorPane<'a> {
            inner: &'a TestPane,
        }

        impl PaneView for CursorPane<'_> {
            fn messages(&self) -> &[Message] {
                self.inner.messages()
            }
            fn scroll_offset(&self) -> u16 {
                self.inner.scroll_offset()
            }
            fn streaming_active(&self) -> bool {
                self.inner.streaming_active()
            }
            fn transcript_cursor_line(&self, _total_lines: usize) -> Option<usize> {
                Some(1)
            }
        }

        let cursor_pane = CursorPane { inner: &pane };

        terminal
            .draw(|f| {
                render_history(f, f.area(), &cursor_pane, &palette, false, true);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);
        assert!(text.contains("You"), "should show You badge");
        assert!(text.contains("\u{258e}"), "cursor marker should be visible");
        assert!(
            text.contains("cursor line"),
            "message text should still be visible"
        );
    }

    #[test]
    fn assistant_message_renders_divider() {
        let mut msg = Message::assistant();
        msg.parts.push(MessagePart::Text("reply text".into()));

        let pane = TestPane {
            messages: vec![msg],
            ..TestPane::empty()
        };
        let palette = Palette::dark();

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("terminal creation failed");

        terminal
            .draw(|f| {
                render_history(f, f.area(), &pane, &palette, false, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);
        assert!(text.contains("LoongClaw"), "should show divider with name");
        assert!(text.contains("reply text"), "should show reply text");
    }

    #[test]
    fn tool_call_running_format() {
        let palette = Palette::dark();
        let line = render_tool_call_line(
            "read_file",
            "src/main.rs",
            &ToolStatus::Running {
                started: Instant::now(),
            },
            &palette,
        );
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("read_file"), "should contain tool name");
        assert!(text.contains("src/main.rs"), "should contain args preview");
        assert!(text.contains("s)"), "should contain elapsed time");
    }

    #[test]
    fn tool_call_done_success_format() {
        let palette = Palette::dark();
        let line = render_tool_call_line(
            "bash",
            "ls -la",
            &ToolStatus::Done {
                success: true,
                output: "file1.rs\nfile2.rs".into(),
                duration_ms: 800,
            },
            &palette,
        );
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("v "), "success should show v icon");
        assert!(text.contains("bash"), "should contain tool name");
        assert!(text.contains("(0.8s)"), "should show duration");
    }

    #[test]
    fn tool_call_done_fail_format() {
        let palette = Palette::dark();
        let line = render_tool_call_line(
            "write_file",
            "/tmp/out",
            &ToolStatus::Done {
                success: false,
                output: "permission denied".into(),
                duration_ms: 2100,
            },
            &palette,
        );
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("x "), "failure should show x icon");
        assert!(text.contains("(2.1s)"), "should show duration");
    }

    #[test]
    fn truncate_output_short_unchanged() {
        assert_eq!(truncate_output("short", 40), "short");
    }

    #[test]
    fn truncate_output_long_ellipsis() {
        let long = "a".repeat(60);
        let result = truncate_output(&long, 40);
        assert!(result.ends_with('\u{2026}'));
        assert_eq!(result.chars().count(), 40);
    }

    #[test]
    fn think_block_collapsed_when_hidden() {
        let mut msg = Message::assistant();
        msg.parts
            .push(MessagePart::ThinkBlock("deep thought".into()));

        let palette = Palette::dark();
        let lines = render_message(&msg, 60, false, &palette);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(text.contains("[... thinking ...]"), "collapsed think block");
        assert!(!text.contains("deep thought"), "content should be hidden");
    }

    #[test]
    fn think_block_expanded_when_shown() {
        let mut msg = Message::assistant();
        msg.parts
            .push(MessagePart::ThinkBlock("deep thought".into()));

        let palette = Palette::dark();
        let lines = render_message(&msg, 60, true, &palette);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(text.contains("~ thinking ~"), "header present");
        assert!(text.contains("deep thought"), "content visible");
    }

    #[test]
    fn scrolled_history_shows_jump_to_latest_hint() {
        let mut long_lines = Vec::new();
        for index in 0..40 {
            long_lines.push(format!("line {index}"));
        }

        let surface_message = Message::surface(long_lines.join("\n"));
        let pane = TestPane {
            messages: vec![surface_message],
            scroll_offset: 6,
            streaming_active: false,
        };
        let palette = Palette::dark();

        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).expect("terminal creation failed");

        terminal
            .draw(|f| {
                render_history(f, f.area(), &pane, &palette, false, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("End latest"),
            "scrolled history should show a jump-to-latest hint: {text:?}"
        );
    }

    #[test]
    fn tail_follow_history_hides_jump_to_latest_hint() {
        let mut long_lines = Vec::new();
        for index in 0..40 {
            long_lines.push(format!("line {index}"));
        }

        let surface_message = Message::surface(long_lines.join("\n"));
        let pane = TestPane {
            messages: vec![surface_message],
            scroll_offset: 0,
            streaming_active: false,
        };
        let palette = Palette::dark();

        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).expect("terminal creation failed");

        terminal
            .draw(|f| {
                render_history(f, f.area(), &pane, &palette, false, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);

        assert!(
            !text.contains("End latest"),
            "tail-follow history should not show the jump-to-latest hint: {text:?}"
        );
    }
}
