use loong_kernel::access::memory::{
    MemoryBackend, MemoryExecutionContext, MemorySessionContext, MemoryWorkspaceContext,
};

use super::Context;

impl MemorySessionContext for Context<'_> {
    fn memory_session_id(&self) -> &str {
        self.session.session_id()
    }
}

impl MemoryWorkspaceContext for Context<'_> {
    fn memory_workspace_root(&self) -> Option<&std::path::Path> {
        self.session.workspace_root.as_deref()
    }
}

/// Expose the single backend selected when this Session was materialized.
///
/// Operation methods remain on `MemoryBackend`; Context must not mirror them
/// and become an equivalent execution API.
impl MemoryExecutionContext for Context<'_> {
    type StageEnvelope = crate::memory::StageEnvelope;
    type CompactOutput = crate::memory::StageDiagnostics;
    type Backend =
        dyn MemoryBackend<StageEnvelope = Self::StageEnvelope, CompactOutput = Self::CompactOutput>;

    fn memory_backend(&self) -> &Self::Backend {
        self.session.memory_backend.as_ref()
    }
}
