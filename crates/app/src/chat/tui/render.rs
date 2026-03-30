use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap, block::Position as TitlePosition},
};

use super::dialog::ClarifyDialog;
use super::history::{self, PaneView};
use super::input::{self, InputView};
use super::layout;
use super::spinner::{self, SpinnerView};
use super::status_bar::{self, StatusBarView};
use super::theme::Palette;

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
    fn show_help(&self) -> bool;
    fn clarify_dialog(&self) -> Option<&ClarifyDialog>;
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
    );

    // 2. First separator
    render_separator(frame, areas.separator1, palette);

    // 3. Spinner / phase line
    spinner::render_spinner(frame, areas.spinner, state.pane(), palette);

    // 4. Second separator
    render_separator(frame, areas.separator2, palette);

    // 5. Input area
    input::render_input(frame, areas.input, textarea, state.pane(), palette);

    // 6. Status bar
    status_bar::render_status_bar(frame, areas.status_bar, state.pane(), palette);

    // 7. Overlays (rendered last, on top of everything)
    if let Some(dialog) = state.clarify_dialog() {
        render_clarify_dialog(dialog, frame, area, palette);
    }

    if state.show_help() {
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
                ("/think-on", "Show thinking blocks"),
                ("/think-off", "Hide thinking blocks"),
                ("/exit", "Exit the TUI"),
            ],
        ),
        (
            "Shortcuts",
            &[
                ("Enter", "Send message"),
                ("Ctrl+C", "Interrupt / cancel"),
                ("PageUp/Dn", "Scroll history"),
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

    let popup_width = 46u16.min(area.width.saturating_sub(4));
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::tui::dialog::ClarifyDialog;
    use crate::chat::tui::message::Message;
    use ratatui::{Terminal, backend::TestBackend};
    use std::time::Instant;

    // Unified test pane implementing all view traits.
    struct TestPane {
        messages: Vec<Message>,
        scroll_offset: u16,
        streaming_text: String,
        is_thinking: bool,
        agent_running: bool,
        spinner_frame: usize,
        dots_frame: usize,
        loop_state: String,
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
                streaming_text: String::new(),
                is_thinking: false,
                agent_running: false,
                spinner_frame: 0,
                dots_frame: 0,
                loop_state: String::new(),
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
        fn streaming_text(&self) -> &str {
            &self.streaming_text
        }
        fn is_thinking(&self) -> bool {
            self.is_thinking
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
    }

    struct TestShell {
        pane: TestPane,
        show_thinking: bool,
        show_help: bool,
        clarify_dialog: Option<ClarifyDialog>,
    }

    impl TestShell {
        fn idle() -> Self {
            Self {
                pane: TestPane::default_idle(),
                show_thinking: false,
                show_help: false,
                clarify_dialog: None,
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
        fn show_help(&self) -> bool {
            self.show_help
        }
        fn clarify_dialog(&self) -> Option<&ClarifyDialog> {
            self.clarify_dialog.as_ref()
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
        let shell = TestShell {
            show_help: true,
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
    }

    #[test]
    fn draw_with_clarify_dialog() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let shell = TestShell {
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
