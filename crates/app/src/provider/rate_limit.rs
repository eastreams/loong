use std::time::Duration;

use reqwest::header::HeaderMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::policy::parse_retry_after_duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHeaderFamily {
    OpenAi,
    Anthropic,
    Bedrock,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitObservation {
    pub requests_limit: Option<u32>,
    pub requests_remaining: Option<u32>,
    pub requests_reset: Option<Duration>,
    pub tokens_limit: Option<u32>,
    pub tokens_remaining: Option<u32>,
    pub tokens_reset: Option<Duration>,
    pub retry_after: Option<Duration>,
    pub provider_family: ProviderHeaderFamily,
}

impl RateLimitObservation {
    pub fn cooldown_hint(&self) -> Option<Duration> {
        [self.retry_after, self.requests_reset, self.tokens_reset]
            .into_iter()
            .flatten()
            .max()
    }

    pub(crate) fn has_signal(&self) -> bool {
        self.has_provider_specific_signal() || self.retry_after.is_some()
    }

    fn has_provider_specific_signal(&self) -> bool {
        self.requests_limit.is_some()
            || self.requests_remaining.is_some()
            || self.requests_reset.is_some()
            || self.tokens_limit.is_some()
            || self.tokens_remaining.is_some()
            || self.tokens_reset.is_some()
    }
}

pub(super) fn parse_rate_limit_headers(headers: &HeaderMap) -> RateLimitObservation {
    parse_rate_limit_headers_at(headers, OffsetDateTime::now_utc())
}

fn parse_rate_limit_headers_at(headers: &HeaderMap, now: OffsetDateTime) -> RateLimitObservation {
    let openai = parse_openai_headers(headers, now);
    if openai.has_provider_specific_signal() {
        return openai;
    }

    let anthropic = parse_anthropic_headers(headers, now);
    if anthropic.has_provider_specific_signal() {
        return anthropic;
    }

    let bedrock = parse_bedrock_headers(headers, now);
    if bedrock.has_provider_specific_signal() {
        return bedrock;
    }

    RateLimitObservation {
        requests_limit: None,
        requests_remaining: None,
        requests_reset: None,
        tokens_limit: None,
        tokens_remaining: None,
        tokens_reset: None,
        retry_after: parse_retry_after_duration(headers),
        provider_family: ProviderHeaderFamily::Generic,
    }
}

fn parse_openai_headers(headers: &HeaderMap, now: OffsetDateTime) -> RateLimitObservation {
    RateLimitObservation {
        requests_limit: header_u32(headers, "x-ratelimit-limit-requests"),
        requests_remaining: header_u32(headers, "x-ratelimit-remaining-requests"),
        requests_reset: header_reset_duration(headers, "x-ratelimit-reset-requests", now),
        tokens_limit: header_u32(headers, "x-ratelimit-limit-tokens"),
        tokens_remaining: header_u32(headers, "x-ratelimit-remaining-tokens"),
        tokens_reset: header_reset_duration(headers, "x-ratelimit-reset-tokens", now),
        retry_after: parse_retry_after_duration(headers),
        provider_family: ProviderHeaderFamily::OpenAi,
    }
}

fn parse_anthropic_headers(headers: &HeaderMap, now: OffsetDateTime) -> RateLimitObservation {
    RateLimitObservation {
        requests_limit: header_u32(headers, "anthropic-ratelimit-requests-limit"),
        requests_remaining: header_u32(headers, "anthropic-ratelimit-requests-remaining"),
        requests_reset: header_reset_duration(headers, "anthropic-ratelimit-requests-reset", now),
        tokens_limit: header_u32(headers, "anthropic-ratelimit-tokens-limit"),
        tokens_remaining: header_u32(headers, "anthropic-ratelimit-tokens-remaining"),
        tokens_reset: header_reset_duration(headers, "anthropic-ratelimit-tokens-reset", now),
        retry_after: parse_retry_after_duration(headers),
        provider_family: ProviderHeaderFamily::Anthropic,
    }
}

fn parse_bedrock_headers(headers: &HeaderMap, now: OffsetDateTime) -> RateLimitObservation {
    RateLimitObservation {
        requests_limit: header_u32(headers, "x-amzn-ratelimit-limit"),
        requests_remaining: header_u32(headers, "x-amzn-ratelimit-remaining"),
        requests_reset: header_reset_duration(headers, "x-amzn-ratelimit-reset", now),
        tokens_limit: None,
        tokens_remaining: None,
        tokens_reset: None,
        retry_after: parse_retry_after_duration(headers),
        provider_family: ProviderHeaderFamily::Bedrock,
    }
}

fn header_u32(headers: &HeaderMap, name: &str) -> Option<u32> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u32>().ok())
}

fn header_reset_duration(headers: &HeaderMap, name: &str, now: OffsetDateTime) -> Option<Duration> {
    let raw = headers.get(name)?.to_str().ok()?.trim();
    parse_duration_hint_at(raw, now)
}

