//! Typed memory access actions and their granted backend boundary.
//!
//! The app may choose a concrete memory system, but authorization and backend
//! entry stay domain-owned here. Each operation has a distinct Action; there is
//! no stringly generic memory action that can bypass operation-specific policy.

mod access;
mod append;
mod compact;
mod error;
mod execution;
mod replace;
mod snapshot;
mod stage;
mod transcript;
mod window;

pub use access::{MemoryAccess, MemorySessionContext, MemoryWorkspaceContext};
pub use append::{MemoryAppendTurnAction, MemoryAppendTurnAllowPolicy};
pub use compact::{MemoryCompactAction, MemoryCompactAllowPolicy};
pub use error::{MemoryAccessError, MemoryBackendError};
pub use execution::{MemoryBackend, MemoryExecutionContext};
pub use replace::{
    MemoryReplaceTurnsAction, MemoryReplaceTurnsAllowPolicy, MemoryReplaceTurnsOutcome,
};
pub use snapshot::{MemorySnapshot, MemoryTurn};
pub use stage::{MemoryReadStageEnvelopeAction, MemoryReadStageEnvelopeAllowPolicy};
pub use transcript::{MemoryTranscriptAction, MemoryTranscriptAllowPolicy};
pub use window::{MemoryWindowAction, MemoryWindowAllowPolicy};

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
