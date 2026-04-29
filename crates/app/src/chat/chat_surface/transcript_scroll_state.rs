#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptScrollState {
    offset_from_bottom: u16,
    last_scroll_start: usize,
    follow_tail: bool,
    snap_on_next_render: bool,
}

impl Default for TranscriptScrollState {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptScrollState {
    pub(crate) const fn new() -> Self {
        Self {
            offset_from_bottom: 0,
            last_scroll_start: 0,
            follow_tail: true,
            snap_on_next_render: true,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.offset_from_bottom = 0;
        self.last_scroll_start = 0;
        self.follow_tail = true;
        self.snap_on_next_render = true;
    }

    pub(crate) fn reset_for_empty_render(&mut self) {
        self.offset_from_bottom = 0;
        self.last_scroll_start = 0;
        self.follow_tail = true;
        self.snap_on_next_render = false;
    }

    pub(crate) fn note_cache_invalidated(&mut self) {
        if self.follow_tail {
            self.snap_on_next_render = true;
        }
    }

    pub(crate) fn prepare_for_appended_content(&mut self) {
        self.offset_from_bottom = 0;
    }

    pub(crate) fn raw_scroll_start(&self, max_scroll_start: usize) -> usize {
        max_scroll_start.saturating_sub(self.offset_from_bottom as usize)
    }

    pub(crate) const fn follow_tail(&self) -> bool {
        self.follow_tail
    }

    pub(crate) const fn snap_on_next_render(&self) -> bool {
        self.snap_on_next_render
    }

    pub(crate) const fn last_scroll_start(&self) -> usize {
        self.last_scroll_start
    }

    pub(crate) fn apply_rendered_scroll_start(
        &mut self,
        max_scroll_start: usize,
        scroll_start: usize,
    ) {
        self.last_scroll_start = scroll_start.min(max_scroll_start);
        self.follow_tail = self.last_scroll_start == max_scroll_start;
        self.offset_from_bottom = max_scroll_start.saturating_sub(self.last_scroll_start) as u16;
        self.snap_on_next_render = false;
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> u16 {
        self.offset_from_bottom
    }

    #[cfg(test)]
    pub(crate) fn set_scroll_offset_for_test(&mut self, value: u16) {
        self.offset_from_bottom = value;
        self.follow_tail = value == 0;
    }

    #[cfg(test)]
    pub(crate) fn set_last_scroll_start_for_test(&mut self, value: usize) {
        self.last_scroll_start = value;
    }

    #[cfg(test)]
    pub(crate) fn set_snap_on_next_render_for_test(&mut self, value: bool) {
        self.snap_on_next_render = value;
    }

    pub(crate) fn scroll_line_up(&mut self) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_add(1);
        self.follow_tail = self.offset_from_bottom == 0;
        self.snap_on_next_render = true;
    }

    pub(crate) fn scroll_line_down(&mut self) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_sub(1);
        self.follow_tail = self.offset_from_bottom == 0;
        self.snap_on_next_render = true;
    }

    pub(crate) fn scroll_page_up(&mut self, step: u16) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_add(step);
        self.follow_tail = self.offset_from_bottom == 0;
        self.snap_on_next_render = true;
    }

    pub(crate) fn scroll_page_down(&mut self, step: u16) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_sub(step);
        self.follow_tail = self.offset_from_bottom == 0;
        self.snap_on_next_render = true;
    }

    pub(crate) fn jump_home(&mut self) {
        self.offset_from_bottom = u16::MAX;
        self.follow_tail = false;
        self.snap_on_next_render = true;
    }

    pub(crate) fn jump_end(&mut self) {
        self.offset_from_bottom = 0;
        self.follow_tail = true;
        self.snap_on_next_render = true;
    }
}

#[cfg(test)]
mod tests {
    use super::TranscriptScrollState;

    #[test]
    fn reset_and_empty_render_restore_tail_state() {
        let mut state = TranscriptScrollState::new();
        state.scroll_page_up(4);
        state.reset();
        assert_eq!(state.scroll_offset(), 0);
        assert!(state.follow_tail());
        assert!(state.snap_on_next_render());

        state.scroll_page_up(4);
        state.reset_for_empty_render();
        assert_eq!(state.scroll_offset(), 0);
        assert!(state.follow_tail());
        assert!(!state.snap_on_next_render());
    }

    #[test]
    fn apply_rendered_scroll_start_tracks_tail_and_offset() {
        let mut state = TranscriptScrollState::new();
        state.apply_rendered_scroll_start(10, 4);
        assert_eq!(state.last_scroll_start(), 4);
        assert_eq!(state.scroll_offset(), 6);
        assert!(!state.follow_tail());

        state.apply_rendered_scroll_start(10, 10);
        assert_eq!(state.scroll_offset(), 0);
        assert!(state.follow_tail());
    }
}
