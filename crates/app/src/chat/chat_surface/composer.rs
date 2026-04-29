use super::utils::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;

pub struct Composer {
    input: String,
    cursor: usize,
}

impl Composer {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
        }
    }

    pub fn height_for_width(&self, width: u16) -> u16 {
        wrapped_height(&self.input, width).clamp(1, 10)
    }

    pub fn height_for_area(&self, width: u16, terminal_height: u16) -> u16 {
        let max_height = composer_max_height_for_terminal(terminal_height);
        wrapped_height(&self.input, width).clamp(1, max_height)
    }

    pub fn is_empty(&self) -> bool {
        self.input.trim().is_empty()
    }

    pub fn text(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn replace_range(&mut self, range: std::ops::Range<usize>, replacement: &str) {
        self.input.replace_range(range.clone(), replacement);
        self.cursor = range.start.saturating_add(replacement.len());
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
    }

    pub fn set_input(&mut self, input: String) {
        self.cursor = input.len();
        self.input = input;
    }

    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.input.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub fn insert_paste(&mut self, text: &str) {
        let normalized = normalize_paste_text(text);
        self.insert_text(normalized.as_str());
    }

    pub fn take_input(&mut self) -> String {
        let input = self.input.clone();
        self.clear();
        input
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, focused: bool) {
        let prefix_style = Style::default()
            .fg(if focused { SURFACE_CYAN } else { SURFACE_GRAY })
            .add_modifier(Modifier::BOLD);
        let rows = wrapped_rows(&self.input, area.width);
        let mut lines = Vec::with_capacity(rows.len().max(1));
        for (index, row) in rows.into_iter().enumerate() {
            let prefix = if index == 0 {
                Span::styled(" › ", prefix_style)
            } else {
                Span::styled("   ", prefix_style)
            };
            let mut spans = vec![prefix];
            spans.extend(highlight_composer_row(row.as_str()));
            lines.push(Line::from(spans));
        }
        if lines.is_empty() {
            lines.push(Line::from(vec![Span::styled(" › ", prefix_style)]));
        }

        let p = Paragraph::new(lines).wrap(Wrap { trim: false });

        f.render_widget(p, area);
    }

    pub fn cursor_position(&self, area: Rect) -> (u16, u16) {
        let prefix_width = 3usize;
        let available_width = area.width.saturating_sub(prefix_width as u16).max(1) as usize;
        let mut row = 0usize;
        let mut line_col = 0usize;

        for grapheme in self.input[..self.cursor].graphemes(true) {
            if grapheme == "\n" {
                row += 1;
                line_col = 0;
                continue;
            }

            let ch_width = display_width(grapheme);
            if line_col + ch_width > available_width {
                row += 1;
                line_col = 0;
            }
            line_col += ch_width;
        }

        let col = prefix_width + line_col;

        (
            area.x + col.min(area.width.saturating_sub(1) as usize) as u16,
            area.y + row.min(area.height.saturating_sub(1) as usize) as u16,
        )
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match key.code {
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                let msg = self.input.clone();
                if !msg.trim().is_empty() {
                    self.clear();
                    return Some(msg);
                }
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.input.insert(self.cursor, '\n');
                self.cursor += 1;
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.input.len();
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = previous_grapheme_boundary(&self.input, self.cursor);
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = next_grapheme_boundary(&self.input, self.cursor);
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.cursor = previous_word_boundary(&self.input, self.cursor);
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.cursor = next_word_boundary(&self.input, self.cursor);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.replace_range(..self.cursor, "");
                self.cursor = 0;
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.replace_range(self.cursor.., "");
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let new_cursor = previous_word_boundary(&self.input, self.cursor);
                self.input.replace_range(new_cursor..self.cursor, "");
                self.cursor = new_cursor;
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
                let end = next_word_boundary(&self.input, self.cursor);
                self.input.replace_range(self.cursor..end, "");
            }
            KeyCode::Char(c) => {
                let mut buffer = [0; 4];
                self.insert_text(c.encode_utf8(&mut buffer));
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let new_cursor = previous_grapheme_boundary(&self.input, self.cursor);
                    self.input.replace_range(new_cursor..self.cursor, "");
                    self.cursor = new_cursor;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    let end = next_grapheme_boundary(&self.input, self.cursor);
                    self.input.replace_range(self.cursor..end, "");
                }
            }
            KeyCode::Left => {
                self.cursor = previous_grapheme_boundary(&self.input, self.cursor);
            }
            KeyCode::Right => {
                self.cursor = next_grapheme_boundary(&self.input, self.cursor);
            }
            KeyCode::Home => {
                self.cursor = line_start_boundary(&self.input, self.cursor);
            }
            KeyCode::End => {
                self.cursor = line_end_boundary(&self.input, self.cursor);
            }
            KeyCode::Enter
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Insert
            | KeyCode::F(_)
            | KeyCode::Null
            | KeyCode::Esc
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
            | KeyCode::KeypadBegin
            | KeyCode::Media(_)
            | KeyCode::Modifier(_) => {}
        }
        None
    }
}

