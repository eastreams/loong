use super::utils::*;
use ratatui::style::Color;
use std::time::Duration;

pub(super) const PENDING_TOOL_ANIMATION_FRAME_MS: u64 = 90;
pub(super) const PENDING_TOOL_LABEL_COLORS: [Color; 6] = [
    SURFACE_DIM_GRAY,
    SURFACE_GRAY,
    SURFACE_ACCENT,
    SURFACE_CYAN,
    Color::White,
    SURFACE_CYAN,
];
pub(super) const PENDING_TOOL_BODY_COLORS: [Color; 6] = [
    SURFACE_GRAY,
    SURFACE_ACCENT,
    SURFACE_CYAN,
    Color::White,
    SURFACE_CYAN,
    SURFACE_ACCENT,
];

pub(super) fn pending_tool_animation_frame(start: std::time::Instant) -> usize {
    if reduced_motion_enabled() {
        return PENDING_TOOL_LABEL_COLORS.len().saturating_sub(2);
    }
    pending_tool_animation_frame_for_elapsed(start.elapsed())
}

pub(super) fn pending_tool_animation_frame_for_elapsed(elapsed: Duration) -> usize {
    let frame_count = PENDING_TOOL_LABEL_COLORS.len().max(1) as u64;
    ((elapsed.as_millis() as u64 / PENDING_TOOL_ANIMATION_FRAME_MS.max(1)) % frame_count) as usize
}

pub(super) fn pending_tool_label_color(start: std::time::Instant) -> Color {
    let frame = pending_tool_animation_frame(start);
    *PENDING_TOOL_LABEL_COLORS
        .get(frame)
        .unwrap_or(&SURFACE_CYAN)
}

pub(super) fn pending_tool_body_color(start: std::time::Instant) -> Color {
    let frame = pending_tool_animation_frame(start);
    *PENDING_TOOL_BODY_COLORS.get(frame).unwrap_or(&Color::White)
}
