use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use super::focus::FocusLayer;
use super::state::BusyInputMode;
use super::theme::Palette;

// ---------------------------------------------------------------------------
// View trait — decouples rendering from the concrete `Pane` struct
// ---------------------------------------------------------------------------

pub(super) trait InputView {
    fn agent_running(&self) -> bool;
    fn pending_submission_count(&self) -> usize;
    fn busy_input_mode(&self) -> BusyInputMode;
    fn transcript_selection_line_count(&self) -> usize {
        0
    }
    fn input_hint(&self) -> Option<&str> {
        None
    }
    fn input_placeholder(&self) -> Option<String> {
        None
    }
}

fn textarea_is_empty(textarea: &tui_textarea::TextArea<'_>) -> bool {
    textarea.lines().iter().all(|line| line.is_empty())
}

fn wrapped_visual_line_count(line: &str, content_width: usize) -> u16 {
    let clamped_content_width = content_width.max(1);
    let visual_width = UnicodeWidthStr::width(line);
    if visual_width == 0 {
        return 1;
    }

    let wrapped_lines = visual_width.div_ceil(clamped_content_width);
    u16::try_from(wrapped_lines).unwrap_or(u16::MAX)
}

fn textarea_visual_line_count(textarea: &tui_textarea::TextArea<'_>, content_width: usize) -> u16 {
    textarea
        .lines()
        .iter()
        .map(|line| wrapped_visual_line_count(line, content_width))
        .sum::<u16>()
        .max(1)
}

pub(super) fn preferred_input_height(
    textarea: &tui_textarea::TextArea<'_>,
    area_width: u16,
) -> u16 {
    let content_width = usize::from(area_width.saturating_sub(4).max(1));
    let visual_line_count = textarea_visual_line_count(textarea, content_width);
    let required_height = visual_line_count.saturating_add(1);

    required_height.max(3)
}

fn review_prompt_hint(selection_count: usize) -> &'static str {
    if selection_count > 0 {
        " Review mode · Shift+Arrows extend · y copy · Esc clear "
    } else {
        " Review mode · v select · Shift+Arrows extend · y copy · Esc return "
    }
}

fn running_prompt_hint(mode: BusyInputMode, pending_submission_count: usize) -> String {
    if pending_submission_count > 0 {
        return match mode {
            BusyInputMode::Queue => {
                format!(
                    " Queue mode · {pending_submission_count} pending · Esc clear · Ctrl+G steer "
                )
            }
            BusyInputMode::Steer => {
                " Steer armed · sends after tool boundary · Esc clear · Ctrl+G queue ".to_owned()
            }
        };
    }

    match mode {
        BusyInputMode::Queue => " Enter queue · Ctrl+G steer · Esc clear pending ".to_owned(),
        BusyInputMode::Steer => {
            " Enter steer after tool · Ctrl+G queue · Esc clear pending ".to_owned()
        }
    }
}

fn idle_prompt_hint(mode: BusyInputMode) -> &'static str {
    match mode {
        BusyInputMode::Queue => " Enter send · busy turns queue · Ctrl+G steer · /help ",
        BusyInputMode::Steer => " Enter send · busy turns steer · Ctrl+G queue · /help ",
    }
}