fn normalize_paste_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn previous_grapheme_boundary(text: &str, cursor: usize) -> usize {
    UnicodeSegmentation::grapheme_indices(text, true)
        .take_while(|(idx, _)| *idx < cursor)
        .last()
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

fn next_grapheme_boundary(text: &str, cursor: usize) -> usize {
    if cursor >= text.len() {
        return text.len();
    }

    UnicodeSegmentation::grapheme_indices(text, true)
        .find_map(|(idx, _grapheme)| {
            if idx <= cursor {
                return None;
            }
            Some(idx)
        })
        .unwrap_or(text.len())
}

fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    let mut seen_word = false;
    for (idx, ch) in text[..cursor].char_indices().rev() {
        if ch.is_whitespace() {
            if seen_word {
                return idx + ch.len_utf8();
            }
        } else {
            seen_word = true;
        }
    }
    0
}

fn next_word_boundary(text: &str, cursor: usize) -> usize {
    let mut seen_word = false;
    for (offset, ch) in text[cursor..].char_indices() {
        let idx = cursor + offset;
        if ch.is_whitespace() {
            if seen_word {
                return idx;
            }
        } else {
            seen_word = true;
        }
    }
    text.len()
}

fn line_start_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind('\n').map(|idx| idx + 1).unwrap_or(0)
}

fn line_end_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .find('\n')
        .map(|offset| cursor + offset)
        .unwrap_or(text.len())
}

fn display_width(grapheme: &str) -> usize {
    crate::presentation::display_width(grapheme).max(1)
}

fn composer_max_height_for_terminal(terminal_height: u16) -> u16 {
    let proportional = terminal_height.saturating_div(4).clamp(3, 14);
    if terminal_height < 16 {
        proportional.min(4)
    } else {
        proportional
    }
}

fn wrapped_height(text: &str, width: u16) -> u16 {
    wrapped_rows(text, width).len().max(1) as u16
}

fn wrapped_rows(text: &str, width: u16) -> Vec<String> {
    let prefix_width = 3usize;
    let available_width = width.saturating_sub(prefix_width as u16).max(1) as usize;
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut line_col = 0usize;

    for grapheme in text.graphemes(true) {
        if grapheme == "\n" {
            rows.push(current);
            current = String::new();
            line_col = 0;
            continue;
        }

        let ch_width = display_width(grapheme);
        if line_col + ch_width > available_width && !current.is_empty() {
            rows.push(current);
            current = String::new();
            line_col = 0;
        }
        current.push_str(grapheme);
        line_col += ch_width;
    }

    rows.push(current);
    rows
}

fn highlight_composer_row(row: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut in_whitespace = None;

    let flush =
        |spans: &mut Vec<Span<'static>>, current: &mut String, in_whitespace: &mut Option<bool>| {
            let Some(is_whitespace) = *in_whitespace else {
                return;
            };
            if current.is_empty() {
                return;
            }
            let text = std::mem::take(current);
            if is_whitespace {
                spans.push(Span::raw(text));
            } else if text.starts_with('$') && text.len() > 1 {
                spans.push(Span::styled(
                    text,
                    Style::default()
                        .fg(SURFACE_ACCENT)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(text));
            }
            *in_whitespace = None;
        };

    for ch in row.chars() {
        let is_whitespace = ch.is_whitespace();
        match in_whitespace {
            Some(mode) if mode == is_whitespace => current.push(ch),
            Some(_) => {
                flush(&mut spans, &mut current, &mut in_whitespace);
                current.push(ch);
                in_whitespace = Some(is_whitespace);
            }
            None => {
                current.push(ch);
                in_whitespace = Some(is_whitespace);
            }
        }
    }

    flush(&mut spans, &mut current, &mut in_whitespace);
    spans
}

