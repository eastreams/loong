use crate::config::ProviderConfig;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, RETRY_AFTER};

const MIN_BACKOFF_MS: u64 = 50;

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

pub(super) fn resolve_status_retry_delay_ms(
    status_code: u16,
    response_headers: &HeaderMap,
    current_backoff_ms: u64,
    max_backoff_ms: u64,
) -> u64 {
    if !should_retry_status(status_code) {
        return current_backoff_ms.min(max_backoff_ms);
    }

    let hinted_ms = matches!(status_code, 429 | 503)
        .then(|| parse_retry_after_ms(response_headers))
        .flatten();
    hinted_ms
        .unwrap_or(current_backoff_ms)
        .clamp(MIN_BACKOFF_MS, max_backoff_ms)
}

pub(super) fn next_backoff_ms(current: u64, max_backoff_ms: u64) -> u64 {
    current.saturating_mul(2).min(max_backoff_ms)
}

fn parse_retry_after_ms(response_headers: &HeaderMap) -> Option<u64> {
    parse_retry_after_ms_at(response_headers, Utc::now())
}

fn parse_retry_after_ms_at(response_headers: &HeaderMap, now: DateTime<Utc>) -> Option<u64> {
    let raw = response_headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }

    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(seconds.saturating_mul(1_000));
    }

    let retry_at = DateTime::parse_from_rfc2822(raw)
        .or_else(|_| DateTime::parse_from_rfc3339(raw))
        .ok()?
        .with_timezone(&Utc);
    let wait = retry_at.signed_duration_since(now);
    if wait <= chrono::Duration::zero() {
        return Some(0);
    }
    let wait_std = wait.to_std().ok()?;
    match u64::try_from(wait_std.as_millis()) {
        Ok(ms) => Some(ms),
        Err(_) => Some(u64::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

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

    #[test]
    fn retry_delay_uses_retry_after_hint_for_rate_limit() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(
            resolve_status_retry_delay_ms(429, &headers, 100, 5_000),
            2_000
        );
    }

    #[test]
    fn retry_delay_clamps_retry_after_hint_to_max_backoff() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("120"));
        assert_eq!(
            resolve_status_retry_delay_ms(503, &headers, 100, 3_000),
            3_000
        );
    }

    #[test]
    fn retry_delay_falls_back_when_retry_after_hint_is_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("not-a-number"));
        assert_eq!(
            resolve_status_retry_delay_ms(429, &headers, 250, 3_000),
            250
        );
    }

    #[test]
    fn retry_delay_uses_retry_after_http_date_hint_when_present() {
        let now = DateTime::parse_from_rfc3339("2026-03-11T10:00:00Z")
            .expect("valid test timestamp")
            .with_timezone(&Utc);
        let retry_at = (now + chrono::Duration::seconds(2)).to_rfc2822();
        let mut headers = HeaderMap::new();
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_str(retry_at.as_str()).expect("valid retry-after header"),
        );
        assert_eq!(parse_retry_after_ms_at(&headers, now), Some(2_000));
    }

    #[test]
    fn retry_delay_clamps_past_retry_after_http_date_to_minimum_backoff() {
        let mut headers = HeaderMap::new();
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(
            resolve_status_retry_delay_ms(429, &headers, 250, 3_000),
            MIN_BACKOFF_MS
        );
    }

    #[test]
    fn retry_delay_ignores_retry_after_for_non_retryable_status() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("30"));
        assert_eq!(
            resolve_status_retry_delay_ms(401, &headers, 250, 3_000),
            250
        );
    }
}
