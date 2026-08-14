//! The context store abstraction.

use std::io;

use contracts::transcript::{TranscriptItem, TranscriptItemKind};

/// A point-in-time projection of the working context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSnapshot {
    pub items: Vec<TranscriptItem>,
    /// Monotonic within one store instance. Not durable across restarts.
    pub version: u64,
    /// Character-count estimate. A real token counter replaces this later.
    pub usage_tokens: usize,
}

impl ContextSnapshot {
    /// Builds a snapshot, computing the usage estimate from the items.
    pub(crate) fn build(items: Vec<TranscriptItem>, version: u64) -> Self {
        let usage_tokens = items.iter().map(usage_of).sum();
        Self {
            items,
            version,
            usage_tokens,
        }
    }
}

/// Counts the characters that a tokenizer would see in one item.
fn usage_of(item: &TranscriptItem) -> usize {
    match &item.kind {
        TranscriptItemKind::Message { text, .. } => text.chars().count(),
        TranscriptItemKind::ToolCall { name, arguments } => {
            name.chars().count() + arguments.chars().count()
        }
        TranscriptItemKind::ToolResult { output, .. } => output.chars().count(),
    }
}

/// Storage for one session's working context.
///
/// `version` is monotonic within one store instance. Its initial value and
/// durability are backend-defined. `flush` commits buffered mutations; a
/// backend without durable storage returns `Ok`.
pub trait ContextStore {
    /// Appends items. Returns the new version.
    fn append(&mut self, items: Vec<TranscriptItem>) -> u64;

    /// Replaces the working context. Returns the new version.
    fn replace(&mut self, items: Vec<TranscriptItem>) -> u64;

    /// Projects the current working context.
    fn snapshot(&self) -> ContextSnapshot;

    /// Commits buffered mutations.
    fn flush(&mut self) -> io::Result<()>;
}
