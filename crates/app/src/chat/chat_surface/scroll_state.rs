/// Generic scroll and selection state for a vertical list menu.
///
/// The selected index is optional so empty lists do not have to invent a fake
/// selection. `scroll_top` is always clamped through `ensure_visible`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScrollState {
    pub(crate) selected_idx: Option<usize>,
    pub(crate) scroll_top: usize,
}

impl ScrollState {
    pub(crate) const fn new() -> Self {
        Self {
            selected_idx: None,
            scroll_top: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.selected_idx = None;
        self.scroll_top = 0;
    }

    pub(crate) fn clamp_selection(&mut self, len: usize) {
        self.selected_idx = match len {
            0 => None,
            _ => Some(self.selected_idx.unwrap_or(0).min(len - 1)),
        };
        if len == 0 {
            self.scroll_top = 0;
        }
    }

    pub(crate) fn ensure_visible(&mut self, len: usize, visible_rows: usize) {
        if len == 0 || visible_rows == 0 {
            self.scroll_top = 0;
            return;
        }
        if let Some(selected_idx) = self.selected_idx {
            if selected_idx < self.scroll_top {
                self.scroll_top = selected_idx;
            } else {
                let bottom = self.scroll_top + visible_rows - 1;
                if selected_idx > bottom {
                    self.scroll_top = selected_idx + 1 - visible_rows;
                }
            }
        } else {
            self.scroll_top = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScrollState;

    #[test]
    fn clamp_and_visibility_keep_selection_onscreen() {
        let mut state = ScrollState::new();
        state.clamp_selection(10);
        assert_eq!(state.selected_idx, Some(0));

        state.selected_idx = Some(9);
        state.ensure_visible(10, 5);
        assert_eq!(state.scroll_top, 5);
    }

    #[test]
    fn empty_lists_reset_scroll_state() {
        let mut state = ScrollState {
            selected_idx: Some(3),
            scroll_top: 7,
        };
        state.clamp_selection(0);
        state.ensure_visible(0, 5);
        assert_eq!(state.selected_idx, None);
        assert_eq!(state.scroll_top, 0);
    }
}
