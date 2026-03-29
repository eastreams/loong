use super::events::UiEvent;
use super::execution_band::project_execution_band_summary;
use super::execution_drawer::drawer_payload_from_live_surface;
use super::focus::FocusTarget;
use super::state::UiState;
use super::transcript::TranscriptRole;

pub(super) fn reduce(state: &mut UiState, event: UiEvent) -> bool {
    match event {
        UiEvent::ComposerInput(ch) => {
            if state.focus_target != FocusTarget::Composer {
                return false;
            }
            state.composer_text.push(ch);
            false
        }
        UiEvent::Backspace => {
            if state.focus_target != FocusTarget::Composer {
                return false;
            }
            state.composer_text.pop();
            false
        }
        UiEvent::AppendUserMessage(text) => {
            state.transcript.push_message(TranscriptRole::User, text);
            false
        }
        UiEvent::UpdateAssistantStream(text) => {
            state.transcript.update_assistant_stream(text);
            false
        }
        UiEvent::FinalizeAssistantMessage(text) => {
            state.transcript.finalize_assistant_message(text);
            false
        }
        UiEvent::UpdateLiveSurface(snapshot) => {
            state.execution_band = project_execution_band_summary(&snapshot);
            if let Some(payload) = drawer_payload_from_live_surface(&snapshot) {
                state.focus_target = FocusTarget::Drawer;
                state.drawer = Some(payload);
            }
            false
        }
        UiEvent::OpenDrawer(payload) => {
            state.drawer = Some(payload);
            false
        }
        UiEvent::CloseDrawer => {
            state.drawer = None;
            state.focus_target = FocusTarget::Composer;
            false
        }
        UiEvent::FocusDrawer => {
            if state.drawer.is_some() {
                state.focus_target = FocusTarget::Drawer;
            }
            false
        }
        UiEvent::FocusComposer => {
            state.focus_target = FocusTarget::Composer;
            false
        }
        UiEvent::ExitRequested => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::live_surface::CliChatLiveSurfaceSnapshot;
    use crate::chat::tui::execution_drawer::{DrawerPayload, DrawerPayloadKind};
    use crate::chat::tui::focus::FocusTarget;
    use crate::chat::tui::transcript::TranscriptRole;
    use crate::conversation::{ConversationTurnPhase, ExecutionLane};

    #[test]
    fn transcript_appends_user_and_assistant_messages_in_order() {
        let mut state = UiState::with_session_id("default");

        assert!(!reduce(
            &mut state,
            UiEvent::AppendUserMessage("summarize the repo".to_owned()),
        ));
        assert!(!reduce(
            &mut state,
            UiEvent::FinalizeAssistantMessage("start with the chat shell".to_owned()),
        ));

        assert!(matches!(
            state.transcript.entries(),
            [user, assistant]
                if user.role == TranscriptRole::User
                    && user.text == "summarize the repo"
                    && !user.streaming
                    && assistant.role == TranscriptRole::Assistant
                    && assistant.text == "start with the chat shell"
                    && !assistant.streaming
        ));
    }

    #[test]
    fn partial_assistant_output_updates_streaming_tail_without_duplication() {
        let mut state = UiState::with_session_id("default");

        assert!(!reduce(
            &mut state,
            UiEvent::UpdateAssistantStream("start".to_owned()),
        ));
        assert!(!reduce(
            &mut state,
            UiEvent::UpdateAssistantStream("start with".to_owned()),
        ));
        assert!(!reduce(
            &mut state,
            UiEvent::FinalizeAssistantMessage("start with".to_owned()),
        ));

        assert!(
            matches!(
                state.transcript.entries(),
                [assistant]
                    if assistant.role == TranscriptRole::Assistant
                        && assistant.text == "start with"
                        && !assistant.streaming
            ),
            "finalizing the assistant response should clear the streaming flag"
        );
    }

    #[test]
    fn live_surface_updates_execution_band_summary() {
        let mut state = UiState::with_session_id("default");

        assert!(!reduce(
            &mut state,
            UiEvent::UpdateLiveSurface(CliChatLiveSurfaceSnapshot {
                phase: ConversationTurnPhase::RunningTools,
                provider_round: Some(1),
                lane: Some(ExecutionLane::Safe),
                tool_call_count: 2,
                message_count: Some(3),
                estimated_tokens: Some(256),
                draft_preview: Some("draft".to_owned()),
                tool_activity_lines: vec![
                    "[running] shell (id=tool-1) - cargo test".to_owned(),
                    "[completed] git.status (id=tool-2) - clean".to_owned(),
                ],
            }),
        ));

        assert_eq!(state.execution_band.running_count, 1);
        assert_eq!(
            state.execution_band.latest_result.as_deref(),
            Some("[completed] git.status (id=tool-2) - clean")
        );
    }

    #[test]
    fn approval_snapshots_auto_open_drawer() {
        let mut state = UiState::with_session_id("default");

        assert!(!reduce(
            &mut state,
            UiEvent::UpdateLiveSurface(CliChatLiveSurfaceSnapshot {
                phase: ConversationTurnPhase::RunningTools,
                provider_round: Some(1),
                lane: Some(ExecutionLane::Safe),
                tool_call_count: 1,
                message_count: Some(2),
                estimated_tokens: Some(128),
                draft_preview: None,
                tool_activity_lines: vec![
                    "[interrupted] provider.switch (id=tool-1) - needs operator confirmation"
                        .to_owned(),
                    "yes / auto / full / esc".to_owned(),
                ],
            }),
        ));

        assert!(matches!(
            state.drawer.as_ref(),
            Some(payload) if payload.kind == DrawerPayloadKind::ApprovalDetail
        ));
        assert_eq!(state.focus_target, FocusTarget::Drawer);
    }

    #[test]
    fn error_snapshots_auto_open_drawer() {
        let mut state = UiState::with_session_id("default");

        assert!(!reduce(
            &mut state,
            UiEvent::UpdateLiveSurface(CliChatLiveSurfaceSnapshot {
                phase: ConversationTurnPhase::Failed,
                provider_round: Some(1),
                lane: Some(ExecutionLane::Safe),
                tool_call_count: 1,
                message_count: Some(2),
                estimated_tokens: Some(128),
                draft_preview: None,
                tool_activity_lines: vec![
                    "[interrupted] shell (id=tool-1) - command exited with code 1".to_owned(),
                ],
            }),
        ));

        assert!(matches!(
            state.drawer.as_ref(),
            Some(payload) if payload.kind == DrawerPayloadKind::ErrorDetail
        ));
        assert_eq!(state.focus_target, FocusTarget::Drawer);
    }

    #[test]
    fn ordinary_running_tool_activity_does_not_auto_open_drawer() {
        let mut state = UiState::with_session_id("default");

        assert!(!reduce(
            &mut state,
            UiEvent::UpdateLiveSurface(CliChatLiveSurfaceSnapshot {
                phase: ConversationTurnPhase::RunningTools,
                provider_round: Some(1),
                lane: Some(ExecutionLane::Fast),
                tool_call_count: 1,
                message_count: Some(2),
                estimated_tokens: Some(128),
                draft_preview: None,
                tool_activity_lines: vec![
                    "[running] shell (id=tool-1) - cargo test".to_owned(),
                    "args: cargo test -p loongclaw-app".to_owned(),
                ],
            }),
        ));

        assert!(state.drawer.is_none());
        assert_eq!(state.focus_target, FocusTarget::Composer);
    }

    #[test]
    fn manual_drawer_expand_collapse_and_focus_transitions_are_explicit() {
        let mut state = UiState::with_session_id("default");
        let payload = DrawerPayload::new(
            DrawerPayloadKind::ToolOutput,
            "tool output",
            vec!["cargo test -p loongclaw-app".to_owned()],
        );

        assert!(!reduce(&mut state, UiEvent::OpenDrawer(payload)));
        assert!(matches!(
            state.drawer.as_ref(),
            Some(current) if current.kind == DrawerPayloadKind::ToolOutput
        ));
        assert_eq!(
            state.focus_target,
            FocusTarget::Composer,
            "opening the drawer manually should not steal focus"
        );

        assert!(!reduce(&mut state, UiEvent::FocusDrawer));
        assert_eq!(state.focus_target, FocusTarget::Drawer);

        assert!(!reduce(&mut state, UiEvent::FocusComposer));
        assert_eq!(state.focus_target, FocusTarget::Composer);

        assert!(!reduce(&mut state, UiEvent::CloseDrawer));
        assert!(state.drawer.is_none());
        assert_eq!(state.focus_target, FocusTarget::Composer);
    }
}
