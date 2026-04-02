use ratatui::layout::{Constraint, Direction, Layout, Rect};

const GUIDED_SPINE_WIDTH: u16 = 15;

pub(crate) struct OnboardLayoutAreas {
    pub header: Rect,
    #[allow(dead_code)] // used in tests; will be rendered when wide spine is enabled
    pub spine: Rect,
    pub content: Rect,
    pub footer: Rect,
}

/// Compute the four layout areas for the onboard wizard.
///
/// `wide_spine = true` allocates a compact spine on the left; otherwise the
/// spine is `Rect::ZERO` and the full body width goes to `content`.
#[allow(clippy::indexing_slicing)] // Layout::split returns exactly as many Rects as constraints
pub(crate) fn compute_layout(area: Rect, wide_spine: bool) -> OnboardLayoutAreas {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(5),    // body (spine + content)
            Constraint::Length(1), // footer
        ])
        .split(area);

    let header = vertical[0];
    let body = vertical[1];
    let footer = vertical[2];

    let (spine, content) = if wide_spine {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(GUIDED_SPINE_WIDTH), // spine
                Constraint::Min(20),                    // content
            ])
            .split(body);
        (cols[0], cols[1])
    } else {
        // Collapsed: no spine column, breadcrumb rendered inside content
        (Rect::ZERO, body)
    };

    OnboardLayoutAreas {
        header,
        spine,
        content,
        footer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_layout_allocates_spine_column() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = compute_layout(area, true);
        assert_eq!(layout.spine.width, GUIDED_SPINE_WIDTH);
        assert!(layout.content.width > 40);
        assert_eq!(layout.header.height, 1);
        assert_eq!(layout.footer.height, 1);
    }

    #[test]
    fn narrow_layout_has_zero_spine() {
        let area = Rect::new(0, 0, 50, 24);
        let layout = compute_layout(area, false);
        assert_eq!(layout.spine, Rect::ZERO);
        assert_eq!(layout.content.width, 50);
    }
}
