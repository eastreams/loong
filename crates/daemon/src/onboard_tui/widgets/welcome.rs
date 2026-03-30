use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

#[allow(dead_code)] // consumed by runner/layout in later tasks
pub(crate) struct WelcomeScreen {
    version: String,
}

#[allow(dead_code)] // consumed by runner/layout in later tasks
impl WelcomeScreen {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
        }
    }
}

impl Widget for WelcomeScreen {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "LOONGCLAW",
                Style::default()
                    .fg(Color::Rgb(245, 169, 127))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                &self.version,
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Setup Wizard",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "This wizard will configure authentication, runtime defaults,",
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                "workspace paths, protocols, and environment readiness.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Safe to rerun. Press Enter to begin.",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_renders_brand_and_version() {
        let widget = WelcomeScreen::new("v0.1.0-alpha.2");
        let area = Rect::new(0, 0, 60, 12);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buffer_text(&buf);
        assert!(content.contains("LOONGCLAW"));
        assert!(content.contains("v0.1.0-alpha.2"));
        assert!(content.contains("Setup Wizard"));
        assert!(content.contains("Enter to begin"));
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            text.push('\n');
        }
        text
    }
}