#[cfg(test)]
mod tests {
    use super::Composer;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn supports_multibyte_input_without_invalid_cursor_boundary() {
        let mut composer = Composer::new();

        assert!(composer.handle_key(key(KeyCode::Char('你'))).is_none());
        assert!(composer.handle_key(key(KeyCode::Char('好'))).is_none());

        let submitted = composer.handle_key(key(KeyCode::Enter));

        assert_eq!(submitted.as_deref(), Some("你好"));
    }

    #[test]
    fn cursor_position_respects_prefix_before_wrapping() {
        let mut composer = Composer::new();
        assert!(composer.handle_key(key(KeyCode::Char('a'))).is_none());
        assert!(composer.handle_key(key(KeyCode::Char('b'))).is_none());

        assert_eq!(composer.cursor_position(Rect::new(0, 0, 6, 3)), (5, 0));

        assert!(composer.handle_key(key(KeyCode::Char('c'))).is_none());
        assert_eq!(composer.cursor_position(Rect::new(0, 0, 6, 3)), (5, 0));

        assert!(composer.handle_key(key(KeyCode::Char('d'))).is_none());

        assert_eq!(composer.cursor_position(Rect::new(0, 0, 6, 3)), (4, 1));
    }

    #[test]
    fn height_for_width_grows_when_single_line_wraps() {
        let mut composer = Composer::new();
        composer.set_input("abcdefg".to_owned());

        assert_eq!(composer.height_for_width(6), 3);
        assert_eq!(composer.height_for_width(10), 1);
    }

    #[test]
    fn height_for_area_uses_terminal_height_without_unbounded_growth() {
        let mut composer = Composer::new();
        composer.set_input("line\n".repeat(40));

        assert_eq!(composer.height_for_area(80, 12), 3);
        assert_eq!(composer.height_for_area(80, 80), 14);
        assert_eq!(composer.height_for_width(80), 10);
    }

    #[test]
    fn wrapped_render_keeps_continuation_rows_indented_under_prompt() {
        let mut composer = Composer::new();
        composer.set_input("abcdefg".to_owned());

        let rows = super::wrapped_rows("abcdefg", 6);

        assert_eq!(
            rows,
            vec!["abc".to_owned(), "def".to_owned(), "g".to_owned()]
        );
    }

    #[test]
    fn word_motion_and_delete_shortcuts_keep_cursor_on_valid_boundaries() {
        let mut composer = Composer::new();
        for ch in "foo 你好 bar".chars() {
            assert!(composer.handle_key(key(KeyCode::Char(ch))).is_none());
        }

        assert!(
            composer
                .handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT))
                .is_none()
        );
        assert!(
            composer
                .handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL))
                .is_none()
        );

        let submitted = composer.handle_key(key(KeyCode::Enter));
        assert_eq!(submitted.as_deref(), Some("foo bar"));
    }

    #[test]
    fn highlight_composer_row_accents_skill_invocations() {
        let spans = super::highlight_composer_row("$demo-skill next");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "$demo-skill");
        assert_eq!(spans[0].style.fg, Some(super::SURFACE_ACCENT));
        assert_eq!(spans[1].content.as_ref(), " ");
        assert_eq!(spans[2].content.as_ref(), "next");
    }

    #[test]
    fn dollar_prefixed_skill_invocation_remains_editable_plain_text() {
        let mut composer = Composer::new();
        for ch in "$demo-skill explain this".chars() {
            assert!(
                composer
                    .handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                    .is_none()
            );
        }

        let submitted = composer.handle_key(key(KeyCode::Enter));
        assert_eq!(submitted.as_deref(), Some("$demo-skill explain this"));
    }

    #[test]
    fn paste_inserts_at_cursor_and_normalizes_line_endings() {
        let mut composer = Composer::new();
        composer.set_input("ab".to_owned());
        assert!(composer.handle_key(key(KeyCode::Left)).is_none());

        composer.insert_paste("你\r\n好");

        assert_eq!(composer.text(), "a你\n好b");
        assert_eq!(composer.cursor(), "a你\n好".len());
    }
}
