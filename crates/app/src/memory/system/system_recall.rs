use super::*;
use super::system_builtin::{
    build_workspace_retrieval_plan_result, derive_session_overview_entry,
    rank_recall_first_entries,
};

#[derive(Default)]
pub struct WorkspaceRecallMemorySystem;

impl MemorySystem for WorkspaceRecallMemorySystem {
    fn id(&self) -> &'static str {
        WORKSPACE_RECALL_MEMORY_SYSTEM_ID
    }

    fn metadata(&self) -> MemorySystemMetadata {
        MemorySystemMetadata::new(
            WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
            [
                MemorySystemCapability::PromptHydration,
                MemorySystemCapability::RetrievalProvenance,
            ],
            "Workspace-document recall system with provenance-aware retrieval and rank-stage reordering.",
        )
        .with_supported_pre_assembly_stage_families([
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ])
        .with_runtime_fallback_kind(MemorySystemRuntimeFallbackKind::SystemBacked)
        .with_supported_recall_modes([
            MemoryRecallMode::PromptAssembly,
            MemoryRecallMode::OperatorInspection,
        ])
    }

    fn build_retrieval_request(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        self.build_retrieval_request_via_plan_result(session_id, workspace_root, config, &[])
    }

    fn build_retrieval_plan_result(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalPlanResult> {
        build_workspace_retrieval_plan_result(self.id(), session_id, workspace_root, config)
    }

    fn run_derive_stage(
        &self,
        session_id: &str,
        _config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        let maybe_entry = derive_session_overview_entry(session_id, recent_window, self.id());
        let entries = maybe_entry.into_iter().collect::<Vec<_>>();

        Ok(Some(entries))
    }

    fn run_retrieve_stage(
        &self,
        request: &MemoryRetrievalRequest,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        let entries = durable_recall::load_workspace_document_recall_entries(
            workspace_root,
            config,
            self.id(),
            request.recall_mode,
            request.scopes.as_slice(),
            request.budget_items,
        )?;
        Ok(Some(entries))
    }

    fn run_rank_stage(
        &self,
        entries: Vec<MemoryContextEntry>,
        _config: &MemoryRuntimeConfig,
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        let ranked_entries = rank_recall_first_entries(entries);

        Ok(Some(ranked_entries))
    }
}

#[derive(Default)]
pub struct RecallFirstMemorySystem;

impl MemorySystem for RecallFirstMemorySystem {
    fn id(&self) -> &'static str {
        RECALL_FIRST_MEMORY_SYSTEM_ID
    }

    fn metadata(&self) -> MemorySystemMetadata {
        MemorySystemMetadata::new(
            RECALL_FIRST_MEMORY_SYSTEM_ID,
            [
                MemorySystemCapability::PromptHydration,
                MemorySystemCapability::RetrievalProvenance,
            ],
            "Recall-first memory system with provenance-aware retrieval and summary suppression when recall is available.",
        )
        .with_supported_pre_assembly_stage_families([
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ])
        .with_runtime_fallback_kind(MemorySystemRuntimeFallbackKind::SystemBacked)
        .with_supported_recall_modes([MemoryRecallMode::PromptAssembly])
    }

    fn build_retrieval_request(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        self.build_retrieval_request_via_plan_result(
            session_id,
            workspace_root,
            config,
            recent_window,
        )
    }

    fn build_retrieval_plan_result(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalPlanResult> {
        build_builtin_retrieval_plan_result(
            self.id(),
            session_id,
            workspace_root,
            config,
            recent_window,
        )
    }

    fn run_derive_stage(
        &self,
        session_id: &str,
        _config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        let maybe_entry = derive_session_overview_entry(session_id, recent_window, self.id());
        let entries = maybe_entry.into_iter().collect::<Vec<_>>();

        Ok(Some(entries))
    }

    fn run_retrieve_stage(
        &self,
        request: &MemoryRetrievalRequest,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        run_builtin_retrieve_stage(self.id(), request, workspace_root, config)
    }

    fn run_rank_stage(
        &self,
        entries: Vec<MemoryContextEntry>,
        _config: &MemoryRuntimeConfig,
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        let ranked_entries = rank_recall_first_entries(entries);
        Ok(Some(ranked_entries))
    }
}
