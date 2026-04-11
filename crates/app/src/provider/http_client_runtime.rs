use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::CliResult;

use super::policy;

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

fn with_provider_http_client_cache<R>(run: impl FnOnce(&mut ProviderHttpClientCache) -> R) -> R {
    let cache =
        PROVIDER_HTTP_CLIENT_CACHE.get_or_init(|| Mutex::new(ProviderHttpClientCache::default()));
    let mut guard = match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    run(&mut guard)
}

fn load_cached_provider_http_client(
    cache_key: ProviderHttpClientCacheKey,
) -> Option<reqwest::Client> {
    with_provider_http_client_cache(|cache| {
        let cached_client = cache.entries.get(&cache_key)?;
        let cloned_client = cached_client.clone();

        Some(cloned_client)
    })
}

fn store_provider_http_client(
    cache_key: ProviderHttpClientCacheKey,
    built_client: reqwest::Client,
) -> reqwest::Client {
    with_provider_http_client_cache(|cache| {
        if let Some(cached_client) = cache.entries.get(&cache_key) {
            let cloned_client = cached_client.clone();

            return cloned_client;
        }

        let cached_client = built_client.clone();
        cache.entries.insert(cache_key, cached_client);

        built_client
    })
}

fn build_provider_http_client(cache_key: ProviderHttpClientCacheKey) -> CliResult<reqwest::Client> {
    let timeout = Duration::from_millis(cache_key.timeout_ms);
    let client_builder = reqwest::Client::builder();
    let timeout_builder = client_builder.timeout(timeout);
    let built_client = timeout_builder
        .build()
        .map_err(|error| format!("build provider http client failed: {error}"))?;

    #[cfg(test)]
    {
        record_provider_http_client_build(cache_key);
    }

    Ok(built_client)
}

pub(super) fn build_http_client(
    request_policy: &policy::ProviderRequestPolicy,
) -> CliResult<reqwest::Client> {
    let cache_key = ProviderHttpClientCacheKey::from_request_policy(request_policy);

    if let Some(cached_client) = load_cached_provider_http_client(cache_key) {
        return Ok(cached_client);
    }

    let built_client = build_provider_http_client(cache_key)?;
    let cached_client = store_provider_http_client(cache_key, built_client);

    Ok(cached_client)
}

static PROVIDER_HTTP_CLIENT_CACHE: OnceLock<Mutex<ProviderHttpClientCache>> = OnceLock::new();

#[cfg(test)]
static PROVIDER_HTTP_CLIENT_BUILD_COUNTS: OnceLock<Mutex<HashMap<u64, usize>>> = OnceLock::new();

#[cfg(test)]
fn clear_provider_http_client_cache() {
    with_provider_http_client_cache(|cache| {
        cache.entries.clear();
    });
    with_provider_http_client_build_counts(|build_counts| {
        build_counts.clear();
    });
}

#[cfg(test)]
fn provider_http_client_cache_contains_timeout(timeout_ms: u64) -> bool {
    let cache_key = ProviderHttpClientCacheKey { timeout_ms };

    with_provider_http_client_cache(|cache| cache.entries.contains_key(&cache_key))
}

#[cfg(test)]
fn with_provider_http_client_build_counts<R>(run: impl FnOnce(&mut HashMap<u64, usize>) -> R) -> R {
    let build_counts = PROVIDER_HTTP_CLIENT_BUILD_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = match build_counts.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    run(&mut guard)
}

#[cfg(test)]
fn record_provider_http_client_build(cache_key: ProviderHttpClientCacheKey) {
    with_provider_http_client_build_counts(|build_counts| {
        let build_count = build_counts.entry(cache_key.timeout_ms).or_default();
        *build_count += 1;
    });
}

#[cfg(test)]
fn provider_http_client_build_count_for_timeout(timeout_ms: u64) -> usize {
    with_provider_http_client_build_counts(|build_counts| {
        build_counts.get(&timeout_ms).copied().unwrap_or_default()
    })
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
        let build_count = provider_http_client_build_count_for_timeout(timeout_ms);

        assert!(contains_timeout);
        assert_eq!(build_count, 1);
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
        let fast_build_count = provider_http_client_build_count_for_timeout(fast_timeout_ms);
        let slow_build_count = provider_http_client_build_count_for_timeout(slow_timeout_ms);

        assert!(contains_fast_timeout);
        assert!(contains_slow_timeout);
        assert_eq!(fast_build_count, 1);
        assert_eq!(slow_build_count, 1);
    }
}
