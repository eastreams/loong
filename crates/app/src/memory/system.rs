use std::path::Path;

use std::collections::BTreeSet;

use crate::CliResult;
use serde_json::Value;

use super::system_runtime::{BuiltinMemorySystemRuntime, MemorySystemRuntime};
use super::{
    CanonicalMemoryKind, CanonicalMemorySearchHit, DerivedMemoryKind, MemoryAuthority,
    MemoryContextEntry, MemoryContextKind, MemoryContextProvenance, MemoryProvenanceSourceKind,
    MemoryRecallMode, MemoryRecordStatus, MemoryRetrievalRequest, MemoryRetrievalStrategy,
    MemoryScope, MemoryStageFamily, MemoryTrustLevel, StageDiagnostics, WindowTurn,
    builtin_pre_assembly_stage_families, durable_recall, runtime_config::MemoryRuntimeConfig,
};
use crate::memory::stage::MemoryRetrievalPlanResult;

mod system_builtin;
mod system_recall;
#[cfg(test)]
#[path = "system/system_tests.rs"]
mod tests;

pub use self::system_builtin::BuiltinMemorySystem;
#[cfg(test)]
use self::system_builtin::{
    BuiltinRetrievalPlan, BuiltinRetrievalPlannerInputs, SeededWorkspaceRetrievalPlan,
    build_cross_session_recall_entries, delegate_lineage_query_seed_from_metadata,
    filter_cross_session_recall_hits, structured_signal_query_seed_from_records,
    workflow_task_policy_from_metadata, workflow_task_query_seed_from_metadata,
};
use self::system_builtin::{build_builtin_retrieval_plan_result, run_builtin_retrieve_stage};
pub use self::system_recall::{RecallFirstMemorySystem, WorkspaceRecallMemorySystem};

pub const MEMORY_SYSTEM_API_VERSION: u16 = 1;
pub const DEFAULT_MEMORY_SYSTEM_ID: &str = "builtin";
pub const WORKSPACE_RECALL_MEMORY_SYSTEM_ID: &str = "workspace_recall";
pub const RECALL_FIRST_MEMORY_SYSTEM_ID: &str = "recall_first";

#[cfg(feature = "memory-sqlite")]
const MAX_CROSS_SESSION_RECALL_SEARCH_CANDIDATES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemorySystemCapability {
    CanonicalStore,
    PromptHydration,
    DeterministicSummary,
    ProfileNoteProjection,
    RetrievalProvenance,
}

impl MemorySystemCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalStore => "canonical_store",
            Self::PromptHydration => "prompt_hydration",
            Self::DeterministicSummary => "deterministic_summary",
            Self::ProfileNoteProjection => "profile_note_projection",
            Self::RetrievalProvenance => "retrieval_provenance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySystemRuntimeFallbackKind {
    MetadataOnly,
    SystemBacked,
}

impl MemorySystemRuntimeFallbackKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MetadataOnly => "metadata_only",
            Self::SystemBacked => "system_backed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySystemMetadata {
    pub id: &'static str,
    pub api_version: u16,
    pub capabilities: BTreeSet<MemorySystemCapability>,
    pub summary: &'static str,
    pub runtime_fallback_kind: MemorySystemRuntimeFallbackKind,
    pub supported_stage_families: Vec<MemoryStageFamily>,
    pub supported_pre_assembly_stage_families: Vec<MemoryStageFamily>,
    pub supported_recall_modes: Vec<MemoryRecallMode>,
}

impl MemorySystemMetadata {
    pub fn new(
        id: &'static str,
        capabilities: impl IntoIterator<Item = MemorySystemCapability>,
        summary: &'static str,
    ) -> Self {
        Self {
            id,
            api_version: MEMORY_SYSTEM_API_VERSION,
            capabilities: capabilities.into_iter().collect(),
            summary,
            runtime_fallback_kind: MemorySystemRuntimeFallbackKind::MetadataOnly,
            supported_stage_families: Vec::new(),
            supported_pre_assembly_stage_families: Vec::new(),
            supported_recall_modes: Vec::new(),
        }
    }

