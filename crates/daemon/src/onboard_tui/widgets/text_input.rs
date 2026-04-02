// TextInputWidget: single-line text input for the onboard wizard.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::onboard_tui::theme::OnboardPalette;

pub(crate) struct TextInputState {
    value: String,
    default: Option<String>,
    default_active: bool,
    cursor: usize,
    error: Option<String>,
}

impl TextInputState {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            default: None,
            default_active: false,
            cursor: 0,
            error: None,
        }
    }

    #[allow(dead_code)] // used in tests
    pub fn with_value(value: impl Into<String>) -> Self {
        let v: String = value.into();
        let len = v.len();
        Self {
            value: v,
            default: None,
            default_active: false,
            cursor: len,
            error: None,
        }
    }

    pub fn with_default(default: impl Into<String>) -> Self {
        Self {
            value: String::new(),
            default: Some(default.into()),
            default_active: true,
            cursor: 0,
            error: None,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn display_value(&self) -> &str {
        if self.default_active {
            self.default.as_deref().unwrap_or("")
        } else {
            &self.value
        }
    }

    pub fn is_default_active(&self) -> bool {
        self.default_active
    }

    pub fn has_default(&self) -> bool {
        self.default.is_some()
    }

    pub fn push(&mut self, c: char) {
        if self.default_active {
            self.default_active = false;
            self.value.clear();
        }
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.default_active {
            self.default_active = false;
            self.value.clear();
            return;
        }
        if self.cursor > 0 {
            let prev = self.value[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.value.remove(prev);
            self.cursor = prev;
        }
        self.error = None;
    }

    pub fn submit_value(&self) -> &str {
        if self.default_active {
            self.default.as_deref().unwrap_or("")
        } else {
            &self.value
        }
    }

    pub fn move_left(&mut self) {
        if self.default_active {
            return;
        }
        if self.cursor > 0 {
            let prev = self.value[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor = prev;
        }
    }

    pub fn move_right(&mut self) {
        if self.default_active {
            return;
        }
        if self.cursor < self.value.len() {
            let next = self.value[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.value.len());
            self.cursor = next;
        }
    }

    pub fn move_home(&mut self) {
        if self.default_active {
            return;
        }
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        if self.default_active {
            return;
        }
        self.cursor = self.value.len();
    }

    pub fn delete(&mut self) {
        if self.default_active {
            self.default_active = false;
            self.value.clear();
            return;
        }
        if self.cursor < self.value.len() {
            self.value.remove(self.cursor);
            self.error = None;
        }
    }

    pub fn clear(&mut self) {
        self.default_active = false;
        self.value.clear();
        self.cursor = 0;
        self.error = None;
    }

    #[allow(dead_code)] // used in tests
    pub fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    #[allow(dead_code)] // used in tests
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn cursor_position(&self) -> usize {
        self.cursor
    }
}

pub(crate) struct TextInputWidget {
    label: String,
    show_label: bool,
}

impl TextInputWidget {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            show_label: true,
        }
    }

    pub fn without_label(mut self) -> Self {
        self.show_label = false;
        self
    }

    pub fn render_with_state(&self, area: Rect, buf: &mut Buffer, state: &TextInputState) {
        let palette = OnboardPalette::current();
        if area.height < 1 {
            return;
        }
        let value_style = if state.default_active {
            Style::default().fg(palette.muted_text)
        } else {
            Style::default().fg(palette.text)
        };
        let cursor_span = Span::styled("\u{258f}", Style::default().fg(palette.brand));
        let mut spans = Vec::new();

        if self.show_label {
            let label_span = Span::styled(&self.label, Style::default().fg(palette.secondary_text));
            spans.push(label_span);
            spans.push(Span::raw(" "));
        }

        if state.default_active {
            let value_span = Span::styled(state.display_value(), value_style);
            spans.push(cursor_span);
            spans.push(value_span);
        } else {
            let (before, after) = state.value().split_at(state.cursor_position());
            let before_span = Span::styled(before, value_style);
            let after_span = Span::styled(after, value_style);
            spans.push(before_span);
            spans.push(cursor_span);
            spans.push(after_span);
        }

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);

        if let Some(error) = &state.error
            && area.height >= 2
        {
            let err_line = Line::from(Span::styled(
                format!("  \u{26a0} {error}"),
                Style::default().fg(palette.error),
            ));
            buf.set_line(area.x, area.y + 1, &err_line, area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    #[test]
    fn input_state_appends_characters() {
        let mut state = TextInputState::new();
        state.push('h');
        state.push('i');
        assert_eq!(state.value(), "hi");
    }

    #[test]
    fn input_state_backspace_removes_last_char() {
        let mut state = TextInputState::with_value("hello");
        state.backspace();
        assert_eq!(state.value(), "hell");
    }

    #[test]
    fn input_state_backspace_on_empty_is_noop() {
        let mut state = TextInputState::new();
        state.backspace();
        assert_eq!(state.value(), "");
    }

    #[test]
    fn input_state_clears_default_on_first_keystroke() {
        let mut state = TextInputState::with_default("placeholder");
        assert_eq!(state.display_value(), "placeholder");
        state.push('x');
        assert_eq!(state.value(), "x");
    }

    #[test]
    fn input_state_sets_validation_error() {
        let mut state = TextInputState::new();
        state.set_error(Some("path not writable".into()));
        assert_eq!(state.error(), Some("path not writable"));
    }

    #[test]
    fn input_state_move_left_moves_cursor() {
        let mut state = TextInputState::with_value("abc");
        assert_eq!(state.cursor, 3);
        state.move_left();
        assert_eq!(state.cursor, 2);
        state.move_left();
        assert_eq!(state.cursor, 1);
        state.move_left();
        assert_eq!(state.cursor, 0);
        // Should not go below 0
        state.move_left();
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn input_state_move_right_moves_cursor() {
        let mut state = TextInputState::with_value("abc");
        state.cursor = 0;
        state.move_right();
        assert_eq!(state.cursor, 1);
        state.move_right();
        assert_eq!(state.cursor, 2);
        state.move_right();
        assert_eq!(state.cursor, 3);
        // Should not go past end
        state.move_right();
        assert_eq!(state.cursor, 3);
    }

    #[test]
    fn input_state_delete_removes_char_at_cursor() {
        let mut state = TextInputState::with_value("abc");
        state.cursor = 1;
        state.delete();
        assert_eq!(state.value(), "ac");
        assert_eq!(state.cursor, 1);
    }

    #[test]
    fn input_state_delete_at_end_is_noop() {
        let mut state = TextInputState::with_value("abc");
        state.delete();
        assert_eq!(state.value(), "abc");
    }

    #[test]
    fn input_state_delete_clears_default() {
        let mut state = TextInputState::with_default("placeholder");
        state.delete();
        assert!(!state.default_active);
        assert_eq!(state.value(), "");
    }

    #[test]
    fn input_state_move_home_and_end() {
        let mut state = TextInputState::with_value("hello");
        state.move_home();
        assert_eq!(state.cursor, 0);
        state.move_end();
        assert_eq!(state.cursor, 5);
    }

    #[test]
    fn input_state_navigation_noop_when_default_active() {
        let mut state = TextInputState::with_default("placeholder");
        state.move_left();
        assert!(state.default_active);
        state.move_right();
        assert!(state.default_active);
        state.move_home();
        assert!(state.default_active);
        state.move_end();
        assert!(state.default_active);
    }

    #[test]
    fn text_input_label_tracks_light_palette() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let widget = TextInputWidget::new("Path");
        let state = TextInputState::new();
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        widget.render_with_state(area, &mut buf, &state);

        assert_eq!(buf[(0, 0)].fg, OnboardPalette::light().secondary_text);
    }

    #[test]
    fn value_only_input_omits_label_prefix() {
        let widget = TextInputWidget::new("Feishu credential env").without_label();
        let state = TextInputState::with_default("FEISHU_APP_ID");
        let area = Rect::new(0, 0, 32, 2);
        let mut buf = Buffer::empty(area);

        widget.render_with_state(area, &mut buf, &state);

        let rendered = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect::<String>();
        assert!(!rendered.contains("Feishu"));
        assert!(rendered.contains("FEISHU_APP_ID"));
    }
}
