use super::events::UiEvent;
use super::state::UiState;

pub(super) fn reduce(state: &mut UiState, event: UiEvent) -> bool {
    match event {
        UiEvent::ComposerInput(ch) => {
            state.composer_text.push(ch);
            false
        }
        UiEvent::Backspace => {
            state.composer_text.pop();
            false
        }
        UiEvent::ExitRequested => true,
    }
}
