use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, block::Position as TitlePosition},
};

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

pub(super) fn render_input(
    frame: &mut Frame<'_>,
    area: Rect,
    textarea: &tui_textarea::TextArea<'_>,
    pane: &impl InputView,
    focus: FocusLayer,
    palette: &Palette,
) {
    let border_color = palette.brand;
    let border_style = Style::default().fg(border_color);

    let default_prompt_hint = match focus {
        FocusLayer::Transcript => {
            if pane.transcript_selection_line_count() > 0 {
                " Review mode | Shift+Arrows extend | y copy | Esc clear "
            } else {
                " Review mode | v select | Shift+Arrows extend | y copy | Esc return "
            }
        }
        FocusLayer::Composer
        | FocusLayer::Help
        | FocusLayer::StatsOverlay
        | FocusLayer::ToolInspector
        | FocusLayer::ClarifyDialog => {
            let pending_submission_count = pane.pending_submission_count();
            let busy_input_mode = pane.busy_input_mode();
            if pane.agent_running() && pending_submission_count > 0 {
                match busy_input_mode {
                    BusyInputMode::Queue => " Queue mode | Esc clears queue | Ctrl+G steer ",
                    BusyInputMode::Steer => {
                        " Steer mode | Esc clears pending steer | Ctrl+G queue "
                    }
                }
            } else if pane.agent_running() {
                match busy_input_mode {
                    BusyInputMode::Queue => " Enter to queue | Ctrl+G steer | Esc clears pending ",
                    BusyInputMode::Steer => " Enter to steer at next tool boundary | Ctrl+G queue ",
                }
            } else {
                match busy_input_mode {
                    BusyInputMode::Queue => {
                        " Enter send | Shift+Enter newline | Ctrl+G steer | /help "
                    }
                    BusyInputMode::Steer => {
                        " Enter send | Shift+Enter newline | Ctrl+G queue | /help "
                    }
                }
            }
        }
    };
    let prompt_hint = pane.input_hint().unwrap_or(default_prompt_hint);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(
            prompt_hint,
            Style::default()
                .fg(palette.dim)
                .add_modifier(ratatui::style::Modifier::ITALIC),
        ))
        .title_position(TitlePosition::Bottom);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Render textarea widget inside the block's inner area.
    frame.render_widget(textarea, inner);

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
        frame.render_widget(placeholder, inner);
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
            text.contains("Enter to queue"),
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
            text.contains("Steer mode"),
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
}
