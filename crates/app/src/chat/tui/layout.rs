use ratatui::layout::{Constraint, Layout, Rect};

/// Layout areas for the hermes-lite-inspired TUI shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ShellAreas {
    pub(super) history: Rect,
    pub(super) separator1: Rect,
    pub(super) spinner: Rect,
    pub(super) separator2: Rect,
    pub(super) input: Rect,
    pub(super) status_bar: Rect,
}

/// Compute the main layout areas for the TUI shell.
///
/// `input_height` is the raw height (including borders); this function clamps
/// it to the `[3, 12]` range before feeding it to the layout solver.
pub(super) fn compute(area: Rect, input_height: u16) -> ShellAreas {
    let clamped = input_height.clamp(3, 12);

    let chunks = Layout::vertical([
        Constraint::Min(6),          // history
        Constraint::Length(1),       // separator
        Constraint::Length(1),       // spinner / phase line
        Constraint::Length(1),       // separator
        Constraint::Length(clamped), // input
        Constraint::Length(1),       // status bar
    ])
    .split(area);

    // Safe access via iterator to avoid direct indexing.
    let mut chunks = chunks.iter().copied();

    ShellAreas {
        history: chunks.next().unwrap_or_default(),
        separator1: chunks.next().unwrap_or_default(),
        spinner: chunks.next().unwrap_or_default(),
        separator2: chunks.next().unwrap_or_default(),
        input: chunks.next().unwrap_or_default(),
        status_bar: chunks.next().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_produces_correct_areas() {
        let area = Rect::new(0, 0, 80, 30);
        let areas = compute(area, 3);

        // Input clamped to 3
        assert_eq!(areas.input.height, 3);
        // Separators and spinner are each 1
        assert_eq!(areas.separator1.height, 1);
        assert_eq!(areas.spinner.height, 1);
        assert_eq!(areas.separator2.height, 1);
        assert_eq!(areas.status_bar.height, 1);
        // History gets the remainder: 30 - 3 - 1 - 1 - 1 - 1 = 23
        assert_eq!(areas.history.height, 23);
    }

    #[test]
    fn input_height_clamped_low() {
        let area = Rect::new(0, 0, 80, 30);
        let areas = compute(area, 1);
        assert_eq!(areas.input.height, 3);
    }

    #[test]
    fn input_height_clamped_high() {
        let area = Rect::new(0, 0, 80, 30);
        let areas = compute(area, 20);
        assert_eq!(areas.input.height, 12);
    }

    #[test]
    fn all_areas_share_full_width() {
        let area = Rect::new(0, 0, 120, 40);
        let areas = compute(area, 5);
        assert_eq!(areas.history.width, 120);
        assert_eq!(areas.separator1.width, 120);
        assert_eq!(areas.spinner.width, 120);
        assert_eq!(areas.separator2.width, 120);
        assert_eq!(areas.input.width, 120);
        assert_eq!(areas.status_bar.width, 120);
    }

    #[test]
    fn zero_size_area_does_not_panic() {
        let area = Rect::new(0, 0, 0, 0);
        let areas = compute(area, 5);
        // Should not panic; all areas may be zero-sized.
        assert_eq!(areas.history.width, 0);
    }
}
