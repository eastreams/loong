//! The durable context store.

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::PathBuf;

use crate::item::ContextItem;
use crate::log;

/// Why opening a store failed.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// Another live store holds the same base path.
    ///
    /// The scope is per host for the local backend.
    #[error("context store path is already open: {0}")]
    InUse(PathBuf),
    /// The store could not be opened, locked, or replayed.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A point-in-time projection of the working context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSnapshot {
    pub items: Vec<ContextItem>,
    /// Monotonic within one store lifetime. Not durable across restarts.
    pub version: u64,
    /// Character-count estimate. A real token counter replaces this later.
    pub usage_tokens: usize,
}

/// The durable context store.
///
/// Heads live next to the base path as `<base>.<generation>.jsonl` in ascending
/// order. The highest generation is the current context. `replace` publishes
/// a new head and keeps the old ones; `append` mutates only the current head.
/// An exclusive advisory lock on `<base>.lock` scopes ownership to one live
/// store per host. The lock is released when the store drops.
///
/// The owning actor keeps this value as a field. Its serial task is the only
/// writer, so no mailbox or actor boundary is needed yet.
pub struct ContextStore {
    base: PathBuf,
    /// Held for the store's lifetime to keep the exclusive lock.
    _lock: File,
    file: File,
    generation: u64,
    pending: Pending,
    items: Vec<ContextItem>,
    version: u64,
}

#[derive(Debug)]
enum Pending {
    Idle,
    Append(Vec<ContextItem>),
    Replace,
}

impl fmt::Debug for ContextStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextStore")
            .field("base", &self.base)
            .field("generation", &self.generation)
            .field("pending", &self.pending)
            .field("items", &self.items.len())
            .field("version", &self.version)
            .finish()
    }
}

impl ContextStore {
    /// Opens the store, takes the exclusive lock, and replays the current
    /// head.
    ///
    /// Returns `Ok` only when no other live store holds the same base path.
    /// [`OpenError::InUse`] reports a conflict. The lock is advisory and per
    /// host; it is released when the store drops.
    pub fn open(base: PathBuf) -> Result<Self, OpenError> {
        fs::create_dir_all(&base)?;
        let lock = log::open_lock_file(&base)?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => OpenError::InUse(base.clone()),
            std::fs::TryLockError::Error(error) => OpenError::Io(error),
        })?;
        let generation = log::current_gen(&base)?.unwrap_or(0);
        let mut file = log::open_head(&base, generation)?;
        log::repair_tail(&mut file)?;
        let items = log::replay_items(&mut file)?;
        Ok(Self {
            base,
            _lock: lock,
            file,
            generation,
            pending: Pending::Idle,
            version: generation,
            items,
        })
    }

    /// Appends items. Returns the new version.
    pub fn append(&mut self, items: Vec<ContextItem>) -> u64 {
        self.items.extend(items.iter().cloned());
        match &mut self.pending {
            Pending::Idle => self.pending = Pending::Append(items),
            Pending::Append(pending) => pending.extend(items),
            // A pending replace already captures the full working items.
            Pending::Replace => {}
        }
        self.version += 1;
        self.version
    }

    /// Replaces the working context. Returns the new version.
    pub fn replace(&mut self, items: Vec<ContextItem>) -> u64 {
        self.items = items.clone();
        self.pending = Pending::Replace;
        self.version += 1;
        self.version
    }

    /// Projects the current working context.
    pub fn snapshot(&self) -> ContextSnapshot {
        ContextSnapshot {
            items: self.items.clone(),
            version: self.version,
            usage_tokens: self
                .items
                .iter()
                .map(|item| item.text.chars().count())
                .sum(),
        }
    }

    /// Writes buffered mutations to the current head.
    ///
    /// Each write reaches the OS, not disk: a power loss can still drop recent
    /// writes. On failure the unwritten suffix stays pending and is retried on
    /// the next flush.
    pub fn flush(&mut self) -> io::Result<()> {
        match std::mem::replace(&mut self.pending, Pending::Idle) {
            Pending::Idle => Ok(()),
            Pending::Append(items) => self.flush_append(items),
            Pending::Replace => self.flush_replace(),
        }
    }

    fn flush_append(&mut self, items: Vec<ContextItem>) -> io::Result<()> {
        let mut written = 0usize;
        let mut result = Ok(());
        for item in &items {
            if let Err(error) = log::write_item(&mut self.file, item) {
                result = Err(error);
                break;
            }
            written += 1;
        }
        if result.is_ok() {
            return Ok(());
        }
        // Written items stay in the OS buffer. The unwritten suffix stays
        // pending and is retried on the next flush.
        self.pending = Pending::Append(items[written..].to_vec());
        result
    }

    fn flush_replace(&mut self) -> io::Result<()> {
        let next = self.generation + 1;
        if let Err(error) = log::publish_head(&self.base, next, &self.items) {
            self.pending = Pending::Replace;
            return Err(error);
        }
        let file = match log::open_head(&self.base, next) {
            Ok(file) => file,
            Err(error) => {
                self.pending = Pending::Replace;
                return Err(error);
            }
        };
        self.file = file;
        self.generation = next;
        Ok(())
    }
}
