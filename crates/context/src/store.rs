//! The context store abstraction.

use std::io;

use crate::item::ContextItem;

/// A point-in-time projection of the working context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSnapshot {
    pub items: Vec<ContextItem>,
    /// Monotonic within one store instance. Not durable across restarts.
    pub version: u64,
    /// Character-count estimate. A real token counter replaces this later.
    pub usage_tokens: usize,
}

impl ContextSnapshot {
    /// Builds a snapshot, computing the usage estimate from the items.
    pub(crate) fn build(items: Vec<ContextItem>, version: u64) -> Self {
        let usage_tokens = items.iter().map(|item| item.text.chars().count()).sum();
        Self {
            items,
            version,
            usage_tokens,
        }
    }
}

/// Storage for one session's working context.
///
/// `version` is monotonic within one store instance. Its initial value and
/// durability are backend-defined. `flush` commits buffered mutations; a
/// backend without durable storage returns `Ok`.
pub trait ContextStore {
    /// Appends items. Returns the new version.
    fn append(&mut self, items: Vec<ContextItem>) -> u64;

    /// Replaces the working context. Returns the new version.
    fn replace(&mut self, items: Vec<ContextItem>) -> u64;

    /// Projects the current working context.
    fn snapshot(&self) -> ContextSnapshot;

    /// Commits buffered mutations.
    fn flush(&mut self) -> io::Result<()>;
}
