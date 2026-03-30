#[derive(Debug, Clone)]
pub(super) struct ClarifyDialog {
    pub(super) question: String,
    pub(super) choices: Vec<String>,
    pub(super) input: String,
    pub(super) cursor: usize,
    pub(super) selected_choice: Option<usize>,
}

impl ClarifyDialog {
    pub(super) fn new(question: String, choices: Vec<String>) -> Self {
        Self {
            question,
            choices,
            input: String::new(),
            cursor: 0,
            selected_choice: None,
        }
    }

    pub(super) fn insert_char(&mut self, ch: char) {
        let byte_idx = self
            .input
            .char_indices()
            .nth(self.cursor)
            .map_or(self.input.len(), |(i, _)| i);
        self.input.insert(byte_idx, ch);
        self.cursor += 1;
    }

    pub(super) fn delete_back(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let byte_idx = self
            .input
            .char_indices()
            .nth(self.cursor)
            .map_or(self.input.len(), |(i, _)| i);
        let next_byte = self
            .input
            .char_indices()
            .nth(self.cursor + 1)
            .map_or(self.input.len(), |(i, _)| i);
        self.input.replace_range(byte_idx..next_byte, "");
    }

    pub(super) fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(super) fn move_cursor_right(&mut self) {
        let char_count = self.input.chars().count();
        if self.cursor < char_count {
            self.cursor += 1;
        }
    }

    pub(super) fn select_up(&mut self) {
        if self.choices.is_empty() {
            return;
        }
        self.selected_choice = Some(match self.selected_choice {
            Some(0) | None => self.choices.len().saturating_sub(1),
            Some(n) => n.saturating_sub(1),
        });
    }

    pub(super) fn select_down(&mut self) {
        if self.choices.is_empty() {
            return;
        }
        self.selected_choice = Some(match self.selected_choice {
            None => 0,
            Some(n) => {
                if n + 1 >= self.choices.len() {
                    0
                } else {
                    n + 1
                }
            }
        });
    }

    pub(super) fn response(&self) -> String {
        if let Some(idx) = self.selected_choice {
            self.choices.get(idx).cloned().unwrap_or_default()
        } else {
            self.input.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_dialog_is_empty() {
        let d = ClarifyDialog::new("question?".into(), vec!["a".into(), "b".into()]);
        assert_eq!(d.question, "question?");
        assert_eq!(d.choices.len(), 2);
        assert!(d.input.is_empty());
        assert_eq!(d.cursor, 0);
        assert!(d.selected_choice.is_none());
    }

    #[test]
    fn insert_and_cursor_movement() {
        let mut d = ClarifyDialog::new(String::new(), vec![]);
        d.insert_char('h');
        d.insert_char('i');
        assert_eq!(d.input, "hi");
        assert_eq!(d.cursor, 2);

        d.move_cursor_left();
        assert_eq!(d.cursor, 1);
        d.insert_char('a');
        assert_eq!(d.input, "hai");
        assert_eq!(d.cursor, 2);

        d.move_cursor_right();
        assert_eq!(d.cursor, 3);
        // Should clamp at end
        d.move_cursor_right();
        assert_eq!(d.cursor, 3);
    }

    #[test]
    fn delete_back() {
        let mut d = ClarifyDialog::new(String::new(), vec![]);
        d.insert_char('a');
        d.insert_char('b');
        d.insert_char('c');
        d.delete_back();
        assert_eq!(d.input, "ab");
        assert_eq!(d.cursor, 2);

        d.move_cursor_left();
        d.delete_back();
        assert_eq!(d.input, "b");
        assert_eq!(d.cursor, 0);

        // delete_back at 0 does nothing
        d.delete_back();
        assert_eq!(d.input, "b");
        assert_eq!(d.cursor, 0);
    }

    #[test]
    fn selection_cycle() {
        let mut d = ClarifyDialog::new(String::new(), vec!["x".into(), "y".into(), "z".into()]);
        d.select_down();
        assert_eq!(d.selected_choice, Some(0));
        d.select_down();
        assert_eq!(d.selected_choice, Some(1));
        d.select_down();
        assert_eq!(d.selected_choice, Some(2));
        d.select_down();
        assert_eq!(d.selected_choice, Some(0)); // wraps

        d.select_up();
        assert_eq!(d.selected_choice, Some(2)); // wraps back
        d.select_up();
        assert_eq!(d.selected_choice, Some(1));
    }

    #[test]
    fn select_on_empty_choices() {
        let mut d = ClarifyDialog::new(String::new(), vec![]);
        d.select_down();
        assert!(d.selected_choice.is_none());
        d.select_up();
        assert!(d.selected_choice.is_none());
    }

    #[test]
    fn response_returns_selected_choice() {
        let mut d = ClarifyDialog::new(String::new(), vec!["alpha".into(), "beta".into()]);
        d.insert_char('x');
        assert_eq!(d.response(), "x"); // freeform when no selection

        d.select_down();
        assert_eq!(d.response(), "alpha"); // choice takes precedence
    }
}
