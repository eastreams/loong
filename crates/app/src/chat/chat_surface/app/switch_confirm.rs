#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwitchConfirmState {
    pub(crate) pending_target_session_id: String,
}
