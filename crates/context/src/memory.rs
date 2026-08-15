//! The in-memory context store backend.

use std::io;

use crate::store::{ContextSnapshot, ContextStore};
use contracts::transcript::TranscriptItem;

/// A context store that keeps items in memory only.
///
/// `flush` is a no-op. Use it for ephemeral sessions and tests.
#[derive(Debug, Default)]
pub struct MemoryStore {
    items: Vec<TranscriptItem>,
    version: u64,
}

impl MemoryStore {
    /// Creates an empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ContextStore for MemoryStore {
    fn append(&mut self, items: Vec<TranscriptItem>) -> u64 {
        self.items.extend(items);
        self.version += 1;
        self.version
    }

    fn replace(&mut self, items: Vec<TranscriptItem>) -> u64 {
        self.items = items;
        self.version += 1;
        self.version
    }

    fn snapshot(&self) -> ContextSnapshot {
        ContextSnapshot::build(self.items.clone(), self.version)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