fn parse_duration_hint_at(raw: &str, now: OffsetDateTime) -> Option<Duration> {
    if raw.is_empty() {
        return None;
    }

    if let Ok(seconds) = raw.parse::<u64>() {
        return parse_integer_duration_hint(seconds, now);
    }

    if let Ok(seconds) = raw.parse::<f64>() {
        if !seconds.is_finite() || seconds.is_sign_negative() {
            return None;
        }
        return Duration::try_from_secs_f64(seconds).ok();
    }

    if let Some(duration) = parse_compound_duration(raw) {
        return Some(duration);
    }

    let reset_at = OffsetDateTime::parse(raw, &Rfc3339).ok()?;
    let remaining = reset_at - now;
    if remaining.is_negative() {
        return Some(Duration::ZERO);
    }
    Duration::try_from_secs_f64(remaining.as_seconds_f64()).ok()
}

fn parse_integer_duration_hint(raw: u64, now: OffsetDateTime) -> Option<Duration> {
    const EPOCH_SECONDS_THRESHOLD: u64 = 1_000_000_000;
    const EPOCH_MILLISECONDS_THRESHOLD: u64 = 1_000_000_000_000;

    if raw >= EPOCH_MILLISECONDS_THRESHOLD {
        let reset_at =
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(raw) * 1_000_000).ok()?;
        return duration_until(reset_at, now);
    }

    if raw >= EPOCH_SECONDS_THRESHOLD {
        let reset_at = OffsetDateTime::from_unix_timestamp(raw as i64).ok()?;
        return duration_until(reset_at, now);
    }

    Some(Duration::from_secs(raw))
}

fn duration_until(reset_at: OffsetDateTime, now: OffsetDateTime) -> Option<Duration> {
    let remaining = reset_at - now;
    if remaining.is_negative() {
        return Some(Duration::ZERO);
    }
    Duration::try_from_secs_f64(remaining.as_seconds_f64()).ok()
}

