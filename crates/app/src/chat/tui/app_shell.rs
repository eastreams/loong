use super::state::UiState;

pub(crate) fn build_shell_bootstrap_state(session_id: &str) -> UiState {
    UiState::with_session_id(session_id)
}