pub(super) fn render_input(
    frame: &mut Frame<'_>,
    area: Rect,
    textarea: &tui_textarea::TextArea<'_>,
    pane: &impl InputView,
    focus: FocusLayer,
    palette: &Palette,
) {
    let default_prompt_hint = match focus {
        FocusLayer::Transcript => {
            review_prompt_hint(pane.transcript_selection_line_count()).to_owned()
        }
        FocusLayer::Composer
        | FocusLayer::Help
        | FocusLayer::ThemePicker
        | FocusLayer::SessionPicker
        | FocusLayer::StatsOverlay
        | FocusLayer::DiffOverlay
        | FocusLayer::ToolInspector
        | FocusLayer::ClarifyDialog => {
            let pending_submission_count = pane.pending_submission_count();
            let busy_input_mode = pane.busy_input_mode();
            if pane.agent_running() {
                running_prompt_hint(busy_input_mode, pending_submission_count)
            } else {
                idle_prompt_hint(busy_input_mode).to_owned()
            }
        }
    };
    let prompt_hint = pane.input_hint().unwrap_or(default_prompt_hint.as_str());

    let block = Block::default().style(Style::default().bg(palette.surface_alt));
    frame.render_widget(block, area);

    let content_area = area.inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    let textarea_height = content_area.height.saturating_sub(1);
    let textarea_area = Rect::new(
        content_area.x,
        content_area.y,
        content_area.width,
        textarea_height,
    );
    let hint_area = Rect::new(
        content_area.x,
        content_area.y.saturating_add(textarea_height),
        content_area.width,
        content_area.height.saturating_sub(textarea_height),
    );

    let rail_area = Rect::new(area.x, area.y, 1, area.height);
    let rail_color = if focus == FocusLayer::Composer {
        palette.brand
    } else {
        palette.separator
    };
    let rail_lines = (0..rail_area.height)
        .map(|_| Line::from(Span::styled("▎", Style::default().fg(rail_color))))
        .collect::<Vec<_>>();
    let rail = Paragraph::new(rail_lines);
    frame.render_widget(rail, rail_area);

    // Render textarea widget inside the block's inner area.
    frame.render_widget(textarea, textarea_area);

    let hint_widget = Paragraph::new(Line::from(Span::styled(
        prompt_hint,
        Style::default()
            .fg(palette.dim)
            .add_modifier(ratatui::style::Modifier::ITALIC),
    )));
    frame.render_widget(hint_widget, hint_area);

    if focus == FocusLayer::Composer
        && textarea_is_empty(textarea)
        && let Some(placeholder) = pane.input_placeholder()
    {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            placeholder,
            Style::default()
                .fg(palette.separator)
                .add_modifier(Modifier::ITALIC),
        )));
        frame.render_widget(placeholder, textarea_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    struct TestInput {
        running: bool,
        pending_submission_count: usize,
        busy_input_mode: BusyInputMode,
    }

    impl InputView for TestInput {
        fn agent_running(&self) -> bool {
            self.running
        }
        fn pending_submission_count(&self) -> usize {
            self.pending_submission_count
        }
        fn busy_input_mode(&self) -> BusyInputMode {
            self.busy_input_mode
        }
    }

    struct SelectionInput {
        selection_count: usize,
    }

    impl InputView for SelectionInput {
        fn agent_running(&self) -> bool {
            false
        }
        fn pending_submission_count(&self) -> usize {
            0
        }
        fn busy_input_mode(&self) -> BusyInputMode {
            BusyInputMode::Queue
        }
        fn transcript_selection_line_count(&self) -> usize {
            self.selection_count
        }
    }

    struct PlaceholderInput;

    impl InputView for PlaceholderInput {
        fn agent_running(&self) -> bool {
            false
        }
        fn pending_submission_count(&self) -> usize {
            0
        }
        fn busy_input_mode(&self) -> BusyInputMode {
            BusyInputMode::Queue
        }
        fn input_placeholder(&self) -> Option<String> {
            Some("Explain the layered kernel design in this workspace".to_owned())
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
    fn idle_input_shows_send_hint() {
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestInput {
            running: false,
            pending_submission_count: 0,
            busy_input_mode: BusyInputMode::Queue,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(
                    f,
                    f.area(),
                    &textarea,
                    &pane,
                    FocusLayer::Composer,
                    &palette,
                );
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Enter send"),
            "idle hint should mention Enter send"
        );
    }

    #[test]
    fn running_input_shows_queue_hint() {
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestInput {
            running: true,
            pending_submission_count: 0,
            busy_input_mode: BusyInputMode::Queue,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(
                    f,
                    f.area(),
                    &textarea,
                    &pane,
                    FocusLayer::Composer,
                    &palette,
                );
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Enter queue"),
            "running hint should mention queue"
        );
    }

    #[test]
    fn running_with_staged_shows_queued_hint() {
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestInput {
            running: true,
            pending_submission_count: 2,
            busy_input_mode: BusyInputMode::Queue,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(
                    f,
                    f.area(),
                    &textarea,
                    &pane,
                    FocusLayer::Composer,
                    &palette,
                );
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Queue mode"),
            "hint should mention queue mode"
        );
        assert!(text.contains("2 pending"), "hint should show pending count");
    }

    #[test]
    fn running_with_steer_mode_shows_steer_hint() {
        let backend = TestBackend::new(72, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestInput {
            running: true,
            pending_submission_count: 1,
            busy_input_mode: BusyInputMode::Steer,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(
                    f,
                    f.area(),
                    &textarea,
                    &pane,
                    FocusLayer::Composer,
                    &palette,
                );
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Steer armed"),
            "hint should mention steer mode"
        );
    }

    #[test]
    fn transcript_focus_shows_review_hint() {
        let backend = TestBackend::new(72, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestInput {
            running: false,
            pending_submission_count: 0,
            busy_input_mode: BusyInputMode::Queue,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(
                    f,
                    f.area(),
                    &textarea,
                    &pane,
                    FocusLayer::Transcript,
                    &palette,
                );
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("Review mode"),
            "transcript focus should explain that review mode is active: {text:?}"
        );
    }

    #[test]
    fn transcript_focus_with_selection_shows_copy_hint() {
        let backend = TestBackend::new(72, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = SelectionInput { selection_count: 3 };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(
                    f,
                    f.area(),
                    &textarea,
                    &pane,
                    FocusLayer::Transcript,
                    &palette,
                );
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("y copy"),
            "selection hint should mention copy: {text:?}"
        );
        assert!(
            text.contains("Esc clear"),
            "selection hint should mention clearing selection: {text:?}"
        );
    }

    #[test]
    fn empty_composer_renders_placeholder_inside_input_box() {
        let backend = TestBackend::new(72, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = PlaceholderInput;
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(
                    f,
                    f.area(),
                    &textarea,
                    &pane,
                    FocusLayer::Composer,
                    &palette,
                );
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("Explain the layered kernel design"),
            "placeholder text should render inside the empty composer: {text:?}"
        );
    }

    #[test]
    fn preferred_input_height_grows_for_wrapped_wide_text() {
        let mut textarea = tui_textarea::TextArea::default();
        textarea.insert_str("派三个subagents去给我分别搜索一下今天的国际新闻");

        let input_height = preferred_input_height(&textarea, 24);

        assert!(
            input_height > 3,
            "wrapped CJK text should grow the input area: {input_height}"
        );
    }
}
