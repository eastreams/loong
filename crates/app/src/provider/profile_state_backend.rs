use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use crate::config::LoongClawConfig;
#[cfg(not(test))]
use crate::config::ProviderProfileStateBackendKind;

use super::{ProviderProfileStateSnapshot, ProviderProfileStateStore};

pub(super) trait ProviderProfileStateBackend: Send + Sync {
    fn load_store(&self) -> ProviderProfileStateStore;
    fn persist_snapshot(
        &self,
        snapshot: &ProviderProfileStateSnapshot,
    ) -> ProviderProfileStatePersistOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderProfileStatePersistOutcome {
    Persisted,
    StaleSkipped,
    Failed,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderProfileStatePersistenceMetricsSnapshot {
    pub(super) persisted: u64,
    pub(super) stale_skipped: u64,
    pub(super) failed: u64,
}

#[derive(Debug, Default)]
struct ProviderProfileStatePersistenceMetrics {
    persisted: u64,
    stale_skipped: u64,
    failed: u64,
}

impl ProviderProfileStatePersistenceMetrics {
    #[cfg(test)]
    fn snapshot(&self) -> ProviderProfileStatePersistenceMetricsSnapshot {
        ProviderProfileStatePersistenceMetricsSnapshot {
            persisted: self.persisted,
            stale_skipped: self.stale_skipped,
            failed: self.failed,
        }
    }
}

fn with_provider_profile_state_persistence_metrics<R>(
    run: impl FnOnce(&mut ProviderProfileStatePersistenceMetrics) -> R,
) -> R {
    let metrics = PROVIDER_PROFILE_STATE_PERSISTENCE_METRICS
        .get_or_init(|| Mutex::new(ProviderProfileStatePersistenceMetrics::default()));
    let mut guard = match metrics.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    run(&mut guard)
}

pub(super) fn record_provider_profile_state_persist_outcome(
    outcome: ProviderProfileStatePersistOutcome,
) {
    with_provider_profile_state_persistence_metrics(|metrics| match outcome {
        ProviderProfileStatePersistOutcome::Persisted => {
            metrics.persisted = metrics.persisted.saturating_add(1);
        }
        ProviderProfileStatePersistOutcome::StaleSkipped => {
            metrics.stale_skipped = metrics.stale_skipped.saturating_add(1);
        }
        ProviderProfileStatePersistOutcome::Failed => {
            metrics.failed = metrics.failed.saturating_add(1);
        }
    });
}

#[cfg(test)]
pub(super) fn provider_profile_state_persistence_metrics_snapshot()
-> ProviderProfileStatePersistenceMetricsSnapshot {
    with_provider_profile_state_persistence_metrics(|metrics| metrics.snapshot())
}

#[cfg(test)]
#[derive(Debug, Default)]
struct InMemoryProviderProfileStateBackend;

#[cfg(test)]
impl ProviderProfileStateBackend for InMemoryProviderProfileStateBackend {
    fn load_store(&self) -> ProviderProfileStateStore {
        ProviderProfileStateStore::default()
    }

    fn persist_snapshot(
        &self,
        _snapshot: &ProviderProfileStateSnapshot,
    ) -> ProviderProfileStatePersistOutcome {
        ProviderProfileStatePersistOutcome::StaleSkipped
    }
}

#[derive(Debug)]
pub(super) struct FileProviderProfileStateBackend {
    state_path: PathBuf,
    persist_revision: AtomicU64,
    persist_lock: Mutex<()>,
}

impl FileProviderProfileStateBackend {
    pub(super) fn with_path(state_path: PathBuf) -> Self {
        Self {
            state_path,
            persist_revision: AtomicU64::new(0),
            persist_lock: Mutex::new(()),
        }
    }

    fn state_path(&self) -> &Path {
        self.state_path.as_path()
    }
}

impl Default for FileProviderProfileStateBackend {
    fn default() -> Self {
        Self::with_path(crate::config::default_loongclaw_home().join("provider-profile-state.json"))
    }
}

impl ProviderProfileStateBackend for FileProviderProfileStateBackend {
    fn load_store(&self) -> ProviderProfileStateStore {
        let raw = match fs::read_to_string(self.state_path()) {
            Ok(raw) => raw,
            Err(_) => {
                self.persist_revision.store(0, Ordering::Release);
                return ProviderProfileStateStore::default();
            }
        };
        let snapshot = match serde_json::from_str::<ProviderProfileStateSnapshot>(&raw) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                self.persist_revision.store(0, Ordering::Release);
                return ProviderProfileStateStore::default();
            }
        };
        let store = ProviderProfileStateStore::from_snapshot(snapshot, Instant::now());
        self.persist_revision
            .store(store.revision, Ordering::Release);
        store
    }

