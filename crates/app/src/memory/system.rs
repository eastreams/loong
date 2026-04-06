use std::collections::BTreeSet;

use super::{
    BuiltinMemoryPreAssemblyExecutor, MemoryPreAssemblyExecutor, MemoryStageFamily,
    RecallFirstMemoryPreAssemblyExecutor, builtin_pre_assembly_stage_families,
};

pub const MEMORY_SYSTEM_API_VERSION: u16 = 1;
pub const DEFAULT_MEMORY_SYSTEM_ID: &str = "builtin";
pub const RECALL_FIRST_MEMORY_SYSTEM_ID: &str = "recall_first";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemorySystemCapability {
    CanonicalStore,
    PromptHydration,
    DeterministicSummary,
    ProfileNoteProjection,
}

impl MemorySystemCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalStore => "canonical_store",
            Self::PromptHydration => "prompt_hydration",
            Self::DeterministicSummary => "deterministic_summary",
            Self::ProfileNoteProjection => "profile_note_projection",
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
        }
    }

    pub fn with_supported_pre_assembly_stage_families(
        mut self,
        families: impl IntoIterator<Item = MemoryStageFamily>,
    ) -> Self {
        self.supported_pre_assembly_stage_families = families.into_iter().collect();
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

    fn pre_assembly_executor(&self) -> Option<Box<dyn MemoryPreAssemblyExecutor>> {
        None
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

    fn pre_assembly_executor(&self) -> Option<Box<dyn MemoryPreAssemblyExecutor>> {
        self.as_ref().pre_assembly_executor()
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
            ],
            "Built-in SQLite-backed canonical memory with deterministic prompt hydration.",
        )
        .with_supported_pre_assembly_stage_families(builtin_pre_assembly_stage_families())
    }

    fn pre_assembly_executor(&self) -> Option<Box<dyn MemoryPreAssemblyExecutor>> {
        let executor = BuiltinMemoryPreAssemblyExecutor;
        let boxed_executor = Box::new(executor);
        Some(boxed_executor)
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
            [MemorySystemCapability::PromptHydration],
            "In-tree alternate memory system that prioritizes advisory recall before summary when recall is available.",
        )
        .with_supported_pre_assembly_stage_families([
            MemoryStageFamily::Retrieve,
            MemoryStageFamily::Rank,
        ])
    }

    fn pre_assembly_executor(&self) -> Option<Box<dyn MemoryPreAssemblyExecutor>> {
        let executor = RecallFirstMemoryPreAssemblyExecutor;
        let boxed_executor = Box::new(executor);
        Some(boxed_executor)
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
            ]
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
    fn recall_first_memory_system_metadata_is_stable() {
        let metadata = RecallFirstMemorySystem.metadata();

        assert_eq!(metadata.id, RECALL_FIRST_MEMORY_SYSTEM_ID);
        assert_eq!(metadata.api_version, MEMORY_SYSTEM_API_VERSION);
        assert_eq!(metadata.capability_names(), vec!["prompt_hydration"]);
        assert_eq!(
            metadata.supported_pre_assembly_stage_families,
            vec![MemoryStageFamily::Retrieve, MemoryStageFamily::Rank]
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
                .any(|entry| entry.id == RECALL_FIRST_MEMORY_SYSTEM_ID)
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
}
