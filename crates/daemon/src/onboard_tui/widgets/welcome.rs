use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

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
                "This wizard will configure authentication,",
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                "runtime defaults, workspace paths, protocols,",
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                "and environment readiness.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Safe to rerun. Press Enter to begin.",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        // Render inside a centered dialog box with rounded border
        let content_lines = lines.len() as u16;
        let max_inner_width = (area.width.saturating_sub(4)).min(60);
        let box_height = content_lines + 2;
        let box_width = max_inner_width + 2;

        let x = area.x + (area.width.saturating_sub(box_width)) / 2;
        let y = area.y + (area.height.saturating_sub(box_height)) / 2;

        let outer = Rect::new(x, y, box_width.min(area.width), box_height.min(area.height));

        if outer.width >= 2 && outer.height >= 2 {
            let border_style = Style::default().fg(Color::Cyan);
            // Top: ╭───╮
            buf.set_string(outer.x, outer.y, "\u{256d}", border_style);
            for bx in (outer.x + 1)..(outer.x + outer.width - 1) {
                buf.set_string(bx, outer.y, "\u{2500}", border_style);
            }
            buf.set_string(outer.x + outer.width - 1, outer.y, "\u{256e}", border_style);
            // Sides: │ │
            for by in (outer.y + 1)..(outer.y + outer.height - 1) {
                buf.set_string(outer.x, by, "\u{2502}", border_style);
                buf.set_string(outer.x + outer.width - 1, by, "\u{2502}", border_style);
            }
            // Bottom: ╰───╯
            buf.set_string(
                outer.x,
                outer.y + outer.height - 1,
                "\u{2570}",
                border_style,
            );
            for bx in (outer.x + 1)..(outer.x + outer.width - 1) {
                buf.set_string(bx, outer.y + outer.height - 1, "\u{2500}", border_style);
            }
            buf.set_string(
                outer.x + outer.width - 1,
                outer.y + outer.height - 1,
                "\u{256f}",
                border_style,
            );
        }

        // Render lines inside the border with left padding
        let inner_x = x + 3;
        let inner_y = y + 1;
        let inner_width = max_inner_width.saturating_sub(2);
        for (i, line) in lines.iter().enumerate() {
            let ly = inner_y + i as u16;
            if ly >= inner_y + content_lines.min(area.height.saturating_sub(2)) {
                break;
            }
            buf.set_line(inner_x, ly, line, inner_width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_renders_brand_and_version() {
        let widget = WelcomeScreen::new("v0.1.0-alpha.2");
        let area = Rect::new(0, 0, 66, 15);
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
