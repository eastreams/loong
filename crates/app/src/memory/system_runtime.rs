use std::sync::Arc;

use async_trait::async_trait;
use loong_core::policy::grant::Granted;
use loong_kernel::access::memory::{
    MemoryAppendTurnAction, MemoryBackend, MemoryBackendError, MemoryCompactAction,
    MemoryReadStageEnvelopeAction, MemoryReplaceTurnsAction, MemoryReplaceTurnsOutcome,
    MemorySnapshot, MemoryTranscriptAction, MemoryWindowAction,
};

use super::orchestrator::{
    BuiltinMemoryOrchestrator, hydrate_stage_envelope_without_execution_adapter,
    skip_compact_stage_without_execution_adapter, skipped_stage_diagnostics,
};
use super::runtime_config::MemoryRuntimeConfig;
use super::{
    MemoryStageFamily, MemorySystem, MemorySystemMetadata, StageDiagnostics, StageEnvelope,
};

#[async_trait]
pub trait MemorySystemRuntime: Send + Sync {
    fn metadata(&self) -> &MemorySystemMetadata;

    fn config(&self) -> &MemoryRuntimeConfig;

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<StageEnvelope, MemoryBackendError>;

    async fn compact(
        &self,
        granted: Granted<MemoryCompactAction>,
    ) -> Result<StageDiagnostics, MemoryBackendError>;
}

/// Session-selected backend for all governed memory side effects.
///
/// Durable storage is shared across memory systems, while stage hydration and
/// compaction remain system-defined. This type owns that composition so Context
/// exposes one `MemoryBackend` instead of forwarding every operation itself.
pub struct MemorySystemBackend {
    runtime: Box<dyn MemorySystemRuntime>,
}

impl MemorySystemBackend {
    pub fn new(runtime: Box<dyn MemorySystemRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl MemoryBackend for MemorySystemBackend {
    type StageEnvelope = StageEnvelope;
    type CompactOutput = StageDiagnostics;

    async fn append_turn(
        &self,
        granted: Granted<MemoryAppendTurnAction>,
    ) -> Result<(), MemoryBackendError> {
        #[cfg(feature = "memory-sqlite")]
        {
            super::sqlite::append_turn_granted(granted, self.runtime.config())
        }
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = granted;
            Err(MemoryBackendError::Execution {
                operation: "append_turn",
                source: "sqlite memory is disabled in this build".into(),
            })
        }
    }

    async fn window(
        &self,
        granted: Granted<MemoryWindowAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError> {
        #[cfg(feature = "memory-sqlite")]
        {
            super::sqlite::window_granted(granted, self.runtime.config())
        }
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = granted;
            Err(MemoryBackendError::Execution {
                operation: "window",
                source: "sqlite memory is disabled in this build".into(),
            })
        }
    }

    async fn transcript(
        &self,
        granted: Granted<MemoryTranscriptAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError> {
        #[cfg(feature = "memory-sqlite")]
        {
            super::sqlite::transcript_granted(granted, self.runtime.config())
        }
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = granted;
            Err(MemoryBackendError::Execution {
                operation: "transcript",
                source: "sqlite memory is disabled in this build".into(),
            })
        }
    }

    async fn replace_turns(
        &self,
        granted: Granted<MemoryReplaceTurnsAction>,
    ) -> Result<MemoryReplaceTurnsOutcome, MemoryBackendError> {
        #[cfg(feature = "memory-sqlite")]
        {
            super::sqlite::replace_turns_granted(granted, self.runtime.config())
        }
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = granted;
            Err(MemoryBackendError::Execution {
                operation: "replace_turns",
                source: "sqlite memory is disabled in this build".into(),
            })
        }
    }

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<Self::StageEnvelope, MemoryBackendError> {
        self.runtime.read_stage_envelope(granted).await
    }

    async fn compact(
        &self,
        granted: Granted<MemoryCompactAction>,
    ) -> Result<Self::CompactOutput, MemoryBackendError> {
        self.runtime.compact(granted).await
    }
}

pub struct SystemBackedMemorySystemRuntime {
    config: MemoryRuntimeConfig,
    metadata: MemorySystemMetadata,
    system: Arc<dyn MemorySystem>,
}

impl SystemBackedMemorySystemRuntime {
    pub fn new(
        config: MemoryRuntimeConfig,
        metadata: MemorySystemMetadata,
        system: Arc<dyn MemorySystem>,
    ) -> Self {
        Self {
            config,
            metadata,
            system,
        }
    }
}

#[async_trait]
impl MemorySystemRuntime for SystemBackedMemorySystemRuntime {
    fn metadata(&self) -> &MemorySystemMetadata {
        &self.metadata
    }

    fn config(&self) -> &MemoryRuntimeConfig {
        &self.config
    }

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<StageEnvelope, MemoryBackendError> {
        let action = granted.as_ref();
        let session_id = action.session_id();
        let workspace_root = action.workspace_root();
        let orchestrator = BuiltinMemoryOrchestrator;
        let system = self.system.as_ref();
        let metadata = &self.metadata;
        let config = &self.config;
        let envelope = orchestrator
            .hydrate_stage_envelope(session_id, workspace_root, config, system, metadata)
            .map_err(|source| MemoryBackendError::Execution {
                operation: "read_stage_envelope",
                source: source.into(),
            })?;

        Ok(envelope)
    }