    fn persist_snapshot(
        &self,
        snapshot: &ProviderProfileStateSnapshot,
    ) -> ProviderProfileStatePersistOutcome {
        let _guard = match self.persist_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let latest_revision = self.persist_revision.load(Ordering::Acquire);
        if snapshot.revision < latest_revision {
            return ProviderProfileStatePersistOutcome::StaleSkipped;
        }

        let path = self.state_path();
        if let Some(parent) = path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return ProviderProfileStatePersistOutcome::Failed;
            }
        }

        let payload = match serde_json::to_vec_pretty(snapshot) {
            Ok(payload) => payload,
            Err(_) => return ProviderProfileStatePersistOutcome::Failed,
        };

        let tmp_path = path.with_extension("json.tmp");
        let mut persisted = false;
        if fs::write(&tmp_path, &payload).is_ok() {
            if fs::rename(&tmp_path, &path).is_ok() {
                persisted = true;
            } else if fs::write(&path, &payload).is_ok() {
                persisted = true;
            }
            let _ = fs::remove_file(tmp_path);
        } else if fs::write(path, &payload).is_ok() {
            persisted = true;
        }
        if persisted {
            self.persist_revision
                .store(snapshot.revision, Ordering::Release);
            return ProviderProfileStatePersistOutcome::Persisted;
        }
        ProviderProfileStatePersistOutcome::Failed
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug)]
pub(super) struct SqliteProviderProfileStateBackend {
    state_path: PathBuf,
    legacy_json_fallback_path: Option<PathBuf>,
    persist_revision: AtomicU64,
    persist_lock: Mutex<()>,
}

#[cfg(feature = "memory-sqlite")]
impl SqliteProviderProfileStateBackend {
    #[cfg(test)]
    pub(super) fn new(state_path: PathBuf) -> Self {
        Self::with_legacy_fallback(state_path, None)
    }

    pub(super) fn with_legacy_fallback(
        state_path: PathBuf,
        legacy_json_fallback_path: Option<PathBuf>,
    ) -> Self {
        Self {
            state_path,
            legacy_json_fallback_path,
            persist_revision: AtomicU64::new(0),
            persist_lock: Mutex::new(()),
        }
    }

    fn state_path(&self) -> &Path {
        self.state_path.as_path()
    }

    fn ensure_parent_dir(&self) -> bool {
        if self.state_path() == Path::new(":memory:") {
            return true;
        }
        let Some(parent) = self.state_path().parent() else {
            return true;
        };
        if parent.as_os_str().is_empty() {
            return true;
        }
        fs::create_dir_all(parent).is_ok()
    }

