#[cfg(feature = "memory-sqlite")]
pub mod recovery;

#[cfg(feature = "memory-sqlite")]
pub mod repository;

pub(crate) const DELEGATE_CANCEL_REQUESTED_EVENT_KIND: &str = "delegate_cancel_requested";
pub(crate) const DELEGATE_CANCELLED_EVENT_KIND: &str = "delegate_cancelled";
pub(crate) const DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED: &str = "operator_requested";
pub(crate) const DELEGATE_CANCELLED_ERROR_PREFIX: &str = "delegate_cancelled:";

pub(crate) fn delegate_cancelled_error(reason: &str) -> String {
    format!(
        "{DELEGATE_CANCELLED_ERROR_PREFIX} {}",
        reason.trim().trim_matches(':')
    )
}
