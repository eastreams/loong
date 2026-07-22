use async_trait::async_trait;
use loong_core::policy::grant::Granted;

use super::{
    MemoryAppendTurnAction, MemoryBackendError, MemoryCompactAction, MemoryReadStageEnvelopeAction,
    MemoryReplaceTurnsAction, MemoryReplaceTurnsOutcome, MemorySnapshot, MemoryTranscriptAction,
    MemoryWindowAction,
};

/// Side-effect port implemented by the app-selected memory backend.
///
/// Every entry consumes the exact `Granted<A>` it executes. Keeping this port
/// in `loong-access` prevents an app backend from exposing an equivalent
/// ungoverned entry point beside the Action path.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    type StageEnvelope: Send;
    type CompactOutput: Send;

    async fn append_turn(
        &self,
        granted: Granted<MemoryAppendTurnAction>,
    ) -> Result<(), MemoryBackendError>;

    async fn window(
        &self,
        granted: Granted<MemoryWindowAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError>;

    async fn transcript(
        &self,
        granted: Granted<MemoryTranscriptAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError>;

    async fn replace_turns(
        &self,
        granted: Granted<MemoryReplaceTurnsAction>,
    ) -> Result<MemoryReplaceTurnsOutcome, MemoryBackendError>;

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<Self::StageEnvelope, MemoryBackendError>;

    async fn compact(
        &self,
        granted: Granted<MemoryCompactAction>,
    ) -> Result<Self::CompactOutput, MemoryBackendError>;
}

/// Narrow Context requirement for executing granted memory Actions.
///
/// Context exposes one Session-owned backend reference. It does not mirror the
/// backend operation surface or become a second execution port.
pub trait MemoryExecutionContext: Sync {
    type StageEnvelope: Send;
    type CompactOutput: Send;
    type Backend: MemoryBackend<StageEnvelope = Self::StageEnvelope, CompactOutput = Self::CompactOutput>
        + ?Sized;

    fn memory_backend(&self) -> &Self::Backend;
}
