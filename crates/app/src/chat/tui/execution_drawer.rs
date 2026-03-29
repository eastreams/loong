use ratatui::text::Line;

use super::super::live_surface::CliChatLiveSurfaceSnapshot;
use crate::conversation::ConversationTurnPhase;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawerPayloadKind {
    ToolOutput,
    ShellLog,
    Diff,
    ApprovalDetail,
    ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrawerPayload {
    pub(crate) kind: DrawerPayloadKind,
    pub(crate) title: String,
    pub(crate) lines: Vec<String>,
}

impl DrawerPayload {
    pub(crate) fn new(
        kind: DrawerPayloadKind,
        title: impl Into<String>,
        lines: Vec<String>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            lines,
        }
    }
}

pub(crate) fn drawer_payload_from_live_surface(
    snapshot: &CliChatLiveSurfaceSnapshot,
) -> Option<DrawerPayload> {
    if snapshot
        .tool_activity_lines
        .iter()
        .any(|line| line_signals_approval(line))
    {
        return Some(DrawerPayload::new(
            DrawerPayloadKind::ApprovalDetail,
            "Approval detail",
            snapshot.tool_activity_lines.clone(),
        ));
    }

    if snapshot.phase == ConversationTurnPhase::Failed
        || snapshot
            .tool_activity_lines
            .iter()
            .any(|line| line_signals_error(line))
    {
        return Some(DrawerPayload::new(
            DrawerPayloadKind::ErrorDetail,
            "Error detail",
            snapshot.tool_activity_lines.clone(),
        ));
    }

    None
}

pub(crate) fn render_execution_drawer_lines(payload: &DrawerPayload) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(payload.title.clone())];
    lines.extend(payload.lines.iter().cloned().map(Line::from));
    lines
}

fn line_signals_approval(line: &str) -> bool {
    let normalized = line.to_ascii_lowercase();
    normalized.contains("operator confirmation")
        || normalized.contains("approval")
        || normalized.contains("yes / auto / full / esc")
}

fn line_signals_error(line: &str) -> bool {
    let normalized = line.to_ascii_lowercase();
    line.starts_with("[interrupted]")
        || normalized.contains("failed")
        || normalized.contains("error")
        || normalized.contains("exited with code")
}