    pub fn with_runtime_fallback_kind(
        mut self,
        runtime_fallback_kind: MemorySystemRuntimeFallbackKind,
    ) -> Self {
        self.runtime_fallback_kind = runtime_fallback_kind;
        self
    }

    pub fn with_supported_stage_families(
        mut self,
        families: impl IntoIterator<Item = MemoryStageFamily>,
    ) -> Self {
        let collected_families = dedupe_stage_families(families);
        let pre_assembly_families = self.supported_pre_assembly_stage_families.clone();
        let combined_families = pre_assembly_families
            .into_iter()
            .chain(collected_families)
            .collect::<Vec<_>>();
        let normalized_families = dedupe_stage_families(combined_families);
        self.supported_stage_families = normalized_families;
        let has_supported_stages = !self.supported_stage_families.is_empty();
        if self.runtime_fallback_kind == MemorySystemRuntimeFallbackKind::MetadataOnly
            && has_supported_stages
        {
            self.runtime_fallback_kind = MemorySystemRuntimeFallbackKind::SystemBacked;
        }
        self
    }

    pub fn with_supported_pre_assembly_stage_families(
        mut self,
        families: impl IntoIterator<Item = MemoryStageFamily>,
    ) -> Self {
        let collected_families = dedupe_stage_families(families);
        let previous_pre_assembly_families = self.supported_pre_assembly_stage_families.clone();
        let additional_stage_families = self
            .supported_stage_families
            .iter()
            .copied()
            .filter(|family| !previous_pre_assembly_families.contains(family))
            .collect::<Vec<_>>();
        let combined_families = collected_families
            .iter()
            .copied()
            .chain(additional_stage_families)
            .collect::<Vec<_>>();
        let normalized_stage_families = dedupe_stage_families(combined_families);
        self.supported_pre_assembly_stage_families = collected_families;
        self.supported_stage_families = normalized_stage_families;
        let has_supported_stages = !self.supported_stage_families.is_empty();
        if self.runtime_fallback_kind == MemorySystemRuntimeFallbackKind::MetadataOnly
            && has_supported_stages
        {
            self.runtime_fallback_kind = MemorySystemRuntimeFallbackKind::SystemBacked;
        }
        self
    }

    pub fn with_supported_recall_modes(
        mut self,
        recall_modes: impl IntoIterator<Item = MemoryRecallMode>,
    ) -> Self {
        self.supported_recall_modes = recall_modes.into_iter().collect();
        self
    }

    pub fn capability_names(&self) -> Vec<&'static str> {
        let mut names = self
            .capabilities
            .iter()
            .copied()
            .map(MemorySystemCapability::as_str)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    pub fn supports_pre_assembly_stage_family(&self, family: MemoryStageFamily) -> bool {
        self.supported_pre_assembly_stage_families.contains(&family)
    }

    pub fn supports_stage_family(&self, family: MemoryStageFamily) -> bool {
        self.supported_stage_families.contains(&family)
    }
}

fn dedupe_stage_families(
    families: impl IntoIterator<Item = MemoryStageFamily>,
) -> Vec<MemoryStageFamily> {
    let mut deduped_families = Vec::new();

    for family in families {
        let already_present = deduped_families.contains(&family);
        if already_present {
            continue;
        }

        deduped_families.push(family);
    }

    deduped_families
}

pub trait MemorySystem: Send + Sync {
    fn id(&self) -> &'static str;

    fn metadata(&self) -> MemorySystemMetadata;

    fn create_runtime(
        &self,
        config: &MemoryRuntimeConfig,
    ) -> CliResult<Option<Box<dyn MemorySystemRuntime>>> {
        let _ = config;

        Ok(None)
    }

    // Compatibility matrix:
    // - request-only system: `build_retrieval_plan_result()` upgrades via request by default
    // - plan-result-only system: `build_retrieval_request()` stays `None` by default
    // - `build_retrieval_request_via_plan_result()` is the explicit back-compat bridge
    // - `build_retrieval_plan_result_via_request()` is the explicit forward-compat bridge

