use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use tokio::sync::watch;

use crate::CliResult;

const MODEL_CATALOG_SINGLEFLIGHT_FOLLOWER_WAIT: Duration = Duration::from_secs(5);
const MODEL_CATALOG_SINGLEFLIGHT_STALE_SLOT_AGE: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
pub(super) struct ModelCatalogCacheEntry {
    models: Vec<String>,
    fresh_expires_at: Instant,
    stale_expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ModelCatalogCacheLookup {
    Fresh(Vec<String>),
    Stale(Vec<String>),
}

#[derive(Debug, Default)]
pub(super) struct ModelCatalogCache {
    pub(super) entries: HashMap<String, ModelCatalogCacheEntry>,
    order: VecDeque<String>,
}

impl ModelCatalogCache {
    pub(super) fn lookup(&mut self, key: &str, now: Instant) -> Option<ModelCatalogCacheLookup> {
        self.prune_dead(now);
        let entry = self.entries.get(key)?;
        if entry.fresh_expires_at > now {
            return Some(ModelCatalogCacheLookup::Fresh(entry.models.clone()));
        }
        Some(ModelCatalogCacheLookup::Stale(entry.models.clone()))
    }

    pub(super) fn put(
        &mut self,
        key: String,
        models: Vec<String>,
        now: Instant,
        ttl: Duration,
        stale_if_error: Duration,
        max_entries: usize,
    ) {
        if ttl.is_zero() || models.is_empty() {
            return;
        }
        let Some(fresh_expires_at) = now.checked_add(ttl) else {
            return;
        };
        let stale_expires_at = fresh_expires_at
            .checked_add(stale_if_error)
            .unwrap_or(fresh_expires_at);

        self.prune_dead(now);
        self.entries.insert(
            key.clone(),
            ModelCatalogCacheEntry {
                models,
                fresh_expires_at,
                stale_expires_at,
            },
        );
        self.order.retain(|existing| existing != &key);
        self.order.push_back(key);
        self.prune_capacity(max_entries);
    }

    fn prune_dead(&mut self, now: Instant) {
        self.entries.retain(|_, entry| entry.stale_expires_at > now);
        self.order.retain(|key| self.entries.contains_key(key));
    }

    fn prune_capacity(&mut self, max_entries: usize) {
        while self.entries.len() > max_entries {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&evicted);
        }
    }
}

#[derive(Debug)]
struct ModelCatalogFetchSlot {
    sender: watch::Sender<Option<CliResult<Vec<String>>>>,
    started_at: Instant,
}

struct ModelCatalogFetchLeaderGuard {
    key: String,
    slot: Arc<ModelCatalogFetchSlot>,
}

impl Drop for ModelCatalogFetchLeaderGuard {
    fn drop(&mut self) {
        with_model_catalog_fetches(|fetches| {
            if let Some(existing) = fetches.get(self.key.as_str())
                && Arc::ptr_eq(existing, &self.slot)
            {
                fetches.remove(self.key.as_str());
            }
        });
    }
}

fn with_model_catalog_cache<R>(run: impl FnOnce(&mut ModelCatalogCache) -> R) -> R {
    let cache = MODEL_CATALOG_CACHE.get_or_init(|| Mutex::new(ModelCatalogCache::default()));
    let mut guard = match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    run(&mut guard)
}

fn with_model_catalog_fetches<R>(
    run: impl FnOnce(&mut HashMap<String, Arc<ModelCatalogFetchSlot>>) -> R,
) -> R {
    let inflight = MODEL_CATALOG_FETCHES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = match inflight.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    run(&mut guard)
}

pub(super) fn load_cached_model_catalog(cache_key: &str) -> Option<ModelCatalogCacheLookup> {
    with_model_catalog_cache(|cache| cache.lookup(cache_key, Instant::now()))
}

pub(super) fn store_model_catalog(
    cache_key: String,
    models: Vec<String>,
    ttl: Duration,
    stale_if_error: Duration,
    max_entries: usize,
) {
    with_model_catalog_cache(|cache| {
        cache.put(
            cache_key,
            models,
            Instant::now(),
            ttl,
            stale_if_error,
            max_entries,
        )
    });
}

pub(super) async fn fetch_model_catalog_singleflight<F, Fut>(
    cache_key: &str,
    fetch_models: F,
) -> CliResult<Vec<String>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = CliResult<Vec<String>>>,
{
    fetch_model_catalog_singleflight_with_timeouts(
        cache_key,
        MODEL_CATALOG_SINGLEFLIGHT_FOLLOWER_WAIT,
        MODEL_CATALOG_SINGLEFLIGHT_STALE_SLOT_AGE,
        fetch_models,
    )
    .await
}

pub(super) async fn fetch_model_catalog_singleflight_with_timeouts<F, Fut>(
    cache_key: &str,
    follower_wait: Duration,
    stale_slot_age: Duration,
    fetch_models: F,
) -> CliResult<Vec<String>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = CliResult<Vec<String>>>,
{
    let mut fetch_models = Some(fetch_models);
    loop {
        let mut should_fetch = false;
        let slot = with_model_catalog_fetches(|fetches| {
            if let Some(existing) = fetches.get(cache_key) {
                return existing.clone();
            }
            let (sender, _receiver) = watch::channel(None);
            let slot = Arc::new(ModelCatalogFetchSlot {
                sender,
                started_at: Instant::now(),
            });
            fetches.insert(cache_key.to_owned(), slot.clone());
            should_fetch = true;
            slot
        });
        let leader_guard = should_fetch.then(|| ModelCatalogFetchLeaderGuard {
            key: cache_key.to_owned(),
            slot: slot.clone(),
        });

        if !should_fetch {
            let mut receiver = slot.sender.subscribe();
            let slot_started_at = slot.started_at;
            let slot_ptr = Arc::as_ptr(&slot) as usize;
            drop(slot);
            loop {
                if let Some(result) = receiver.borrow().clone() {
                    return result;
                }
                match tokio::time::timeout(follower_wait, receiver.changed()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => {
                        // Leader exited before publishing a result; retry and rejoin as follower/leader.
                        break;
                    }
                    Err(_) => {
                        let should_evict = slot_started_at.elapsed() >= stale_slot_age;
                        if !should_evict {
                            continue;
                        }
                        let removed = with_model_catalog_fetches(|fetches| {
                            if let Some(existing) = fetches.get(cache_key)
                                && (Arc::as_ptr(existing) as usize) == slot_ptr
                            {
                                fetches.remove(cache_key);
                                return true;
                            }
                            false
                        });
                        if removed {
                            break;
                        }
                    }
                }
            }
            continue;
        }

        let Some(fetch_models) = fetch_models.take() else {
            return Err(
                "provider model catalog singleflight leader fetch closure missing".to_owned(),
            );
        };
        let result = fetch_models().await;
        let _ = slot.sender.send(Some(result.clone()));
        drop(leader_guard);
        return result;
    }
}

#[cfg(test)]
pub(super) fn clear_model_catalog_singleflight_slot(cache_key: &str) {
    with_model_catalog_fetches(|fetches| {
        fetches.remove(cache_key);
    });
}

#[cfg(test)]
pub(super) fn model_catalog_singleflight_slot_count() -> usize {
    with_model_catalog_fetches(|fetches| fetches.len())
}

static MODEL_CATALOG_CACHE: OnceLock<Mutex<ModelCatalogCache>> = OnceLock::new();
static MODEL_CATALOG_FETCHES: OnceLock<Mutex<HashMap<String, Arc<ModelCatalogFetchSlot>>>> =
    OnceLock::new();
