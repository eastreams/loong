#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusTarget {
    Composer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UiState {
    pub(crate) session_id: String,
    pub(crate) drawer_open: bool,
    pub(crate) focus_target: FocusTarget,
    pub(crate) composer_text: String,
    pub(crate) status_message: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            drawer_open: false,
            focus_target: FocusTarget::Composer,
            composer_text: String::new(),
            status_message: "No tool activity yet.".to_owned(),
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
