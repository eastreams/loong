use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::UNIX_EPOCH,
};

use wasmtime::{Engine as WasmtimeEngine, Module as WasmtimeModule};

use super::wasm_runtime_policy::{
    wasm_module_cache_capacity_from_env, wasm_module_cache_max_bytes_from_env,
};

static WASM_MODULE_CACHE: OnceLock<Mutex<WasmModuleCache>> = OnceLock::new();
static WASM_MODULE_CACHE_CAPACITY: OnceLock<usize> = OnceLock::new();
static WASM_MODULE_CACHE_MAX_BYTES: OnceLock<usize> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct WasmModuleCacheKey {
    artifact_path: PathBuf,
    module_size_bytes: u64,
    artifact_modified_unix_ns: Option<u128>,
    artifact_file_identity: Option<WasmArtifactFileIdentity>,
    expected_sha256: Option<String>,
    fuel_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct WasmArtifactFileIdentity {
    device_id: u64,
    inode: u64,
    ctime_seconds: i64,
    ctime_nanoseconds: i64,
}

#[derive(Debug, Clone)]
pub(super) struct CachedWasmModule {
    pub(super) engine: WasmtimeEngine,
    pub(super) module: WasmtimeModule,
    pub(super) artifact_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WasmModuleCacheLookup {
    pub(super) hit: bool,
    pub(super) inserted: bool,
    pub(super) evicted_entries: usize,
    pub(super) cache_len: usize,
    pub(super) cache_capacity: usize,
    pub(super) cache_total_module_bytes: usize,
    pub(super) cache_max_bytes: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WasmModuleCacheInsertOutcome {
    pub(super) inserted: bool,
    pub(super) evicted_entries: usize,
}

#[derive(Debug, Clone)]
pub(super) struct WasmModuleCacheEntry {
    module: Arc<CachedWasmModule>,
    module_size_bytes: usize,
}

#[derive(Debug, Default)]
pub(super) struct WasmModuleCache {
    order: VecDeque<WasmModuleCacheKey>,
    entries: HashMap<WasmModuleCacheKey, WasmModuleCacheEntry>,
    total_module_bytes: usize,
}

impl WasmModuleCache {
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn total_module_bytes(&self) -> usize {
        self.total_module_bytes
    }

    fn touch(&mut self, key: &WasmModuleCacheKey) {
        if let Some(position) = self.order.iter().position(|candidate| candidate == key) {
            let _ = self.order.remove(position);
        }
        self.order.push_back(key.clone());
    }

    fn remove_by_key(&mut self, key: &WasmModuleCacheKey) -> bool {
        self.order.retain(|candidate| candidate != key);
        let Some(removed) = self.entries.remove(key) else {
            return false;
        };
        self.total_module_bytes = self
            .total_module_bytes
            .saturating_sub(removed.module_size_bytes);
        true
    }

    fn evict_oldest(&mut self) -> bool {
        while let Some(oldest) = self.order.pop_front() {
            if let Some(removed) = self.entries.remove(&oldest) {
                self.total_module_bytes = self
                    .total_module_bytes
                    .saturating_sub(removed.module_size_bytes);
                return true;
            }
        }
        false
    }

    fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
        self.total_module_bytes = 0;
    }

    pub(super) fn get(&mut self, key: &WasmModuleCacheKey) -> Option<Arc<CachedWasmModule>> {
        let entry = self.entries.get(key)?.module.clone();
        self.touch(key);
        Some(entry)
    }

    pub(super) fn insert(
        &mut self,
        key: WasmModuleCacheKey,
        value: Arc<CachedWasmModule>,
        module_size_bytes: usize,
        max_entries: usize,
        max_total_bytes: usize,
    ) -> WasmModuleCacheInsertOutcome {
        if max_entries == 0 || max_total_bytes == 0 {
            self.clear();
            return WasmModuleCacheInsertOutcome {
                inserted: false,
                evicted_entries: 0,
            };
        }

        if module_size_bytes > max_total_bytes {
            return WasmModuleCacheInsertOutcome {
                inserted: false,
                evicted_entries: 0,
            };
        }

        let _ = self.remove_by_key(&key);
        let mut evicted_entries = 0usize;
        while self.entries.len() >= max_entries
            || self.total_module_bytes.saturating_add(module_size_bytes) > max_total_bytes
        {
            if !self.evict_oldest() {
                break;
            }
            evicted_entries = evicted_entries.saturating_add(1);
        }

        if self.entries.len() >= max_entries
            || self.total_module_bytes.saturating_add(module_size_bytes) > max_total_bytes
        {
            return WasmModuleCacheInsertOutcome {
                inserted: false,
                evicted_entries,
            };
        }
        self.order.push_back(key.clone());
        self.total_module_bytes = self.total_module_bytes.saturating_add(module_size_bytes);
        self.entries.insert(
            key,
            WasmModuleCacheEntry {
                module: value,
                module_size_bytes,
            },
        );
        WasmModuleCacheInsertOutcome {
            inserted: true,
            evicted_entries,
        }
    }
}

fn wasm_module_cache() -> &'static Mutex<WasmModuleCache> {
    WASM_MODULE_CACHE.get_or_init(|| Mutex::new(WasmModuleCache::default()))
}

pub(super) fn wasm_module_cache_capacity() -> usize {
    *WASM_MODULE_CACHE_CAPACITY.get_or_init(wasm_module_cache_capacity_from_env)
}

pub(super) fn wasm_module_cache_max_bytes() -> usize {
    *WASM_MODULE_CACHE_MAX_BYTES.get_or_init(wasm_module_cache_max_bytes_from_env)
}

pub(super) fn modified_unix_nanos(metadata: &fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
}

#[cfg(unix)]
pub(super) fn wasm_artifact_file_identity(
    metadata: &fs::Metadata,
) -> Option<WasmArtifactFileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Some(WasmArtifactFileIdentity {
        device_id: metadata.dev(),
        inode: metadata.ino(),
        ctime_seconds: metadata.ctime(),
        ctime_nanoseconds: metadata.ctime_nsec(),
    })
}