    fn ensure_schema(conn: &rusqlite::Connection) -> bool {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS provider_profile_state(
              singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
              version INTEGER NOT NULL,
              revision INTEGER NOT NULL,
              generated_at_unix_ms INTEGER NOT NULL,
              payload TEXT NOT NULL,
              updated_at_unix_ms INTEGER NOT NULL
            );
            ",
        )
        .is_ok()
    }

    fn open_connection(&self) -> Option<rusqlite::Connection> {
        if !self.ensure_parent_dir() {
            return None;
        }
        rusqlite::Connection::open(self.state_path()).ok()
    }

    fn load_legacy_json_store(&self) -> Option<ProviderProfileStateStore> {
        let legacy_path = self.legacy_json_fallback_path.as_ref()?;
        let raw = fs::read_to_string(legacy_path).ok()?;
        let snapshot = serde_json::from_str::<ProviderProfileStateSnapshot>(&raw).ok()?;
        Some(ProviderProfileStateStore::from_snapshot(
            snapshot,
            Instant::now(),
        ))
    }

    fn persist_snapshot_to_sqlite(
        &self,
        conn: &rusqlite::Connection,
        snapshot: &ProviderProfileStateSnapshot,
    ) -> ProviderProfileStatePersistOutcome {
        let payload = match serde_json::to_string(snapshot) {
            Ok(payload) => payload,
            Err(_) => return ProviderProfileStatePersistOutcome::Failed,
        };

        let persisted_rows = conn.execute(
            "
            INSERT INTO provider_profile_state(
              singleton,
              version,
              revision,
              generated_at_unix_ms,
              payload,
              updated_at_unix_ms
            )
            VALUES(1, ?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(singleton) DO UPDATE SET
              version = excluded.version,
              revision = excluded.revision,
              generated_at_unix_ms = excluded.generated_at_unix_ms,
              payload = excluded.payload,
              updated_at_unix_ms = excluded.updated_at_unix_ms
            WHERE excluded.revision >= provider_profile_state.revision
            ",
            rusqlite::params![
                i64::from(snapshot.version),
                snapshot.revision as i64,
                snapshot.generated_at_unix_ms as i64,
                payload,
                super::current_unix_timestamp_ms() as i64
            ],
        );
        match persisted_rows {
            Ok(0) => ProviderProfileStatePersistOutcome::StaleSkipped,
            Ok(_) => ProviderProfileStatePersistOutcome::Persisted,
            Err(_) => ProviderProfileStatePersistOutcome::Failed,
        }
    }
}

#[cfg(feature = "memory-sqlite")]
impl ProviderProfileStateBackend for SqliteProviderProfileStateBackend {
    fn load_store(&self) -> ProviderProfileStateStore {
        let Some(conn) = self.open_connection() else {
            self.persist_revision.store(0, Ordering::Release);
            return ProviderProfileStateStore::default();
        };
        if !Self::ensure_schema(&conn) {
            self.persist_revision.store(0, Ordering::Release);
            return ProviderProfileStateStore::default();
        }
        let loaded = conn.query_row(
            "SELECT payload, revision
             FROM provider_profile_state
             WHERE singleton = 1",
            [],
            |row| -> rusqlite::Result<(String, i64)> { Ok((row.get(0)?, row.get(1)?)) },
        );
        let (payload, persisted_revision) = match loaded {
            Ok(tuple) => tuple,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                if let Some(legacy_store) = self.load_legacy_json_store() {
                    let snapshot = legacy_store.to_snapshot(Instant::now());
                    if matches!(
                        self.persist_snapshot_to_sqlite(&conn, &snapshot),
                        ProviderProfileStatePersistOutcome::Persisted
                    ) {
                        self.persist_revision
                            .store(snapshot.revision, Ordering::Release);
                    }
                    return legacy_store;
                }
                self.persist_revision.store(0, Ordering::Release);
                return ProviderProfileStateStore::default();
            }
            Err(_) => {
                self.persist_revision.store(0, Ordering::Release);
                return ProviderProfileStateStore::default();
            }
        };
        let snapshot = match serde_json::from_str::<ProviderProfileStateSnapshot>(&payload) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                self.persist_revision.store(0, Ordering::Release);
                return ProviderProfileStateStore::default();
            }
        };
        let mut store = ProviderProfileStateStore::from_snapshot(snapshot, Instant::now());
        let persisted_revision = u64::try_from(persisted_revision).unwrap_or(0);
        if persisted_revision > store.revision {
            store.revision = persisted_revision;
        }
        self.persist_revision
            .store(store.revision, Ordering::Release);
        store
    }

    fn persist_snapshot(
        &self,
        snapshot: &ProviderProfileStateSnapshot,
    ) -> ProviderProfileStatePersistOutcome {
        let _guard = match self.persist_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let latest_revision = self.persist_revision.load(Ordering::Acquire);
        if snapshot.revision < latest_revision {
            return ProviderProfileStatePersistOutcome::StaleSkipped;
        }

        let Some(conn) = self.open_connection() else {
            return ProviderProfileStatePersistOutcome::Failed;
        };
        if !Self::ensure_schema(&conn) {
            return ProviderProfileStatePersistOutcome::Failed;
        }

        match self.persist_snapshot_to_sqlite(&conn, snapshot) {
            ProviderProfileStatePersistOutcome::Persisted => {
                self.persist_revision
                    .store(snapshot.revision, Ordering::Release);
                ProviderProfileStatePersistOutcome::Persisted
            }
            ProviderProfileStatePersistOutcome::StaleSkipped => {
                ProviderProfileStatePersistOutcome::StaleSkipped
            }
            ProviderProfileStatePersistOutcome::Failed => {
                ProviderProfileStatePersistOutcome::Failed
            }
        }
    }
}

