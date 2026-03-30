// SelectionCardWidget: selectable card list for the onboard wizard.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::StatefulWidget;

#[allow(dead_code)] // consumed by later tasks (screen / runner)
pub(crate) struct SelectionItem {
    pub label: String,
    pub hint: Option<String>,
}

#[allow(dead_code)] // consumed by later tasks (screen / runner)
impl SelectionItem {
    pub fn new(label: impl Into<String>, hint: Option<impl Into<String>>) -> Self {
        Self {
            label: label.into(),
            hint: hint.map(Into::into),
        }
    }
}

#[allow(dead_code)] // consumed by later tasks (screen / runner)
pub(crate) struct SelectionCardState {
    selected: usize,
    count: usize,
}

#[allow(dead_code)] // consumed by later tasks (screen / runner)
impl SelectionCardState {
    pub fn new(count: usize) -> Self {
        Self { selected: 0, count }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.count.saturating_sub(1));
    }

    pub fn next(&mut self) {
        if self.count == 0 {
            return;
        }
        self.selected = (self.selected + 1) % self.count;
    }

    pub fn previous(&mut self) {
        if self.count == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            self.count - 1
        } else {
            self.selected - 1
        };
    }
}

#[allow(dead_code)] // consumed by later tasks (screen / runner)
pub(crate) struct SelectionCardWidget {
    items: Vec<SelectionItem>,
}

#[allow(dead_code)] // consumed by later tasks (screen / runner)
impl SelectionCardWidget {
    pub fn new(items: Vec<SelectionItem>) -> Self {
        Self { items }
    }
}

impl StatefulWidget for SelectionCardWidget {
    type State = SelectionCardState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        for (i, item) in self.items.iter().enumerate() {
            let y = area.y + (i as u16) * 2;
            if y + 1 >= area.y + area.height {
                break;
            }
            let is_selected = i == state.selected();
            let border_style = if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let indicator = if is_selected { "\u{25b8}" } else { " " };
            let label_style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            // Top border
            let border = "\u{2500}".repeat((area.width as usize).saturating_sub(3));
            let top = format!(" \u{250c}{border}\u{2510}");
            let top_truncated = &top[..top.len().min(area.width as usize)];
            buf.set_string(area.x, y, top_truncated, border_style);

            // Content line
            let mut spans = vec![
                Span::styled(format!(" {indicator} "), border_style),
                Span::styled(&item.label, label_style),
            ];
            if let Some(hint) = &item.hint {
                let pad = (area.width as usize).saturating_sub(item.label.len() + hint.len() + 6);
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(hint, Style::default().fg(Color::Green)));
            }
            buf.set_line(area.x, y + 1, &Line::from(spans), area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::StatefulWidget;

    #[test]
    fn card_highlights_selected_item() {
        let items = vec![
            SelectionItem::new("OpenAI", Some("credentials found")),
            SelectionItem::new("Anthropic", None::<&str>),
            SelectionItem::new("DeepSeek", None::<&str>),
        ];
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(3);
        state.select(0);

        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &mut state);

        let content = buffer_text(&buf);
        assert!(
            content.contains("OpenAI"),
            "selected item should be visible"
        );
        assert!(content.contains("credentials found"), "hint should render");
    }

    #[test]
    fn card_state_wraps_on_navigation() {
        let mut state = SelectionCardState::new(3);
        state.select(2);
        state.next();
        assert_eq!(state.selected(), 0, "should wrap to first item");
    }

    #[test]
    fn card_state_wraps_backward() {
        let mut state = SelectionCardState::new(3);
        state.select(0);
        state.previous();
        assert_eq!(state.selected(), 2, "should wrap to last item");
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
