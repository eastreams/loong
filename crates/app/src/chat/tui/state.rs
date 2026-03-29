use super::execution_band::ExecutionBandSummary;
use super::execution_drawer::DrawerPayload;
use super::focus::FocusTarget;
use super::transcript::TranscriptState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UiState {
    pub(crate) session_id: String,
    pub(crate) drawer: Option<DrawerPayload>,
    pub(crate) focus_target: FocusTarget,
    pub(crate) composer_text: String,
    pub(crate) transcript: TranscriptState,
    pub(crate) execution_band: ExecutionBandSummary,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            drawer: None,
            focus_target: FocusTarget::Composer,
            composer_text: String::new(),
            transcript: TranscriptState::default(),
            execution_band: ExecutionBandSummary::default(),
        }
    }
}

impl UiState {
    pub(crate) fn with_session_id(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::chat::live_surface::CliChatLiveSurfaceSnapshot;
    use crate::chat::tui::execution_band::{
        project_execution_band_summary, render_execution_band_summary,
    };
    use crate::conversation::{ConversationTurnPhase, ExecutionLane};

    #[test]
    fn execution_band_projects_only_summary_state_by_default() {
        let summary = project_execution_band_summary(&CliChatLiveSurfaceSnapshot {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(1),
            lane: Some(ExecutionLane::Safe),
            tool_call_count: 2,
            message_count: Some(3),
            estimated_tokens: Some(512),
            draft_preview: Some("draft reply".to_owned()),
            tool_activity_lines: vec![
                "[running] shell (id=tool-1) - cargo test".to_owned(),
                "args: cargo test -p loongclaw-app".to_owned(),
                "[completed] git.status (id=tool-2) - clean".to_owned(),
            ],
        });
        let rendered = render_execution_band_summary(&summary);

        assert_eq!(summary.running_count, 1);
        assert_eq!(summary.pending_approval_count, 0);
        assert_eq!(summary.background_count, 0);
        assert!(
            rendered.contains("running 1"),
            "execution band should stay compact and surface the running count: {rendered}"
        );
        assert!(
            rendered.contains("latest [completed] git.status"),
            "execution band should surface the most recent terminal result: {rendered}"
        );
        assert!(
            !rendered.contains("args: cargo test"),
            "execution band should not spill full tool details into the quiet summary: {rendered}"
        );
    }
}
