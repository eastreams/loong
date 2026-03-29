use super::super::live_surface::CliChatLiveSurfaceSnapshot;
use super::execution_drawer::{is_raw_args_line, line_signals_approval};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExecutionBandSummary {
    pub(crate) running_count: usize,
    pub(crate) pending_approval_count: usize,
    pub(crate) latest_result: Option<String>,
    pub(crate) background_count: usize,
}

pub(crate) fn project_execution_band_summary(
    snapshot: &CliChatLiveSurfaceSnapshot,
) -> ExecutionBandSummary {
    let running_count = snapshot
        .tool_activity_lines
        .iter()
        .filter(|line| line.starts_with("[running]"))
        .count();
    let pending_approval_count = snapshot
        .tool_activity_lines
        .iter()
        .filter(|line| line.starts_with('['))
        .filter(|line| !is_raw_args_line(line))
        .filter(|line| line_signals_approval(line))
        .count();
    let latest_result = snapshot
        .tool_activity_lines
        .iter()
        .rev()
        .find(|line| line.starts_with('[') && !line.starts_with("[running]"))
        .cloned();

    ExecutionBandSummary {
        running_count,
        pending_approval_count,
        latest_result,
        background_count: 0,
    }
}

pub(crate) fn render_execution_band_summary(summary: &ExecutionBandSummary) -> String {
    let latest_result = summary.latest_result.as_deref().unwrap_or("none");
    format!(
        "running {} | approvals {} | background {} | latest {}",
        summary.running_count,
        summary.pending_approval_count,
        summary.background_count,
        latest_result
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::live_surface::CliChatLiveSurfaceSnapshot;
    use crate::conversation::{ConversationTurnPhase, ExecutionLane};

    #[test]
    fn approval_hints_count_as_pending_without_becoming_latest_result() {
        let summary = project_execution_band_summary(&CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Safe),
            tool_call_count: 1,
            message_count: Some(2),
            estimated_tokens: Some(128),
            draft_preview: None,
            tool_activity_lines: vec![
                "[interrupted] shell (id=tool-1) - needs operator confirmation".to_owned(),
                "yes / auto / full / esc".to_owned(),
                "args: cargo test -p loongclaw-app".to_owned(),
            ],
        });

        assert_eq!(summary.pending_approval_count, 1);
        assert_eq!(
            summary.latest_result.as_deref(),
            Some("[interrupted] shell (id=tool-1) - needs operator confirmation")
        );
    }
}