    async fn compact(
        &self,
        granted: Granted<MemoryCompactAction>,
    ) -> Result<StageDiagnostics, MemoryBackendError> {
        let action = granted.as_ref();
        let session_id = action.session_id();
        let workspace_root = action.workspace_root();
        let family = MemoryStageFamily::Compact;
        let supports_compact_stage = self.metadata.supports_stage_family(family);
        if !supports_compact_stage {
            let diagnostics = skipped_stage_diagnostics(family, None);
            return Ok(diagnostics);
        }

        let compact_stage_result =
            self.system
                .run_compact_stage(session_id, workspace_root, &self.config);
        match compact_stage_result {
            Ok(Some(diagnostics)) => Ok(diagnostics),
            Ok(None) => {
                let diagnostics = skip_compact_stage_without_execution_adapter(family);
                Ok(diagnostics)
            }
            Err(error) if self.config.effective_fail_open() => {
                let diagnostics = StageDiagnostics {
                    family,
                    outcome: super::StageOutcome::Fallback,
                    budget_ms: None,
                    elapsed_ms: None,
                    fallback_activated: true,
                    message: Some(error),
                    planner_snapshot: None,
                };
                Ok(diagnostics)
            }
            Err(source) => Err(MemoryBackendError::Execution {
                operation: "compact",
                source: source.into(),
            }),
        }
    }
}

pub struct BuiltinMemorySystemRuntime {
    config: MemoryRuntimeConfig,
    metadata: MemorySystemMetadata,
    system: Arc<dyn MemorySystem>,
}

impl BuiltinMemorySystemRuntime {
    pub fn new(
        config: MemoryRuntimeConfig,
        metadata: MemorySystemMetadata,
        system: Arc<dyn MemorySystem>,
    ) -> Self {
        Self {
            config,
            metadata,
            system,
        }
    }
}

#[async_trait]
impl MemorySystemRuntime for BuiltinMemorySystemRuntime {
    fn metadata(&self) -> &MemorySystemMetadata {
        &self.metadata
    }

    fn config(&self) -> &MemoryRuntimeConfig {
        &self.config
    }

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<StageEnvelope, MemoryBackendError> {
        let action = granted.as_ref();
        let session_id = action.session_id();
        let workspace_root = action.workspace_root();
        let orchestrator = BuiltinMemoryOrchestrator;
        let system = self.system.as_ref();
        let metadata = &self.metadata;
        let config = &self.config;
        let envelope = orchestrator
            .hydrate_stage_envelope(session_id, workspace_root, config, system, metadata)
            .map_err(|source| MemoryBackendError::Execution {
                operation: "read_stage_envelope",
                source: source.into(),
            })?;

        Ok(envelope)
    }

    async fn compact(
        &self,
        granted: Granted<MemoryCompactAction>,
    ) -> Result<StageDiagnostics, MemoryBackendError> {
        let action = granted.as_ref();
        let session_id = action.session_id();
        let workspace_root = action.workspace_root();
        let diagnostics = super::orchestrator::run_builtin_compact_stage(
            session_id,
            workspace_root,
            &self.config,
        )
        .await
        .map_err(|source| MemoryBackendError::Execution {
            operation: "compact",
            source: source.into(),
        })?;

        Ok(diagnostics)
    }
}

pub struct MetadataOnlyMemorySystemRuntime {
    config: MemoryRuntimeConfig,
    metadata: MemorySystemMetadata,
}

impl MetadataOnlyMemorySystemRuntime {
    pub fn new(config: MemoryRuntimeConfig, metadata: MemorySystemMetadata) -> Self {
        Self { config, metadata }
    }
}

#[async_trait]
impl MemorySystemRuntime for MetadataOnlyMemorySystemRuntime {
    fn metadata(&self) -> &MemorySystemMetadata {
        &self.metadata
    }

    fn config(&self) -> &MemoryRuntimeConfig {
        &self.config
    }

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<StageEnvelope, MemoryBackendError> {
        let session_id = granted.as_ref().session_id();
        let envelope = hydrate_stage_envelope_without_execution_adapter(
            session_id,
            &self.config,
            &self.metadata,
        )
        .map_err(|source| MemoryBackendError::Execution {
            operation: "read_stage_envelope",
            source: source.into(),
        })?;

        Ok(envelope)
    }

    async fn compact(
        &self,
        _granted: Granted<MemoryCompactAction>,
    ) -> Result<StageDiagnostics, MemoryBackendError> {
        let family = MemoryStageFamily::Compact;
        let supports_compact_stage = self.metadata.supports_stage_family(family);
        if !supports_compact_stage {
            let diagnostics = skipped_stage_diagnostics(family, None);
            return Ok(diagnostics);
        }

        let diagnostics = skip_compact_stage_without_execution_adapter(family);
        Ok(diagnostics)
    }
}
