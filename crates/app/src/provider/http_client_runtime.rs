use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::CliResult;

use super::policy;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderHttpClientRuntimeMetricsSnapshot {
    pub cache_entry_count: usize,
    pub cache_hit_count: u64,
    pub cache_miss_count: u64,
    pub built_client_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ProviderHttpClientCacheKey {
    timeout_ms: u64,
}

impl ProviderHttpClientCacheKey {
    fn from_request_policy(request_policy: &policy::ProviderRequestPolicy) -> Self {
        Self {
            timeout_ms: request_policy.timeout_ms,
        }
    }
}

#[derive(Debug, Default)]
struct ProviderHttpClientCache {
    entries: HashMap<ProviderHttpClientCacheKey, reqwest::Client>,
}

#[derive(Debug, Default)]
struct ProviderHttpClientRuntimeMetrics {
    cache_hit_count: u64,
    cache_miss_count: u64,
    built_client_count: u64,
}

fn with_provider_http_client_cache<R>(run: impl FnOnce(&mut ProviderHttpClientCache) -> R) -> R {
    let cache =
        PROVIDER_HTTP_CLIENT_CACHE.get_or_init(|| Mutex::new(ProviderHttpClientCache::default()));
    let mut guard = match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    run(&mut guard)
}

fn with_provider_http_client_runtime_metrics<R>(
    run: impl FnOnce(&mut ProviderHttpClientRuntimeMetrics) -> R,
) -> R {
    let metrics = PROVIDER_HTTP_CLIENT_RUNTIME_METRICS
        .get_or_init(|| Mutex::new(ProviderHttpClientRuntimeMetrics::default()));
    let mut guard = match metrics.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    run(&mut guard)
}

fn load_or_build_provider_http_client(
    cache_key: ProviderHttpClientCacheKey,
) -> CliResult<reqwest::Client> {
    with_provider_http_client_cache(|cache| {
        if let Some(cached_client) = cache.entries.get(&cache_key) {
            record_provider_http_client_cache_hit();
            return Ok(cached_client.clone());
        }

        record_provider_http_client_cache_miss();
        let built_client = build_provider_http_client(cache_key)?;
        let cached_client = built_client.clone();
        cache.entries.insert(cache_key, cached_client);

        Ok(built_client)
    })
}

fn build_provider_http_client(cache_key: ProviderHttpClientCacheKey) -> CliResult<reqwest::Client> {
    let timeout = Duration::from_millis(cache_key.timeout_ms);
    let client_builder = reqwest::Client::builder();
    let timeout_builder = client_builder.timeout(timeout);
    let built_client = timeout_builder
        .build()
        .map_err(|error| format!("build provider http client failed: {error}"))?;

    record_provider_http_client_build();

    Ok(built_client)
}

pub(super) fn build_http_client(
    request_policy: &policy::ProviderRequestPolicy,
) -> CliResult<reqwest::Client> {
    let cache_key = ProviderHttpClientCacheKey::from_request_policy(request_policy);

    load_or_build_provider_http_client(cache_key)
}

static PROVIDER_HTTP_CLIENT_CACHE: OnceLock<Mutex<ProviderHttpClientCache>> = OnceLock::new();
static PROVIDER_HTTP_CLIENT_RUNTIME_METRICS: OnceLock<Mutex<ProviderHttpClientRuntimeMetrics>> =
    OnceLock::new();

pub fn provider_http_client_runtime_metrics_snapshot() -> ProviderHttpClientRuntimeMetricsSnapshot {
    let cache_entry_count = with_provider_http_client_cache(|cache| cache.entries.len());
    with_provider_http_client_runtime_metrics(|metrics| ProviderHttpClientRuntimeMetricsSnapshot {
        cache_entry_count,
        cache_hit_count: metrics.cache_hit_count,
        cache_miss_count: metrics.cache_miss_count,
        built_client_count: metrics.built_client_count,
    })
}

fn record_provider_http_client_cache_hit() {
    with_provider_http_client_runtime_metrics(|metrics| {
        metrics.cache_hit_count = metrics.cache_hit_count.saturating_add(1);
    });
}

fn record_provider_http_client_cache_miss() {
    with_provider_http_client_runtime_metrics(|metrics| {
        metrics.cache_miss_count = metrics.cache_miss_count.saturating_add(1);
    });
}

fn record_provider_http_client_build() {
    with_provider_http_client_runtime_metrics(|metrics| {
        metrics.built_client_count = metrics.built_client_count.saturating_add(1);
    });
}

#[cfg(test)]
fn clear_provider_http_client_cache() {
    with_provider_http_client_cache(|cache| {
        cache.entries.clear();
    });
    with_provider_http_client_runtime_metrics(|metrics| {
        *metrics = ProviderHttpClientRuntimeMetrics::default();
    });
}

#[cfg(test)]
fn provider_http_client_cache_contains_timeout(timeout_ms: u64) -> bool {
    let cache_key = ProviderHttpClientCacheKey { timeout_ms };

    with_provider_http_client_cache(|cache| cache.entries.contains_key(&cache_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_http_client_cache_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn test_request_policy(timeout_ms: u64) -> policy::ProviderRequestPolicy {
        policy::ProviderRequestPolicy {
            timeout_ms,
            max_attempts: 1,
            initial_backoff_ms: 50,
            max_backoff_ms: 50,
        }
    }

    #[test]
    fn provider_http_client_cache_reuses_clients_for_same_timeout_policy() {
        let _guard = provider_http_client_cache_test_lock()
            .lock()
            .expect("provider http client cache test lock");
        clear_provider_http_client_cache();
        let timeout_ms = 123_457;
        let request_policy = test_request_policy(timeout_ms);

        let _first_client = build_http_client(&request_policy).expect("first cached client");
        let _second_client = build_http_client(&request_policy).expect("second cached client");

        let contains_timeout = provider_http_client_cache_contains_timeout(timeout_ms);
        let metrics = provider_http_client_runtime_metrics_snapshot();

        assert!(contains_timeout);
        assert_eq!(metrics.cache_entry_count, 1);
        assert_eq!(metrics.cache_hit_count, 1);
        assert_eq!(metrics.cache_miss_count, 1);
        assert_eq!(metrics.built_client_count, 1);
    }

    #[test]
    fn provider_http_client_cache_separates_distinct_timeout_policies() {
        let _guard = provider_http_client_cache_test_lock()
            .lock()
            .expect("provider http client cache test lock");
        clear_provider_http_client_cache();
        let fast_timeout_ms = 123_458;
        let slow_timeout_ms = 123_459;
        let fast_policy = test_request_policy(fast_timeout_ms);
        let slow_policy = test_request_policy(slow_timeout_ms);

        let _fast_client = build_http_client(&fast_policy).expect("fast cached client");
        let _slow_client = build_http_client(&slow_policy).expect("slow cached client");

        let contains_fast_timeout = provider_http_client_cache_contains_timeout(fast_timeout_ms);
        let contains_slow_timeout = provider_http_client_cache_contains_timeout(slow_timeout_ms);
        let metrics = provider_http_client_runtime_metrics_snapshot();

        assert!(contains_fast_timeout);
        assert!(contains_slow_timeout);
        assert_eq!(metrics.cache_entry_count, 2);
        assert_eq!(metrics.cache_hit_count, 0);
        assert_eq!(metrics.cache_miss_count, 2);
        assert_eq!(metrics.built_client_count, 2);
    }
}
