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
        // Each item takes 2 lines (label + hint) + 1 blank separator.
        // Last item has no trailing blank.
        let lines_per_item: u16 = 3;
        let total_lines =
            (self.items.len() as u16) * lines_per_item - self.items.len().min(1) as u16;

        // Vertically center the block in the content area.
        let top_pad = area.height.saturating_sub(total_lines) / 2;

        for (i, item) in self.items.iter().enumerate() {
            let y = area.y + top_pad + (i as u16) * lines_per_item;
            if y >= area.y + area.height {
                break;
            }
            let is_selected = i == state.selected();
            let indicator = if is_selected { "\u{25cf}" } else { "\u{25cb}" };
            let indicator_style = if is_selected {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let label_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            // Label line: ● Label  or  ○ Label
            let label_line = Line::from(vec![
                Span::styled(format!("  {indicator} "), indicator_style),
                Span::styled(&item.label, label_style),
            ]);
            buf.set_line(area.x, y, &label_line, area.width);

            // Hint line (indented under label)
            if let Some(hint) = &item.hint
                && y + 1 < area.y + area.height
            {
                let hint_style = Style::default().fg(Color::DarkGray);
                let hint_line = Line::from(Span::styled(format!("      {hint}"), hint_style));
                buf.set_line(area.x, y + 1, &hint_line, area.width);
            }
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
