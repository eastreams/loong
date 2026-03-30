use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Span,
    widgets::{Block, Borders, block::Position as TitlePosition},
};

use super::theme::Palette;

// ---------------------------------------------------------------------------
// View trait — decouples rendering from the concrete `Pane` struct
// ---------------------------------------------------------------------------

pub(super) trait InputView {
    fn agent_running(&self) -> bool;
    fn has_staged_message(&self) -> bool;
}

pub(super) fn render_input(
    frame: &mut Frame<'_>,
    area: Rect,
    textarea: &tui_textarea::TextArea<'_>,
    pane: &impl InputView,
    palette: &Palette,
) {
    let border_color = palette.brand;
    let border_style = Style::default().fg(border_color);

    let prompt_hint = if pane.agent_running() && pane.has_staged_message() {
        " Queued: 1 message | Esc to clear "
    } else if pane.agent_running() {
        " Enter to queue | Esc to cancel queue "
    } else {
        " Enter to send | /help for commands "
    };

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    struct TestInput {
        running: bool,
        staged: bool,
    }

    impl InputView for TestInput {
        fn agent_running(&self) -> bool {
            self.running
        }
        fn has_staged_message(&self) -> bool {
            self.staged
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
            staged: false,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(f, f.area(), &textarea, &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Enter to send"),
            "idle hint should mention Enter to send"
        );
    }

    #[test]
    fn running_input_shows_queue_hint() {
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestInput {
            running: true,
            staged: false,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(f, f.area(), &textarea, &pane, &palette);
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
            staged: true,
        };
        let palette = Palette::dark();
        let textarea = tui_textarea::TextArea::default();

        terminal
            .draw(|f| {
                render_input(f, f.area(), &textarea, &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Queued: 1 message"),
            "staged hint should mention queued message"
        );
    }
}
