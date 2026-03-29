use ratatui::layout::{Constraint, Layout, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ShellLayout {
    pub(super) header: Rect,
    pub(super) transcript: Rect,
    pub(super) execution_band: Rect,
    pub(super) drawer: Option<Rect>,
    pub(super) composer: Rect,
}

pub(super) fn split_shell(area: Rect, has_drawer: bool) -> ShellLayout {
    let constraints = if has_drawer {
        vec![
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Length(3),
        ]
    } else {
        vec![
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
            Constraint::Length(3),
        ]
    };

    let chunks = Layout::vertical(constraints).split(area);
    let mut chunks = chunks.iter().copied();
    let header = chunks.next().unwrap_or_default();
    let transcript = chunks.next().unwrap_or_default();
    let execution_band = chunks.next().unwrap_or_default();
    let (drawer, composer) = if has_drawer {
        (
            Some(chunks.next().unwrap_or_default()),
            chunks.next().unwrap_or_default(),
        )
    } else {
        (None, chunks.next().unwrap_or_default())
    };

    ShellLayout {
        header,
        transcript,
        execution_band,
        drawer,
        composer,
    }
}
