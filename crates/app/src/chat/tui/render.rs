use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
        block::Position as TitlePosition,
    },
};

use super::dialog::ClarifyDialog;
use super::focus::{FocusLayer, FocusStack};
use super::history::{self, PaneView};
use super::input::{self, InputView};
use super::layout;
use super::message::{ToolStatus, format_tool_args_preview};
use super::spinner::{self, SpinnerView};
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
    let input_height = textarea.lines().len() as u16 + 2; // +2 for borders
    let areas = layout::compute(area, input_height);

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

    if state.focus().has(FocusLayer::Help) {
        render_help_overlay(frame, area, palette);
    }
}

// ---------------------------------------------------------------------------
// Separator
// ---------------------------------------------------------------------------

fn render_separator(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let sep = Paragraph::new(Line::styled(
        "\u{2500}".repeat(area.width as usize),
        Style::default().fg(palette.separator),
    ));
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
        .border_style(Style::default().fg(palette.warning))
        .title(Span::styled(
            " Agent Question ",
            Style::default()
                .fg(palette.brand)
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
        .style(Style::default().bg(Color::Rgb(0x1a, 0x1a, 0x1a)));

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
    let help_items: &[(&str, &[(&str, &str)])] = &[
        (
            "General",
            &[
                ("/help", "Toggle this help"),
                ("/clear", "Clear conversation"),
                ("/model", "Show current model"),
                ("/review", "Toggle transcript review"),
                ("/think-on", "Show thinking blocks"),
                ("/think-off", "Hide thinking blocks"),
                ("/exit", "Exit the TUI"),
            ],
        ),
        (
            "Shortcuts",
            &[
                ("Enter", "Send message"),
                ("Shift+Enter", "New line"),
                ("Up/Down", "Scroll history when empty"),
                ("PageUp/Dn", "Page scroll history"),
                ("Home/End", "Jump top/latest when empty"),
                ("Ctrl+R", "Toggle transcript review"),
                ("Ctrl+O", "Open latest tool details"),
                ("Ctrl+C", "Interrupt / cancel"),
                ("Esc", "Close dialogs"),
            ],
        ),
    ];

    let mut content_lines: Vec<Line<'_>> = Vec::new();
    for (section, items) in help_items {
        if !content_lines.is_empty() {
            content_lines.push(Line::default());
        }
        content_lines.push(Line::styled(
            format!(" {section}"),
            Style::default()
                .fg(palette.brand)
                .add_modifier(Modifier::BOLD),
        ));
        for (cmd, desc) in *items {
            content_lines.push(Line::from(vec![
                Span::styled(format!("  {cmd:<16}"), Style::default().fg(palette.text)),
                Span::styled(format!(" {desc}"), Style::default().fg(palette.dim)),
            ]));
        }
    }

    let popup_width = 60u16.min(area.width.saturating_sub(4));
    let popup_height = (content_lines.len() as u16 + 2).min(area.height.saturating_sub(2));

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.brand))
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
        .style(Style::default().bg(Color::Rgb(0x1a, 0x1a, 0x1a)));

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
        .border_style(Style::default().fg(palette.info))
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
        .style(Style::default().bg(Color::Rgb(0x1a, 0x1a, 0x1a)));

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

    for output_line in output_text.lines() {
        let prefixed_line = format!(" {output_line}");
        let line = Line::styled(prefixed_line, Style::default().fg(palette.text));
        content_lines.push(line);
    }

    if output_text.is_empty() {
        let empty_line = Line::styled(" ", Style::default().fg(palette.text));
        content_lines.push(empty_line);
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
    use crate::chat::tui::dialog::ClarifyDialog;
    use crate::chat::tui::message::{Message, ToolStatus};
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
    }

    impl InputView for TestPane {
        fn agent_running(&self) -> bool {
            self.agent_running
        }
        fn has_staged_message(&self) -> bool {
            false
        }
    }

    struct TestShell {
        pane: TestPane,
        show_thinking: bool,
        focus: FocusStack,
        clarify_dialog: Option<ClarifyDialog>,
        tool_inspector: Option<TestToolInspector>,
    }

    impl TestShell {
        fn idle() -> Self {
            Self {
                pane: TestPane::default_idle(),
                show_thinking: false,
                focus: FocusStack::new(),
                clarify_dialog: None,
                tool_inspector: None,
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
    fn separator_renders_horizontal_rule() {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_separator(f, f.area(), &palette);
            })
            .expect("draw");

        let buf = terminal.backend().buffer().clone();
        for x in 0..20 {
            let sym = buf.cell((x, 0)).map_or("", |c| c.symbol());
            assert_eq!(sym, "\u{2500}", "separator should be horizontal rule");
        }
    }
}