#[cfg(test)]
fn default_provider_profile_state_backend() -> Arc<dyn ProviderProfileStateBackend> {
    Arc::new(InMemoryProviderProfileStateBackend)
}

#[cfg(not(test))]
fn default_provider_profile_state_backend() -> Arc<dyn ProviderProfileStateBackend> {
    Arc::new(FileProviderProfileStateBackend::default())
}

#[cfg(not(test))]
fn configured_provider_profile_state_backend(
    config: &LoongClawConfig,
) -> Arc<dyn ProviderProfileStateBackend> {
    match config.provider.resolved_profile_state_backend() {
        ProviderProfileStateBackendKind::File => {
            Arc::new(FileProviderProfileStateBackend::default())
        }
        ProviderProfileStateBackendKind::Sqlite => {
            #[cfg(feature = "memory-sqlite")]
            {
                let state_path = config
                    .provider
                    .resolved_profile_state_sqlite_path_with_default();
                let legacy_json =
                    crate::config::default_loongclaw_home().join("provider-profile-state.json");
                Arc::new(SqliteProviderProfileStateBackend::with_legacy_fallback(
                    state_path,
                    Some(legacy_json),
                ))
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Arc::new(FileProviderProfileStateBackend::default())
            }
        }
    }
}

#[cfg(not(test))]
pub(super) fn ensure_provider_profile_state_backend(config: &LoongClawConfig) {
    if PROVIDER_PROFILE_STATE_BACKEND.get().is_some() {
        return;
    }
    let backend = configured_provider_profile_state_backend(config);
    let _ = PROVIDER_PROFILE_STATE_BACKEND.set(backend);
}

#[cfg(test)]
pub(super) fn ensure_provider_profile_state_backend(_config: &LoongClawConfig) {}

pub(super) fn with_provider_profile_states<R>(
    run: impl FnOnce(&mut ProviderProfileStateStore) -> R,
) -> R {
    let store = PROVIDER_PROFILE_STATES
        .get_or_init(|| Mutex::new(provider_profile_state_backend().load_store()));
    let mut guard = match store.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    run(&mut guard)
}

pub(super) fn persist_provider_profile_state_snapshot(snapshot: &ProviderProfileStateSnapshot) {
    let outcome = provider_profile_state_backend().persist_snapshot(snapshot);
    record_provider_profile_state_persist_outcome(outcome);
}

#[cfg(test)]
pub(super) fn provider_profile_state_backend() -> &'static Arc<dyn ProviderProfileStateBackend> {
    PROVIDER_PROFILE_STATE_BACKEND.get_or_init(default_provider_profile_state_backend)
}

#[cfg(not(test))]
fn provider_profile_state_backend() -> &'static Arc<dyn ProviderProfileStateBackend> {
    PROVIDER_PROFILE_STATE_BACKEND.get_or_init(default_provider_profile_state_backend)
}

static PROVIDER_PROFILE_STATES: OnceLock<Mutex<ProviderProfileStateStore>> = OnceLock::new();
static PROVIDER_PROFILE_STATE_PERSISTENCE_METRICS: OnceLock<
    Mutex<ProviderProfileStatePersistenceMetrics>,
> = OnceLock::new();
static PROVIDER_PROFILE_STATE_BACKEND: OnceLock<Arc<dyn ProviderProfileStateBackend>> =
    OnceLock::new();
