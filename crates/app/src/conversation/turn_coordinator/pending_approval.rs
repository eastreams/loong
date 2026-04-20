use super::*;

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingApprovalInputDecision {
    RunOnce,
    SessionAuto,
    SessionFull,
    Cancel,
}

#[cfg(feature = "memory-sqlite")]
impl PendingApprovalInputDecision {
    pub(super) fn approval_decision(self) -> ApprovalDecision {
        match self {
            Self::RunOnce | Self::SessionAuto | Self::SessionFull => ApprovalDecision::ApproveOnce,
            Self::Cancel => ApprovalDecision::Deny,
        }
    }

    pub(super) fn session_mode(self) -> Option<ToolConsentMode> {
        match self {
            Self::RunOnce | Self::Cancel => None,
            Self::SessionAuto => Some(ToolConsentMode::Auto),
            Self::SessionFull => Some(ToolConsentMode::Full),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn normalize_pending_approval_control_input(input: &str) -> String {
    crate::conversation::turn_shared::normalize_approval_prompt_control_input(input)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn parse_pending_approval_input_decision(
    input: &str,
) -> Option<PendingApprovalInputDecision> {
    match parse_approval_prompt_action_input(input)? {
        ApprovalPromptActionId::Yes => Some(PendingApprovalInputDecision::RunOnce),
        ApprovalPromptActionId::Auto => Some(PendingApprovalInputDecision::SessionAuto),
        ApprovalPromptActionId::Full => Some(PendingApprovalInputDecision::SessionFull),
        ApprovalPromptActionId::Esc => Some(PendingApprovalInputDecision::Cancel),
    }
}