    /// Legacy compatibility hook for systems that only know how to describe retrieval as a
    /// request. New implementations should prefer `build_retrieval_plan_result`.
    fn build_retrieval_request(
        &self,
        _session_id: &str,
        _workspace_root: Option<&Path>,
        _config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        None
    }

    /// Explicit compatibility adapter for plan-result-first systems that still need to surface
    /// a legacy retrieval request at call sites that have not moved to plan results yet.
    fn build_retrieval_request_via_plan_result(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        self.build_retrieval_plan_result(session_id, workspace_root, config, recent_window)
            .map(MemoryRetrievalPlanResult::into_request)
    }

    /// Explicit compatibility adapter for request-first systems that have not yet implemented a
    /// native retrieval plan result. This mirrors `build_retrieval_request_via_plan_result` but
    /// does not change the default asymmetry of the trait surface.
    fn build_retrieval_plan_result_via_request(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalPlanResult> {
        self.build_retrieval_request(session_id, workspace_root, config, recent_window)
            .map(MemoryRetrievalRequest::into_plan_result)
    }

    /// Preferred retrieval-planning surface. The default implementation only upgrades legacy
    /// request builders into a structured plan result; it does not automatically adapt a
    /// plan-result-only implementation back into `build_retrieval_request`.
    fn build_retrieval_plan_result(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalPlanResult> {
        self.build_retrieval_plan_result_via_request(
            session_id,
            workspace_root,
            config,
            recent_window,
        )
    }

    fn run_derive_stage(
        &self,
        _session_id: &str,
        _config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        Ok(None)
    }

    fn run_retrieve_stage(
        &self,
        _request: &MemoryRetrievalRequest,
        _workspace_root: Option<&Path>,
        _config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        Ok(None)
    }

    fn run_rank_stage(
        &self,
        _entries: Vec<MemoryContextEntry>,
        _config: &MemoryRuntimeConfig,
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        Ok(None)
    }

    fn run_compact_stage(
        &self,
        _session_id: &str,
        _workspace_root: Option<&Path>,
        _config: &MemoryRuntimeConfig,
    ) -> Result<Option<StageDiagnostics>, String> {
        Ok(None)
    }
}

impl<T> MemorySystem for Box<T>
where
    T: MemorySystem + ?Sized,
{
    fn id(&self) -> &'static str {
        self.as_ref().id()
    }

    fn metadata(&self) -> MemorySystemMetadata {
        self.as_ref().metadata()
    }

    fn create_runtime(
        &self,
        config: &MemoryRuntimeConfig,
    ) -> CliResult<Option<Box<dyn MemorySystemRuntime>>> {
        self.as_ref().create_runtime(config)
    }

    fn build_retrieval_request(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        self.as_ref()
            .build_retrieval_request(session_id, workspace_root, config, recent_window)
    }

    fn build_retrieval_request_via_plan_result(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        self.as_ref().build_retrieval_request_via_plan_result(
            session_id,
            workspace_root,
            config,
            recent_window,
        )
    }

    fn build_retrieval_plan_result_via_request(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalPlanResult> {
        self.as_ref().build_retrieval_plan_result_via_request(
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
        self.as_ref()
            .build_retrieval_plan_result(session_id, workspace_root, config, recent_window)
    }

    fn run_derive_stage(
        &self,
        session_id: &str,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        self.as_ref()
            .run_derive_stage(session_id, config, recent_window)
    }

    fn run_retrieve_stage(
        &self,
        request: &MemoryRetrievalRequest,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        self.as_ref()
            .run_retrieve_stage(request, workspace_root, config, recent_window)
    }

    fn run_rank_stage(
        &self,
        entries: Vec<MemoryContextEntry>,
        config: &MemoryRuntimeConfig,
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        self.as_ref().run_rank_stage(entries, config)
    }

    fn run_compact_stage(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
    ) -> Result<Option<StageDiagnostics>, String> {
        self.as_ref()
            .run_compact_stage(session_id, workspace_root, config)
    }
}
