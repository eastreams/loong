use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

use super::message::{Message, MessagePart, Role, ToolStatus};
use super::theme::Palette;

// ---------------------------------------------------------------------------
// View trait — decouples rendering from the concrete `Pane` struct
// ---------------------------------------------------------------------------

pub(super) trait PaneView {
    fn messages(&self) -> &[Message];
    fn scroll_offset(&self) -> u16;
    fn streaming_active(&self) -> bool;
}

const JUMP_TO_LATEST_HINT: &str = " End latest ";

// ---------------------------------------------------------------------------
// Top-level render entry point
// ---------------------------------------------------------------------------

pub(super) fn render_history(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &impl PaneView,
    palette: &Palette,
    show_thinking: bool,
) {
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    let show_welcome = pane.messages().is_empty()
        || (pane.messages().len() == 1
            && pane
                .messages()
                .first()
                .is_some_and(|m| m.role == Role::User));
    if show_welcome {
        lines.extend(render_welcome(width, palette));
    }

    for msg in pane.messages() {
        lines.extend(render_message(msg, width, show_thinking, palette));
        lines.push(Line::default()); // gap between messages
    }

    // Show cursor indicator on the last part of the current assistant message
    // when streaming is active.
    if pane.streaming_active()
        && let Some(last_msg) = pane.messages().last()
        && last_msg.role == Role::Assistant
        && let Some(last_line) = lines.last_mut()
    {
        last_line.spans.push(Span::styled(
            "\u{2588}",
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
    }

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
                    format!("  {args_preview}"),
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
                render_history(f, f.area(), &pane, &palette, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("LoongClaw"),
            "welcome banner should contain LoongClaw"
        );
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
                render_history(f, f.area(), &pane, &palette, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);
        assert!(text.contains("You"), "should show You badge");
        assert!(text.contains("hello world"), "should show message text");
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
                render_history(f, f.area(), &pane, &palette, false);
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
                render_history(f, f.area(), &pane, &palette, false);
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
                render_history(f, f.area(), &pane, &palette, false);
            })
            .expect("draw failed");

        let text = buffer_text(&terminal);

        assert!(
            !text.contains("End latest"),
            "tail-follow history should not show the jump-to-latest hint: {text:?}"
        );
    }
}