#[cfg(not(unix))]
pub(super) fn wasm_artifact_file_identity(
    _metadata: &fs::Metadata,
) -> Option<WasmArtifactFileIdentity> {
    None
}

pub(super) fn build_wasm_module_cache_key(
    artifact_path: &Path,
    module_size_bytes: u64,
    artifact_modified_unix_ns: Option<u128>,
    artifact_file_identity: Option<WasmArtifactFileIdentity>,
    expected_sha256: Option<String>,
    fuel_enabled: bool,
) -> WasmModuleCacheKey {
    WasmModuleCacheKey {
        artifact_path: artifact_path.to_path_buf(),
        module_size_bytes,
        artifact_modified_unix_ns,
        artifact_file_identity,
        expected_sha256,
        fuel_enabled,
    }
}

pub(super) fn lookup_cached_wasm_module(
    cache_key: &WasmModuleCacheKey,
) -> Result<Option<(Arc<CachedWasmModule>, WasmModuleCacheLookup)>, String> {
    let cache_capacity = wasm_module_cache_capacity();
    let cache_max_bytes = wasm_module_cache_max_bytes();
    let cache_lock = wasm_module_cache();
    let mut cache = cache_lock
        .lock()
        .map_err(|error| format!("failed to lock wasm module cache: {error}"))?;
    let Some(cached) = cache.get(cache_key) else {
        return Ok(None);
    };
    Ok(Some((
        cached,
        WasmModuleCacheLookup {
            hit: true,
            inserted: false,
            evicted_entries: 0,
            cache_len: cache.len(),
            cache_capacity,
            cache_total_module_bytes: cache.total_module_bytes(),
            cache_max_bytes,
        },
    )))
}

pub(super) fn insert_cached_wasm_module(
    cache_key: WasmModuleCacheKey,
    module: Arc<CachedWasmModule>,
    module_size_bytes: usize,
) -> Result<WasmModuleCacheLookup, String> {
    let cache_capacity = wasm_module_cache_capacity();
    let cache_max_bytes = wasm_module_cache_max_bytes();
    let cache_lock = wasm_module_cache();
    let mut cache = cache_lock
        .lock()
        .map_err(|error| format!("failed to lock wasm module cache: {error}"))?;
    let insert_outcome = cache.insert(
        cache_key,
        module,
        module_size_bytes,
        cache_capacity,
        cache_max_bytes,
    );
    Ok(WasmModuleCacheLookup {
        hit: false,
        inserted: insert_outcome.inserted,
        evicted_entries: insert_outcome.evicted_entries,
        cache_len: cache.len(),
        cache_capacity,
        cache_total_module_bytes: cache.total_module_bytes(),
        cache_max_bytes,
    })
}
