use crate::config::ProviderConfig;

pub(super) struct ProviderRequestPolicy {
    pub(super) timeout_ms: u64,
    pub(super) max_attempts: usize,
    pub(super) initial_backoff_ms: u64,
    pub(super) max_backoff_ms: u64,
}

impl ProviderRequestPolicy {
    pub(super) fn from_config(config: &ProviderConfig) -> Self {
        let timeout_ms = config.request_timeout_ms.clamp(1_000, 180_000);
        let max_attempts = config.retry_max_attempts.clamp(1, 8);
        let initial_backoff_ms = config.retry_initial_backoff_ms.clamp(50, 10_000);
        let max_backoff_ms = config
            .retry_max_backoff_ms
            .max(initial_backoff_ms)
            .min(30_000);

        Self {
            timeout_ms,
            max_attempts,
            initial_backoff_ms,
            max_backoff_ms,
        }
    }
}

pub(super) fn should_retry_status(status_code: u16) -> bool {
    matches!(status_code, 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504)
}

pub(super) fn should_retry_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

pub(super) fn next_backoff_ms(current: u64, max_backoff_ms: u64) -> u64 {
    current.saturating_mul(2).min(max_backoff_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_status_policy_covers_transient_failures() {
        assert!(should_retry_status(429));
        assert!(should_retry_status(503));
        assert!(!should_retry_status(401));
        assert!(!should_retry_status(422));
    }

    #[test]
    fn backoff_policy_respects_upper_bound() {
        assert_eq!(next_backoff_ms(100, 400), 200);
        assert_eq!(next_backoff_ms(400, 400), 400);
        assert_eq!(next_backoff_ms(500, 400), 400);
    }
}
