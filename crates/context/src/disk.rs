//! The disk context store backend.

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::PathBuf;

use crate::log;
use crate::store::{ContextSnapshot, ContextStore};
use loong_contracts::transcript::TranscriptItem;

/// Why opening a disk store failed.
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

/// The durable disk-backed context store.
///
/// Heads live inside the base directory as `<generation>.jsonl` in ascending
/// order. The highest generation is the current context. `replace` publishes
/// a new head and keeps the old ones; `append` mutates only the current head.
/// An exclusive advisory lock on the `lock` file inside the directory scopes
/// ownership to one live store per host. The lock is released when the store
/// drops.
pub struct DiskStore {
    base: PathBuf,
    /// Held for the store's lifetime to keep the exclusive lock.
    _lock: File,
    file: File,
    generation: u64,
    pending: Pending,
    items: Vec<TranscriptItem>,
    version: u64,
}

#[derive(Debug)]
enum Pending {
    Idle,
    Append(Vec<TranscriptItem>),
    /// The next head lives in `items`; flush publishes it in one shot.
    Replace,
}

impl fmt::Debug for DiskStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiskStore")
            .field("base", &self.base)
            .field("generation", &self.generation)
            .field("pending", &self.pending)
            .field("items", &self.items.len())
            .field("version", &self.version)
            .finish()
    }
}

impl DiskStore {
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

    fn flush_append(&mut self, items: Vec<TranscriptItem>) -> io::Result<()> {
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

impl ContextStore for DiskStore {
    fn append(&mut self, items: Vec<TranscriptItem>) -> u64 {
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

    fn replace(&mut self, items: Vec<TranscriptItem>) -> u64 {
        self.items = items.clone();
        self.pending = Pending::Replace;
        self.version += 1;
        self.version
    }

    fn snapshot(&self) -> ContextSnapshot {
        ContextSnapshot::build(self.items.clone(), self.version)
    }

    fn flush(&mut self) -> io::Result<()> {
        match std::mem::replace(&mut self.pending, Pending::Idle) {
            Pending::Idle => Ok(()),
            Pending::Append(items) => self.flush_append(items),
            Pending::Replace => self.flush_replace(),
        }
    }
}
