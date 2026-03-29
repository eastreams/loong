use ratatui::layout::{Constraint, Layout, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ShellLayout {
    pub(super) header: Rect,
    pub(super) transcript: Rect,
    pub(super) execution_band: Rect,
    pub(super) composer: Rect,
}

pub(super) fn split_shell(area: Rect) -> ShellLayout {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(3),
        Constraint::Length(3),
    ])
    .split(area);
    let mut chunks = chunks.iter().copied();
    let header = chunks.next().unwrap_or_default();
    let transcript = chunks.next().unwrap_or_default();
    let execution_band = chunks.next().unwrap_or_default();
    let composer = chunks.next().unwrap_or_default();

    ShellLayout {
        header,
        transcript,
        execution_band,
        composer,
    }
}
