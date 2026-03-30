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
    fn streaming_text(&self) -> &str;
    fn is_thinking(&self) -> bool;
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
) {
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if pane.messages().is_empty() {
        lines.extend(render_welcome(width, palette));
    }

    for msg in pane.messages() {
        lines.extend(render_message(msg, width, show_thinking, palette));
        lines.push(Line::default()); // gap between messages
    }

    // Streaming text (not yet finalized into a message)
    let streaming = pane.streaming_text();
    if !streaming.is_empty() {
        let style = if pane.is_thinking() {
            Style::default()
                .fg(palette.think_block)
                .add_modifier(Modifier::ITALIC | Modifier::DIM)
        } else {
            Style::default().fg(palette.text)
        };
        for line_str in streaming.lines() {
            lines.push(Line::styled(line_str.to_string(), style));
        }
    }

    // Count wrapped visual rows for scroll math.
    let wrap_width = area.width as usize;
    let total_lines: u16 = lines
        .iter()
        .map(|line| {
            let w = line.width();
            if w == 0 || wrap_width == 0 {
                1u16
            } else {
                (w as u16).saturating_sub(1) / (wrap_width as u16).max(1) + 1
            }
        })
        .sum();

    let visible = area.height;
    let max_scroll = total_lines.saturating_sub(visible);

    // scroll_offset == 0 means "follow tail" (auto-scroll to bottom).
    let scroll = if pane.scroll_offset() == 0 {
        max_scroll
    } else {
        max_scroll.saturating_sub(pane.scroll_offset())
    };

    let para = Paragraph::new(lines)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });

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
                        for line_str in text.lines() {
                            lines.push(Line::styled(
                                format!("  {line_str}"),
                                Style::default().fg(palette.text),
                            ));
                        }
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
            Line::from(vec![
                Span::styled("  | ".to_string(), Style::default().fg(palette.dim)),
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(
                    format!("{tool_name:<12}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {preview}"), Style::default().fg(palette.dim)),
                Span::styled(format!("  ({dur:.1}s)"), Style::default().fg(palette.dim)),
            ])
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
        streaming_text: String,
        is_thinking: bool,
    }

    impl TestPane {
        fn empty() -> Self {
            Self {
                messages: Vec::new(),
                scroll_offset: 0,
                streaming_text: String::new(),
                is_thinking: false,
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
        fn streaming_text(&self) -> &str {
            &self.streaming_text
        }
        fn is_thinking(&self) -> bool {
            self.is_thinking
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
}
