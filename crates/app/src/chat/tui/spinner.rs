use std::time::Instant;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::theme::Palette;

pub(super) const FRAMES: &[&str] = &["\u{25d0}", "\u{25d3}", "\u{25d1}", "\u{25d2}"];
pub(super) const DOTS: &[&str] = &["   ", ".  ", ".. ", "..."];

/// Status message fade-out threshold in seconds.
const STATUS_FADE_SECS: u64 = 10;

// ---------------------------------------------------------------------------
// View trait — decouples rendering from the concrete `Pane` struct
// ---------------------------------------------------------------------------

pub(super) trait SpinnerView {
    fn agent_running(&self) -> bool;
    fn spinner_frame(&self) -> usize;
    fn dots_frame(&self) -> usize;
    fn loop_state(&self) -> &str;
    fn loop_iteration(&self) -> u32;
    fn status_message(&self) -> Option<(&str, &Instant)>;
}

pub(super) fn render_spinner(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &impl SpinnerView,
    palette: &Palette,
) {
    let content = if pane.agent_running() {
        let idx = pane.spinner_frame() % FRAMES.len();
        let spinner = FRAMES.get(idx).copied().unwrap_or("");
        let didx = pane.dots_frame() % DOTS.len();
        let dots = DOTS.get(didx).copied().unwrap_or("");

        let state_info = if pane.loop_state().is_empty() {
            String::new()
        } else {
            format!(" | {}", pane.loop_state())
        };

        Line::from(vec![
            Span::styled(
                format!(" {spinner} "),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("Iteration {}", pane.loop_iteration()),
                Style::default().fg(palette.text),
            ),
            Span::styled(state_info, Style::default().fg(palette.dim)),
            Span::styled(format!(" {dots}"), Style::default().fg(palette.warning)),
        ])
    } else if let Some((msg, when)) = pane.status_message() {
        if when.elapsed().as_secs() < STATUS_FADE_SECS {
            Line::styled(
                format!(" {msg}"),
                Style::default()
                    .fg(palette.dim)
                    .add_modifier(Modifier::ITALIC),
            )
        } else {
            ready_line(palette)
        }
    } else {
        ready_line(palette)
    };

    frame.render_widget(Paragraph::new(content), area);
}

fn ready_line(palette: &Palette) -> Line<'static> {
    Line::styled(
        " Ready".to_string(),
        Style::default()
            .fg(palette.dim)
            .add_modifier(Modifier::ITALIC),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::time::Instant;

    struct TestSpinner {
        running: bool,
        spinner_frame: usize,
        dots_frame: usize,
        loop_state: String,
        loop_iteration: u32,
        status_message: Option<(String, Instant)>,
    }

    impl TestSpinner {
        fn idle() -> Self {
            Self {
                running: false,
                spinner_frame: 0,
                dots_frame: 0,
                loop_state: String::new(),
                loop_iteration: 0,
                status_message: None,
            }
        }

        fn active() -> Self {
            Self {
                running: true,
                spinner_frame: 1,
                dots_frame: 2,
                loop_state: "calling model".into(),
                loop_iteration: 2,
                status_message: None,
            }
        }
    }

    impl SpinnerView for TestSpinner {
        fn agent_running(&self) -> bool {
            self.running
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
    fn idle_renders_ready() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner::idle();
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("Ready"), "idle state should show Ready");
    }

    #[test]
    fn running_renders_iteration_and_state() {
        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner::active();
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("Iteration 2"), "should show iteration number");
        assert!(text.contains("calling model"), "should show loop state");
    }

    #[test]
    fn spinner_frame_cycles() {
        assert_eq!(FRAMES.len(), 4);
        assert_eq!(DOTS.len(), 4);
        // Modular access never panics
        for i in 0..20 {
            let _ = FRAMES.get(i % FRAMES.len());
            let _ = DOTS.get(i % DOTS.len());
        }
    }

    #[test]
    fn status_message_displayed_when_recent() {
        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner {
            status_message: Some(("Model switched".into(), Instant::now())),
            ..TestSpinner::idle()
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Model switched"),
            "recent status message should be visible"
        );
    }
}
