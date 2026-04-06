#[cfg(feature = "memory-sqlite")]
pub mod recovery;

pub mod presentation;

#[cfg(feature = "memory-sqlite")]
pub mod repository;

#[allow(dead_code)]
pub(crate) const DELEGATE_CANCEL_REQUESTED_EVENT_KIND: &str = "delegate_cancel_requested";
#[allow(dead_code)]
pub(crate) const DELEGATE_CANCELLED_EVENT_KIND: &str = "delegate_cancelled";
#[allow(dead_code)]
pub(crate) const DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED: &str = "operator_requested";
#[allow(dead_code)]
pub(crate) const DELEGATE_CANCELLED_ERROR_PREFIX: &str = "delegate_cancelled:";

#[allow(dead_code)]
pub(crate) fn delegate_cancelled_error(reason: &str) -> String {
    format!(
        "{DELEGATE_CANCELLED_ERROR_PREFIX} {}",
        reason.trim().trim_matches(':')
    )
}

#[allow(dead_code)]
pub(crate) fn parse_delegate_cancelled_reason(error: &str) -> Option<String> {
    error
        .strip_prefix(DELEGATE_CANCELLED_ERROR_PREFIX)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
