use std::path::Path;

use std::collections::BTreeSet;

use super::{
    DerivedMemoryKind, MemoryContextEntry, MemoryContextKind, MemoryContextProvenance,
    MemoryProvenanceSourceKind, MemoryRecallMode, MemoryRetrievalRequest, MemoryScope,
    MemoryStageFamily, WindowTurn, builtin_pre_assembly_stage_families, durable_recall,
    runtime_config::MemoryRuntimeConfig,
};

pub const MEMORY_SYSTEM_API_VERSION: u16 = 1;
pub const DEFAULT_MEMORY_SYSTEM_ID: &str = "builtin";
pub const WORKSPACE_RECALL_MEMORY_SYSTEM_ID: &str = "workspace_recall";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySystemMetadata {
    pub id: &'static str,
    pub api_version: u16,
    pub capabilities: BTreeSet<MemorySystemCapability>,
    pub summary: &'static str,
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
            supported_pre_assembly_stage_families: Vec::new(),
            supported_recall_modes: Vec::new(),
        }
    }

    pub fn with_supported_pre_assembly_stage_families(
        mut self,
        families: impl IntoIterator<Item = MemoryStageFamily>,
    ) -> Self {
        self.supported_pre_assembly_stage_families = families.into_iter().collect();
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
}

pub trait MemorySystem: Send + Sync {
    fn id(&self) -> &'static str;

    fn metadata(&self) -> MemorySystemMetadata;

    fn build_retrieval_request(
        &self,
        _session_id: &str,
        _workspace_root: Option<&Path>,
        _config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        None
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
}

#[derive(Default)]
pub struct BuiltinMemorySystem;

impl MemorySystem for BuiltinMemorySystem {
    fn id(&self) -> &'static str {
        DEFAULT_MEMORY_SYSTEM_ID
    }

    fn metadata(&self) -> MemorySystemMetadata {
        MemorySystemMetadata::new(
            DEFAULT_MEMORY_SYSTEM_ID,
            [
                MemorySystemCapability::CanonicalStore,
                MemorySystemCapability::PromptHydration,
                MemorySystemCapability::DeterministicSummary,
                MemorySystemCapability::ProfileNoteProjection,
                MemorySystemCapability::RetrievalProvenance,
            ],
            "Built-in SQLite-backed canonical memory with deterministic prompt hydration.",
        )
        .with_supported_pre_assembly_stage_families(builtin_pre_assembly_stage_families())
        .with_supported_recall_modes([MemoryRecallMode::PromptAssembly])
    }

    fn build_retrieval_request(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        let supports_query_recall =
            matches!(config.mode, crate::config::MemoryMode::WindowPlusSummary);
        let has_workspace_root = workspace_root.is_some();
        let query = if supports_query_recall {
            super::orchestrator::retrieval_query_from_recent_window(recent_window)
        } else {
            None
        };
        let has_query = query.is_some();
        let has_retrieval_path = has_workspace_root || has_query;
        if !has_retrieval_path {
            return None;
        }

        let (scopes, allowed_kinds, budget_items) = if has_query {
            let scopes = vec![
                MemoryScope::Session,
                MemoryScope::Workspace,
                MemoryScope::Agent,
                MemoryScope::User,
            ];
            let mut allowed_kinds = vec![
                DerivedMemoryKind::Profile,
                DerivedMemoryKind::Fact,
                DerivedMemoryKind::Episode,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Overview,
            ];
            if has_workspace_root {
                allowed_kinds.push(DerivedMemoryKind::Reference);
            }
            let budget_items = if recent_window.is_empty() {
                6
            } else {
                config.sliding_window.min(6)
            };
            (scopes, allowed_kinds, budget_items)
        } else {
            let scopes = vec![MemoryScope::Workspace, MemoryScope::Session];
            let allowed_kinds = vec![DerivedMemoryKind::Reference];
            let budget_items = 1;
            (scopes, allowed_kinds, budget_items)
        };

        let retrieval_request = MemoryRetrievalRequest {
            session_id: session_id.to_owned(),
            memory_system_id: self.id().to_owned(),
            query,
            recall_mode: MemoryRecallMode::PromptAssembly,
            scopes,
            budget_items,
            allowed_kinds,
        };

        Some(retrieval_request)
    }

    fn run_derive_stage(
        &self,
        _session_id: &str,
        _config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        Ok(Some(Vec::new()))
    }

