#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UiEvent {
    ComposerInput(char),
    Backspace,
    ExitRequested,
}
