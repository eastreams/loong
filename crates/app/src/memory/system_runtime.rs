use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use loong_contracts::{MemoryCoreOutcome, MemoryCoreRequest};

use super::orchestrator::{
    BuiltinMemoryOrchestrator, hydrate_stage_envelope_without_execution_adapter,
    skip_compact_stage_without_execution_adapter, skipped_stage_diagnostics,
};
use super::runtime_config::MemoryRuntimeConfig;
use super::{
    MemoryCoreOperation, MemoryStageFamily, MemorySystem, MemorySystemMetadata, StageDiagnostics,
    StageEnvelope,
};

#[async_trait]
pub trait MemorySystemRuntime: Send + Sync {
    fn metadata(&self) -> &MemorySystemMetadata;

    fn supported_core_operations(&self) -> Vec<MemoryCoreOperation>;

    fn execute_core(&self, request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String>;

    fn hydrate_stage_envelope(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
    ) -> Result<StageEnvelope, String>;

    async fn run_compact_stage(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
    ) -> Result<StageDiagnostics, String>;
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

    fn supported_core_operations(&self) -> Vec<MemoryCoreOperation> {
        super::supported_memory_core_operations(self.config.backend)
    }

    fn execute_core(&self, request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
        super::execute_builtin_backend_memory_core(request, &self.config)
    }

    fn hydrate_stage_envelope(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
    ) -> Result<StageEnvelope, String> {
        let orchestrator = BuiltinMemoryOrchestrator;
        let system = self.system.as_ref();
        let metadata = &self.metadata;
        let config = &self.config;
        let envelope = orchestrator.hydrate_stage_envelope(
            session_id,
            workspace_root,
            config,
            system,
            metadata,
        )?;

        Ok(envelope)
    }

    async fn run_compact_stage(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
    ) -> Result<StageDiagnostics, String> {
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
                };
                Ok(diagnostics)
            }
            Err(error) => Err(format!("memory compact stage failed: {error}")),
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

    fn supported_core_operations(&self) -> Vec<MemoryCoreOperation> {
        super::supported_memory_core_operations(self.config.backend)
    }

    fn execute_core(&self, request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
        super::execute_builtin_backend_memory_core(request, &self.config)
    }

    fn hydrate_stage_envelope(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
    ) -> Result<StageEnvelope, String> {
        let orchestrator = BuiltinMemoryOrchestrator;
        let system = self.system.as_ref();
        let metadata = &self.metadata;
        let config = &self.config;
        let envelope = orchestrator.hydrate_stage_envelope(
            session_id,
            workspace_root,
            config,
            system,
            metadata,
        )?;

        Ok(envelope)
    }

    async fn run_compact_stage(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
    ) -> Result<StageDiagnostics, String> {
        let diagnostics = super::orchestrator::run_builtin_compact_stage(
            session_id,
            workspace_root,
            &self.config,
        )
        .await?;

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

    fn supported_core_operations(&self) -> Vec<MemoryCoreOperation> {
        super::supported_memory_core_operations(self.config.backend)
    }

    fn execute_core(&self, request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
        super::execute_builtin_backend_memory_core(request, &self.config)
    }

    fn hydrate_stage_envelope(
        &self,
        session_id: &str,
        _workspace_root: Option<&Path>,
    ) -> Result<StageEnvelope, String> {
        let envelope = hydrate_stage_envelope_without_execution_adapter(
            session_id,
            &self.config,
            &self.metadata,
        )?;

        Ok(envelope)
    }

    async fn run_compact_stage(
        &self,
        _session_id: &str,
        _workspace_root: Option<&Path>,
    ) -> Result<StageDiagnostics, String> {
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