    fn run_retrieve_stage(
        &self,
        request: &MemoryRetrievalRequest,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        let mut entries = durable_recall::load_durable_recall_entries(
            workspace_root,
            config,
            self.id(),
            request.recall_mode,
        )?;

        #[cfg(feature = "memory-sqlite")]
        if let Some(query) = request.query.as_deref() {
            let hits = super::sqlite::search_canonical_records_for_recall(
                query,
                request.budget_items,
                Some(request.session_id.as_str()),
                config,
            )?;
            if !hits.is_empty() {
                let content =
                    super::orchestrator::render_cross_session_recall_block(hits.as_slice());
                let provenance = MemoryContextProvenance::new(
                    self.id(),
                    MemoryProvenanceSourceKind::MemorySystem,
                    Some("cross_session_recall".to_owned()),
                    None,
                    Some(MemoryScope::Session),
                    request.recall_mode,
                );
                let entry = MemoryContextEntry {
                    kind: MemoryContextKind::RetrievedMemory,
                    role: "system".to_owned(),
                    content,
                    provenance: vec![provenance],
                };
                entries.push(entry);
            }
        }

        Ok(Some(entries))
    }

    fn run_rank_stage(
        &self,
        entries: Vec<MemoryContextEntry>,
        _config: &MemoryRuntimeConfig,
    ) -> Result<Option<Vec<MemoryContextEntry>>, String> {
        Ok(Some(entries))
    }
}

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
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ])
        .with_supported_recall_modes([MemoryRecallMode::PromptAssembly])
    }

    fn build_retrieval_request(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        _recent_window: &[WindowTurn],
    ) -> Option<MemoryRetrievalRequest> {
        let has_workspace_root = workspace_root.is_some();
        if !has_workspace_root {
            return None;
        }

        let budget_items = config.sliding_window.min(4);
        let normalized_budget_items = budget_items.max(1);

        Some(MemoryRetrievalRequest {
            session_id: session_id.to_owned(),
            memory_system_id: self.id().to_owned(),
            query: None,
            recall_mode: MemoryRecallMode::PromptAssembly,
            scopes: vec![crate::memory::MemoryScope::Workspace],
            budget_items: normalized_budget_items,
            allowed_kinds: vec![crate::memory::DerivedMemoryKind::Reference],
        })
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
        let mut profile_entries = Vec::new();
        let mut retrieved_entries = Vec::new();
        let mut summary_entries = Vec::new();
        let mut history_entries = Vec::new();

        for entry in entries {
            match entry.kind {
                MemoryContextKind::Profile => profile_entries.push(entry),
                MemoryContextKind::RetrievedMemory => retrieved_entries.push(entry),
                MemoryContextKind::Summary => summary_entries.push(entry),
                MemoryContextKind::Turn => history_entries.push(entry),
            }
        }

        let has_retrieved_entries = !retrieved_entries.is_empty();
        let mut ranked_entries = Vec::new();
        ranked_entries.extend(profile_entries);
        ranked_entries.extend(retrieved_entries);
        if !has_retrieved_entries {
            ranked_entries.extend(summary_entries);
        }
        ranked_entries.extend(history_entries);

        Ok(Some(ranked_entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StageAwareRegistryMemorySystem;

    impl MemorySystem for StageAwareRegistryMemorySystem {
        fn id(&self) -> &'static str {
            "registry-stage-aware"
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                "registry-stage-aware",
                [MemorySystemCapability::PromptHydration],
                "Registry stage-aware test system",
            )
            .with_supported_pre_assembly_stage_families([MemoryStageFamily::Retrieve])
        }
    }

    #[test]
    fn builtin_memory_system_metadata_is_stable() {
        let metadata = BuiltinMemorySystem.metadata();
        assert_eq!(metadata.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(metadata.api_version, MEMORY_SYSTEM_API_VERSION);
        assert_eq!(
            metadata.capability_names(),
            vec![
                "canonical_store",
                "deterministic_summary",
                "profile_note_projection",
                "prompt_hydration",
                "retrieval_provenance",
            ]
        );
        assert_eq!(
            metadata.supported_recall_modes,
            vec![MemoryRecallMode::PromptAssembly]
        );
    }

    #[test]
    fn memory_system_field_exposes_builtin_pre_assembly_stage_families() {
        let metadata = BuiltinMemorySystem.metadata();
        assert_eq!(
            metadata.supported_pre_assembly_stage_families,
            builtin_pre_assembly_stage_families()
        );
    }

    #[test]
    fn memory_system_field_allows_custom_registry_stage_family_sets() {
        let custom = StageAwareRegistryMemorySystem.metadata();
        assert_eq!(custom.id, "registry-stage-aware");
        assert_eq!(
            custom.supported_pre_assembly_stage_families,
            vec![MemoryStageFamily::Retrieve]
        );
        assert!(custom.supported_recall_modes.is_empty());
    }

    #[test]
    fn memory_system_registry_includes_builtin_metadata() {
        let metadata = crate::memory::list_memory_system_metadata().expect("list memory systems");
        assert!(
            metadata
                .iter()
                .any(|entry| entry.id == DEFAULT_MEMORY_SYSTEM_ID)
        );
        assert!(
            metadata
                .iter()
                .any(|entry| entry.id == WORKSPACE_RECALL_MEMORY_SYSTEM_ID)
        );
    }

    #[test]
    fn workspace_recall_memory_system_metadata_is_stable() {
        let metadata = WorkspaceRecallMemorySystem.metadata();
        assert_eq!(metadata.id, WORKSPACE_RECALL_MEMORY_SYSTEM_ID);
        assert_eq!(
            metadata.capability_names(),
            vec!["prompt_hydration", "retrieval_provenance"]
        );
        assert_eq!(
            metadata.supported_pre_assembly_stage_families,
            vec![MemoryStageFamily::Retrieve, MemoryStageFamily::Rank]
        );
        assert_eq!(
            metadata.supported_recall_modes,
            vec![MemoryRecallMode::PromptAssembly]
        );
    }

    #[test]
    fn memory_system_runtime_snapshot_defaults_to_builtin() {
        let config = crate::config::LoongClawConfig::default();
        let snapshot = crate::memory::collect_memory_system_runtime_snapshot(&config)
            .expect("collect memory-system snapshot");
        assert_eq!(snapshot.selected.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(
            snapshot.selected.source,
            crate::memory::MemorySystemSelectionSource::Default
        );
        assert_eq!(snapshot.selected_metadata.id, DEFAULT_MEMORY_SYSTEM_ID);
    }

    #[test]
    fn workspace_recall_rank_stage_keeps_summary_without_retrieved_entries() {
        let entries = vec![
            MemoryContextEntry {
                kind: MemoryContextKind::Profile,
                role: "system".to_owned(),
                content: "profile".to_owned(),
                provenance: Vec::new(),
            },
            MemoryContextEntry {
                kind: MemoryContextKind::Summary,
                role: "system".to_owned(),
                content: "summary".to_owned(),
                provenance: Vec::new(),
            },
            MemoryContextEntry {
                kind: MemoryContextKind::Turn,
                role: "user".to_owned(),
                content: "turn".to_owned(),
                provenance: Vec::new(),
            },
        ];

        let ranked_entries = WorkspaceRecallMemorySystem
            .run_rank_stage(entries, &MemoryRuntimeConfig::default())
            .expect("rank stage should succeed")
            .expect("workspace recall rank stage should return entries");

        let kinds = ranked_entries
            .into_iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                MemoryContextKind::Profile,
                MemoryContextKind::Summary,
                MemoryContextKind::Turn,
            ]
        );
    }

    #[test]
    fn workspace_recall_rank_stage_drops_summary_when_retrieved_entries_exist() {
        let entries = vec![
            MemoryContextEntry {
                kind: MemoryContextKind::Summary,
                role: "system".to_owned(),
                content: "summary".to_owned(),
                provenance: Vec::new(),
            },
            MemoryContextEntry {
                kind: MemoryContextKind::RetrievedMemory,
                role: "system".to_owned(),
                content: "retrieved".to_owned(),
                provenance: Vec::new(),
            },
            MemoryContextEntry {
                kind: MemoryContextKind::Turn,
                role: "user".to_owned(),
                content: "turn".to_owned(),
                provenance: Vec::new(),
            },
        ];

        let ranked_entries = WorkspaceRecallMemorySystem
            .run_rank_stage(entries, &MemoryRuntimeConfig::default())
            .expect("rank stage should succeed")
            .expect("workspace recall rank stage should return entries");

        let kinds = ranked_entries
            .into_iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![MemoryContextKind::RetrievedMemory, MemoryContextKind::Turn]
        );
    }
}