fn parse_compound_duration(raw: &str) -> Option<Duration> {
    let raw = raw.trim();
    let mut cursor = 0usize;
    let mut total_seconds = 0f64;

    while cursor < raw.len() {
        while cursor < raw.len() {
            let ch = raw[cursor..].chars().next()?;
            if ch.is_ascii_whitespace() || ch == ',' || ch == ';' {
                cursor += ch.len_utf8();
                continue;
            }
            break;
        }
        if cursor >= raw.len() {
            break;
        }
        let number_start = cursor;
        while cursor < raw.len() {
            let ch = raw[cursor..].chars().next()?;
            if ch.is_ascii_digit() || ch == '.' {
                cursor += ch.len_utf8();
                continue;
            }
            break;
        }
        if number_start == cursor {
            return None;
        }
        let value = raw[number_start..cursor].parse::<f64>().ok()?;
        while cursor < raw.len() {
            let ch = raw[cursor..].chars().next()?;
            if ch.is_ascii_whitespace() {
                cursor += ch.len_utf8();
                continue;
            }
            break;
        }
        let remaining = &raw[cursor..];
        let (unit, factor_seconds) = if remaining
            .get(..2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ms"))
        {
            ("ms", 0.001)
        } else if remaining
            .chars()
            .next()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'h'))
        {
            ("h", 3600.0)
        } else if remaining
            .chars()
            .next()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'m'))
        {
            ("m", 60.0)
        } else if remaining
            .chars()
            .next()
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'s'))
        {
            ("s", 1.0)
        } else {
            return None;
        };
        total_seconds += value * factor_seconds;
        cursor += unit.len();
    }

    if total_seconds.is_sign_negative() || !total_seconds.is_finite() {
        return None;
    }
    Duration::try_from_secs_f64(total_seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn parse_with_now(headers: &HeaderMap, now: OffsetDateTime) -> RateLimitObservation {
        parse_rate_limit_headers_at(headers, now)
    }

    #[test]
    fn parse_rate_limit_headers_detects_openai_family() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let reset_at = (now + time::Duration::seconds(30))
            .format(&Rfc3339)
            .expect("timestamp should format");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-limit-requests",
            HeaderValue::from_static("1000"),
        );
        headers.insert(
            "x-ratelimit-remaining-requests",
            HeaderValue::from_static("42"),
        );
        headers.insert(
            "x-ratelimit-reset-requests",
            HeaderValue::from_str(reset_at.as_str()).expect("header value"),
        );
        headers.insert(
            "x-ratelimit-limit-tokens",
            HeaderValue::from_static("20000"),
        );
        headers.insert(
            "x-ratelimit-remaining-tokens",
            HeaderValue::from_static("1200"),
        );
        headers.insert("x-ratelimit-reset-tokens", HeaderValue::from_static("45"));

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::OpenAi);
        assert_eq!(observation.requests_limit, Some(1000));
        assert_eq!(observation.requests_remaining, Some(42));
        assert_eq!(observation.requests_reset, Some(Duration::from_secs(30)));
        assert_eq!(observation.tokens_limit, Some(20000));
        assert_eq!(observation.tokens_remaining, Some(1200));
        assert_eq!(observation.tokens_reset, Some(Duration::from_secs(45)));
        assert_eq!(observation.cooldown_hint(), Some(Duration::from_secs(45)));
    }

    #[test]
    fn parse_rate_limit_headers_detects_openai_duration_string_resets() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-limit-requests",
            HeaderValue::from_static("1000"),
        );
        headers.insert("x-ratelimit-reset-requests", HeaderValue::from_static("1s"));
        headers.insert("x-ratelimit-reset-tokens", HeaderValue::from_static("6m0s"));

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::OpenAi);
        assert_eq!(observation.requests_reset, Some(Duration::from_secs(1)));
        assert_eq!(observation.tokens_reset, Some(Duration::from_secs(360)));
        assert_eq!(observation.cooldown_hint(), Some(Duration::from_secs(360)));
    }

    #[test]
    fn parse_rate_limit_headers_accepts_spaced_and_uppercase_duration_strings() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-limit-requests",
            HeaderValue::from_static("1000"),
        );
        headers.insert(
            "x-ratelimit-reset-requests",
            HeaderValue::from_static("1H 30M"),
        );
        headers.insert(
            "x-ratelimit-reset-tokens",
            HeaderValue::from_static("1m 250MS"),
        );

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::OpenAi);
        assert_eq!(observation.requests_reset, Some(Duration::from_secs(5_400)));
        assert_eq!(
            observation.tokens_reset,
            Some(Duration::from_millis(60_250))
        );
        assert_eq!(
            observation.cooldown_hint(),
            Some(Duration::from_secs(5_400))
        );
    }

    #[test]
    fn parse_rate_limit_headers_treats_large_numeric_reset_as_epoch_seconds() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-limit-requests",
            HeaderValue::from_static("1000"),
        );
        headers.insert(
            "x-ratelimit-reset-requests",
            HeaderValue::from_static("1700000300"),
        );

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::OpenAi);
        assert_eq!(observation.requests_reset, Some(Duration::from_secs(300)));
    }

    #[test]
    fn parse_rate_limit_headers_ignores_malformed_reset_header() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-limit-requests",
            HeaderValue::from_static("1000"),
        );
        headers.insert(
            "x-ratelimit-reset-requests",
            HeaderValue::from_static("not-a-duration"),
        );

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::OpenAi);
        assert_eq!(observation.requests_reset, None);
    }

    #[test]
    fn parse_rate_limit_headers_detects_anthropic_family() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let mut headers = HeaderMap::new();
        headers.insert(
            "anthropic-ratelimit-requests-limit",
            HeaderValue::from_static("100"),
        );
        headers.insert(
            "anthropic-ratelimit-requests-remaining",
            HeaderValue::from_static("3"),
        );
        headers.insert(
            "anthropic-ratelimit-requests-reset",
            HeaderValue::from_static("12"),
        );
        headers.insert(
            "anthropic-ratelimit-tokens-limit",
            HeaderValue::from_static("1000"),
        );
        headers.insert(
            "anthropic-ratelimit-tokens-remaining",
            HeaderValue::from_static("250"),
        );
        headers.insert(
            "anthropic-ratelimit-tokens-reset",
            HeaderValue::from_static("30"),
        );

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::Anthropic);
        assert_eq!(observation.requests_limit, Some(100));
        assert_eq!(observation.requests_remaining, Some(3));
        assert_eq!(observation.requests_reset, Some(Duration::from_secs(12)));
        assert_eq!(observation.tokens_limit, Some(1000));
        assert_eq!(observation.tokens_remaining, Some(250));
        assert_eq!(observation.tokens_reset, Some(Duration::from_secs(30)));
        assert_eq!(observation.cooldown_hint(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn parse_rate_limit_headers_detects_bedrock_family() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let mut headers = HeaderMap::new();
        headers.insert("x-amzn-ratelimit-limit", HeaderValue::from_static("50"));
        headers.insert("x-amzn-ratelimit-remaining", HeaderValue::from_static("2"));
        headers.insert("retry-after", HeaderValue::from_static("8"));

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::Bedrock);
        assert_eq!(observation.requests_limit, Some(50));
        assert_eq!(observation.requests_remaining, Some(2));
        assert_eq!(observation.retry_after, Some(Duration::from_secs(8)));
        assert_eq!(observation.cooldown_hint(), Some(Duration::from_secs(8)));
    }

    #[test]
    fn parse_rate_limit_headers_falls_back_to_generic_retry_after() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("4"));

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::Generic);
        assert_eq!(observation.retry_after, Some(Duration::from_secs(4)));
        assert_eq!(observation.cooldown_hint(), Some(Duration::from_secs(4)));
    }

    #[test]
    fn parse_rate_limit_headers_handles_missing_headers() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("test timestamp should be valid");
        let headers = HeaderMap::new();

        let observation = parse_with_now(&headers, now);

        assert_eq!(observation.provider_family, ProviderHeaderFamily::Generic);
        assert!(!observation.has_signal());
        assert!(observation.cooldown_hint().is_none());
    }
}
