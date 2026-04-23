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

pub(crate) fn build_builtin_retrieval_plan_result(
    memory_system_id: &str,
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) -> Option<MemoryRetrievalPlanResult> {
    let retrieval_plan = BuiltinRetrievalPlan::from_runtime_inputs(
        session_id,
        workspace_root,
        config,
        recent_window,
    )?;

    Some(retrieval_plan.into_result(memory_system_id, session_id))
}

fn build_workspace_retrieval_plan_result(
    memory_system_id: &str,
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Option<MemoryRetrievalPlanResult> {
    let has_workspace_root = workspace_root.is_some();
    if !has_workspace_root {
        return None;
    }

    let budget_items = config.sliding_window.min(4);
    let normalized_budget_items = budget_items.max(1);
    let request = MemoryRetrievalRequest {
        session_id: session_id.to_owned(),
        memory_system_id: memory_system_id.to_owned(),
        strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
        planning_notes: vec!["workspace recall system".to_owned()],
        query: None,
        recall_mode: MemoryRecallMode::PromptAssembly,
        scopes: vec![crate::memory::MemoryScope::Workspace],
        budget_items: normalized_budget_items,
        allowed_kinds: vec![crate::memory::DerivedMemoryKind::Reference],
    };

    Some(request.into_plan_result())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuiltinRetrievalPlan {
    strategy: MemoryRetrievalStrategy,
    planning_notes: Vec<String>,
    query: Option<String>,
    scopes: Vec<MemoryScope>,
    budget_items: usize,
    allowed_kinds: Vec<DerivedMemoryKind>,
}

struct BuiltinRetrievalPlannerInputs {
    has_workspace_root: bool,
    recent_user_query: Option<String>,
    recent_user_budget_items: usize,
    structured_query: Option<String>,
    task_progress_plan: Option<SeededWorkspaceRetrievalPlan>,
    workflow_task_plan: Option<SeededWorkspaceRetrievalPlan>,
    delegate_lineage_plan: Option<SeededWorkspaceRetrievalPlan>,
    has_structured_session_signals: bool,
}

impl BuiltinRetrievalPlan {
    fn new(
        strategy: MemoryRetrievalStrategy,
        planning_notes: Vec<String>,
        query: Option<String>,
        scopes: Vec<MemoryScope>,
        budget_items: usize,
        allowed_kinds: Vec<DerivedMemoryKind>,
    ) -> Self {
        Self {
            strategy,
            planning_notes,
            query,
            scopes,
            budget_items,
            allowed_kinds,
        }
    }

    fn workspace_scoped(
        strategy: MemoryRetrievalStrategy,
        planning_notes: Vec<String>,
        query: Option<String>,
        budget_items: usize,
        allowed_kinds: Vec<DerivedMemoryKind>,
    ) -> Self {
        Self::new(
            strategy,
            planning_notes,
            query,
            vec![MemoryScope::Workspace, MemoryScope::Session],
            budget_items,
            allowed_kinds,
        )
    }

    fn recent_user_query(has_workspace_root: bool, query: String, budget_items: usize) -> Self {
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

        Self::new(
            if has_workspace_root {
                MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
            } else {
                MemoryRetrievalStrategy::RecentUserQuery
            },
            vec![
                "recent_user_query seed".to_owned(),
                if has_workspace_root {
                    "workspace_root present".to_owned()
                } else {
                    "workspace_root absent".to_owned()
                },
            ],
            Some(query),
            vec![
                MemoryScope::Session,
                MemoryScope::Workspace,
                MemoryScope::Agent,
                MemoryScope::User,
            ],
            budget_items,
            allowed_kinds,
        )
    }

    fn from_seeded_workspace(seeded: SeededWorkspaceRetrievalPlan) -> Self {
        Self::workspace_scoped(
            seeded.strategy,
            seeded.planning_notes,
            Some(seeded.query),
            seeded.budget_items,
            seeded.allowed_kinds,
        )
    }

    fn from_runtime_inputs(
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<Self> {
        let has_workspace_root = workspace_root.is_some();
        let supports_query_recall =
            matches!(config.mode, crate::config::MemoryMode::WindowPlusSummary);
        let recent_user_query = if supports_query_recall {
            super::orchestrator::retrieval_query_from_recent_window(recent_window)
        } else {
            None
        };
        let has_retrieval_path = has_workspace_root || recent_user_query.is_some();
        if !has_retrieval_path {
            return None;
        }

        let inputs = BuiltinRetrievalPlannerInputs {
            has_workspace_root,
            recent_user_query,
            recent_user_budget_items: if recent_window.is_empty() {
                6
            } else {
                config.sliding_window.min(6)
            },
            structured_query: structured_signal_query_seed(session_id, recent_window),
            task_progress_plan: if has_workspace_root {
                task_progress_retrieval_plan(session_id, config)
            } else {
                None
            },
            workflow_task_plan: if has_workspace_root {
                workflow_task_retrieval_plan(session_id, config)
            } else {
                None
            },
            delegate_lineage_plan: if has_workspace_root {
                delegate_lineage_retrieval_plan(session_id, config)
            } else {
                None
            },
            has_structured_session_signals: recent_window_has_structured_session_signals(
                session_id,
                recent_window,
            ),
        };

        Self::from_planner_inputs(inputs)
    }

    fn from_planner_inputs(inputs: BuiltinRetrievalPlannerInputs) -> Option<Self> {
        let BuiltinRetrievalPlannerInputs {
            has_workspace_root,
            recent_user_query,
            recent_user_budget_items,
            structured_query,
            task_progress_plan,
            workflow_task_plan,
            delegate_lineage_plan,
            has_structured_session_signals,
        } = inputs;

        if let Some(query) = recent_user_query {
            return Some(Self::recent_user_query(
                has_workspace_root,
                query,
                recent_user_budget_items,
            ));
        }

        let plan = if let Some(structured_query) = structured_query {
            let seeded = SeededWorkspaceRetrievalPlan::structured_signal_query(structured_query);
            Self::from_seeded_workspace(seeded)
        } else if let Some(task_progress_plan) = task_progress_plan {
            Self::from_seeded_workspace(task_progress_plan)
        } else if let Some(workflow_task_plan) = workflow_task_plan {
            Self::from_seeded_workspace(workflow_task_plan)
        } else if let Some(delegate_lineage_plan) = delegate_lineage_plan {
            Self::from_seeded_workspace(delegate_lineage_plan)
        } else if has_structured_session_signals {
            let seeded =
                SeededWorkspaceRetrievalPlan::workspace_reference_with_structured_signals();
            Self::workspace_scoped(
                seeded.strategy,
                seeded.planning_notes,
                None,
                seeded.budget_items,
                seeded.allowed_kinds,
            )
        } else {
            let seeded = SeededWorkspaceRetrievalPlan::workspace_reference_fallback();
            Self::workspace_scoped(
                seeded.strategy,
                seeded.planning_notes,
                None,
                seeded.budget_items,
                seeded.allowed_kinds,
            )
        };

        Some(plan)
    }

    fn into_request(self, memory_system_id: &str, session_id: &str) -> MemoryRetrievalRequest {
        MemoryRetrievalRequest {
            session_id: session_id.to_owned(),
            memory_system_id: memory_system_id.to_owned(),
            strategy: self.strategy,
            planning_notes: self.planning_notes,
            query: self.query,
            recall_mode: MemoryRecallMode::PromptAssembly,
            scopes: self.scopes,
            budget_items: self.budget_items,
            allowed_kinds: self.allowed_kinds,
        }
    }

    fn into_result(self, memory_system_id: &str, session_id: &str) -> MemoryRetrievalPlanResult {
        let request = self.into_request(memory_system_id, session_id);
        request.into_plan_result()
    }
}

fn recent_window_has_structured_session_signals(
    session_id: &str,
    recent_window: &[WindowTurn],
) -> bool {
    !collect_structured_canonical_records(session_id, recent_window).is_empty()
}

fn structured_signal_query_seed(session_id: &str, recent_window: &[WindowTurn]) -> Option<String> {
    let records = collect_structured_canonical_records(session_id, recent_window);
    structured_signal_query_seed_from_records(records.as_slice())
}

struct SeededWorkspaceRetrievalPlan {
    strategy: MemoryRetrievalStrategy,
    planning_notes: Vec<String>,
    query: String,
    budget_items: usize,
    allowed_kinds: Vec<DerivedMemoryKind>,
}

impl SeededWorkspaceRetrievalPlan {
    fn new(
        strategy: MemoryRetrievalStrategy,
        planning_notes: Vec<String>,
        query: String,
        budget_items: usize,
        allowed_kinds: Vec<DerivedMemoryKind>,
    ) -> Self {
        Self {
            strategy,
            planning_notes,
            query,
            budget_items,
            allowed_kinds,
        }
    }

    fn seeded_query(
        strategy: MemoryRetrievalStrategy,
        seed_note: impl Into<String>,
        query: String,
        policy: WorkspaceRetrievalPolicy,
        extra_notes: Vec<String>,
    ) -> Self {
        let mut planning_notes = vec![seed_note.into()];
        planning_notes.extend(extra_notes);

        Self::new(
            strategy,
            planning_notes,
            query,
            policy.budget_items,
            policy.allowed_kinds,
        )
    }

    fn seeded_no_query(
        strategy: MemoryRetrievalStrategy,
        seed_note: impl Into<String>,
        policy: WorkspaceRetrievalPolicy,
        extra_notes: Vec<String>,
    ) -> Self {
        let mut planning_notes = vec![seed_note.into()];
        planning_notes.extend(extra_notes);

        Self::new(
            strategy,
            planning_notes,
            String::new(),
            policy.budget_items,
            policy.allowed_kinds,
        )
    }

    fn structured_signal_query(query: String) -> Self {
        Self::seeded_query(
            MemoryRetrievalStrategy::StructuredSignalQueryWithWorkspace,
            "structured_signal query seed",
            query,
            WorkspaceRetrievalPolicy::structured_signal_query(),
            vec!["workspace_root present".to_owned()],
        )
    }

    fn task_progress(query: String, policy: WorkspaceRetrievalPolicy) -> Self {
        let budget_items = policy.budget_items;
        Self::seeded_query(
            MemoryRetrievalStrategy::TaskProgressIntentQueryWithWorkspace,
            "task_progress intent_summary seed",
            query,
            policy,
            vec![format!("task_progress budget={budget_items}")],
        )
    }

    fn workflow_task(query: String, policy: WorkspaceRetrievalPolicy) -> Self {
        let budget_items = policy.budget_items;
        Self::seeded_query(
            MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace,
            "workflow task seed",
            query,
            policy,
            vec![format!("workflow task budget={budget_items}")],
        )
    }

    fn delegate_lineage(query: String) -> Self {
        Self::seeded_query(
            if query.contains('\n') {
                MemoryRetrievalStrategy::DelegateLineageQueryWithWorkspace
            } else {
                MemoryRetrievalStrategy::DelegateLabelQueryWithWorkspace
            },
            "delegate label/lineage seed",
            query,
            WorkspaceRetrievalPolicy::delegate_lineage(),
            vec!["workspace_root present".to_owned()],
        )
    }

    fn workspace_reference_with_structured_signals() -> Self {
        Self::seeded_no_query(
            MemoryRetrievalStrategy::WorkspaceReferenceWithStructuredSignals,
            "structured signals influenced kinds",
            WorkspaceRetrievalPolicy::workspace_reference_with_structured_signals(),
            vec!["workspace_root present".to_owned()],
        )
    }

    fn workspace_reference_fallback() -> Self {
        Self::seeded_no_query(
            MemoryRetrievalStrategy::WorkspaceReferenceOnly,
            "workspace reference fallback",
            WorkspaceRetrievalPolicy::workspace_reference_fallback(),
            Vec::new(),
        )
    }
}

#[derive(Clone)]
struct WorkspaceRetrievalPolicy {
    budget_items: usize,
    allowed_kinds: Vec<DerivedMemoryKind>,
}

impl WorkspaceRetrievalPolicy {
    fn new(budget_items: usize, allowed_kinds: Vec<DerivedMemoryKind>) -> Self {
        Self {
            budget_items,
            allowed_kinds,
        }
    }

    fn reference_only(budget_items: usize) -> Self {
        Self::new(budget_items, vec![DerivedMemoryKind::Reference])
    }

    fn reference_procedure_fact(budget_items: usize) -> Self {
        Self::new(
            budget_items,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Fact,
            ],
        )
    }

    fn reference_procedure_overview(budget_items: usize) -> Self {
        Self::new(
            budget_items,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Overview,
            ],
        )
    }

    fn reference_overview_fact(budget_items: usize) -> Self {
        Self::new(
            budget_items,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Overview,
                DerivedMemoryKind::Fact,
            ],
        )
    }

    fn reference_procedure_overview_fact(budget_items: usize) -> Self {
        Self::new(
            budget_items,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Overview,
                DerivedMemoryKind::Fact,
            ],
        )
    }

    fn structured_signal_query() -> Self {
        Self::reference_procedure_fact(1)
    }

    fn workspace_reference_with_structured_signals() -> Self {
        Self::reference_procedure_fact(1)
    }

    fn workspace_reference_fallback() -> Self {
        Self::reference_only(1)
    }

    fn delegate_lineage() -> Self {
        Self::reference_procedure_overview(1)
    }

    fn task_progress(is_pending_like: bool) -> Self {
        if is_pending_like {
            return Self::reference_procedure_overview_fact(2);
        }

        Self::reference_overview_fact(1)
    }
}

fn task_progress_retrieval_plan(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Option<SeededWorkspaceRetrievalPlan> {
    #[cfg(feature = "memory-sqlite")]
    {
        let record = super::sqlite::load_latest_task_progress_record(session_id, config)
            .ok()
            .flatten()?;
        let query = record
            .intent_summary
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())?;
        let is_pending_like = matches!(
            record.status,
            crate::task_progress::TaskProgressStatus::Waiting
                | crate::task_progress::TaskProgressStatus::Blocked
                | crate::task_progress::TaskProgressStatus::Verifying
        );
        Some(SeededWorkspaceRetrievalPlan::task_progress(
            query,
            WorkspaceRetrievalPolicy::task_progress(is_pending_like),
        ))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, config);
        None
    }
}

fn workflow_task_retrieval_plan(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Option<SeededWorkspaceRetrievalPlan> {
    #[cfg(feature = "memory-sqlite")]
    {
        let metadata = super::sqlite::load_session_metadata_hint(session_id, config)
            .ok()
            .flatten()?;
        let query = workflow_task_query_seed_from_metadata(&metadata)?;
        Some(SeededWorkspaceRetrievalPlan::workflow_task(
            query,
            workflow_task_policy_from_metadata(&metadata),
        ))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, config);
        None
    }
}

fn workflow_task_query_seed_from_metadata(
    metadata: &super::sqlite::SessionMetadataHint,
) -> Option<String> {
    let mut lines = Vec::new();

    let task_present = append_unique_query_line(
        &mut lines,
        metadata.workflow_task.as_deref(),
        None::<fn(&str) -> String>,
    );
    if !task_present {
        return None;
    }

    append_unique_query_line(
        &mut lines,
        metadata
            .workflow_phase
            .as_deref()
            .filter(|value| value.trim() != "execute"),
        Some(|value: &str| format!("phase: {}", value.trim())),
    );
    append_unique_query_line(
        &mut lines,
        metadata
            .workflow_operation_kind
            .as_deref()
            .filter(|value| value.trim() != "task"),
        Some(|value: &str| format!("operation_kind: {}", value.trim())),
    );
    append_unique_query_line(
        &mut lines,
        metadata
            .workflow_operation_scope
            .as_deref()
            .filter(|value| value.trim() != "task"),
        Some(|value: &str| format!("operation_scope: {}", value.trim())),
    );

    finalize_query_lines(lines)
}

fn structured_signal_query_seed_from_records(
    records: &[super::CanonicalMemoryRecord],
) -> Option<String> {
    const MAX_SIGNAL_TERMS: usize = 3;

    let mut terms = Vec::new();

    for record in records.iter().rev() {
        let maybe_term = match record.kind {
            CanonicalMemoryKind::ToolDecision => record
                .metadata
                .get("decision")
                .and_then(|value| value.get("tool_name"))
                .and_then(Value::as_str),
            CanonicalMemoryKind::ToolOutcome => record
                .metadata
                .get("outcome")
                .and_then(|value| value.get("tool_name"))
                .and_then(Value::as_str),
            CanonicalMemoryKind::ConversationEvent
            | CanonicalMemoryKind::AcpRuntimeEvent
            | CanonicalMemoryKind::AcpFinalEvent => {
                record.metadata.get("event").and_then(Value::as_str)
            }
            CanonicalMemoryKind::ImportedProfile
            | CanonicalMemoryKind::UserTurn
            | CanonicalMemoryKind::AssistantTurn => None,
        };

        let appended =
            append_unique_query_line(&mut terms, maybe_term, Some(str::to_ascii_lowercase));
        if !appended {
            continue;
        }
        if terms.len() >= MAX_SIGNAL_TERMS {
            break;
        }
    }

    finalize_query_lines_reversed(terms)
}

fn workflow_task_policy_from_metadata(
    metadata: &super::sqlite::SessionMetadataHint,
) -> WorkspaceRetrievalPolicy {
    let workflow_phase = metadata.workflow_phase.as_deref();
    let workflow_operation_kind = metadata.workflow_operation_kind.as_deref();
    let workflow_operation_scope = metadata.workflow_operation_scope.as_deref();
    let is_execute_like = matches!(workflow_phase, Some("execute"))
        || matches!(metadata.state.as_str(), "ready" | "running");
    let is_task_scope = matches!(workflow_operation_kind, Some("task"))
        || matches!(workflow_operation_scope, Some("task"));
    let is_approval_kind = matches!(workflow_operation_kind, Some("approval"));
    let is_session_scope = matches!(workflow_operation_scope, Some("session"));

    if is_task_scope && is_execute_like {
        return WorkspaceRetrievalPolicy::reference_procedure_overview_fact(2);
    }

    if is_approval_kind {
        return WorkspaceRetrievalPolicy::new(
            1,
            vec![
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Fact,
            ],
        );
    }

    if is_session_scope {
        return WorkspaceRetrievalPolicy::reference_overview_fact(1);
    }

    WorkspaceRetrievalPolicy::reference_overview_fact(if is_execute_like { 2 } else { 1 })
}

fn delegate_lineage_query_seed_from_metadata(
    metadata: &super::sqlite::SessionMetadataHint,
) -> Option<String> {
    let is_delegate_child =
        metadata.kind == "delegate_child" || metadata.parent_session_id.is_some();
    if !is_delegate_child {
        return None;
    }

    let mut labels = Vec::new();

    let label_present = append_unique_query_line(
        &mut labels,
        metadata.label.as_deref(),
        None::<fn(&str) -> String>,
    );
    if !label_present {
        return None;
    }

    append_unique_query_line(
        &mut labels,
        metadata.parent_label.as_deref(),
        None::<fn(&str) -> String>,
    );

    if metadata.lineage_depth > 1 {
        append_unique_query_line(
            &mut labels,
            metadata.lineage_root_label.as_deref(),
            None::<fn(&str) -> String>,
        );
    }

    finalize_query_lines(labels)
}

fn append_unique_query_line<F>(
    lines: &mut Vec<String>,
    raw: Option<&str>,
    mapper: Option<F>,
) -> bool
where
    F: Fn(&str) -> String,
{
    let Some(raw) = raw else {
        return false;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    let line = match mapper {
        Some(mapper) => mapper(trimmed),
        None => trimmed.to_owned(),
    };
    if line.trim().is_empty() {
        return false;
    }
    if lines.contains(&line) {
        return false;
    }

    lines.push(line);
    true
}

fn finalize_query_lines(lines: Vec<String>) -> Option<String> {
    if lines.is_empty() {
        return None;
    }

    Some(lines.join("\n"))
}

fn finalize_query_lines_reversed(mut lines: Vec<String>) -> Option<String> {
    if lines.is_empty() {
        return None;
    }

    lines.reverse();
    Some(lines.join("\n"))
}

fn delegate_lineage_retrieval_plan(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Option<SeededWorkspaceRetrievalPlan> {
    #[cfg(feature = "memory-sqlite")]
    {
        let metadata = super::sqlite::load_session_metadata_hint(session_id, config)
            .ok()
            .flatten()?;
        let query = delegate_lineage_query_seed_from_metadata(&metadata)?;
        Some(SeededWorkspaceRetrievalPlan::delegate_lineage(query))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, config);
        None
    }
}

fn run_builtin_retrieve_stage(
    memory_system_id: &str,
    request: &MemoryRetrievalRequest,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Result<Option<Vec<MemoryContextEntry>>, String> {
    let mut entries = durable_recall::load_durable_recall_entries(
        workspace_root,
        config,
        memory_system_id,
        request.recall_mode,
    )?;

    #[cfg(feature = "memory-sqlite")]
    if let Some(query) = request.query.as_deref() {
        let search_limit = cross_session_recall_search_limit(request);
        let hits = super::sqlite::search_canonical_records_for_recall(
            query,
            search_limit,
            Some(request.session_id.as_str()),
            config,
        )?;
        let filtered_hits = filter_cross_session_recall_hits(request, hits);
        let bounded_budget = request.budget_items.max(1);
        let bounded_filtered_hits = filtered_hits
            .into_iter()
            .take(bounded_budget)
            .collect::<Vec<_>>();
        let recall_entries = build_cross_session_recall_entries(
            memory_system_id,
            request.recall_mode,
            bounded_filtered_hits.as_slice(),
        );
        if !recall_entries.is_empty() {
            entries.extend(recall_entries);
        }
    }

    Ok(Some(entries))
}

fn rank_recall_first_entries(entries: Vec<MemoryContextEntry>) -> Vec<MemoryContextEntry> {
    let mut profile_entries = Vec::new();
    let mut derived_entries = Vec::new();
    let mut retrieved_entries = Vec::new();
    let mut summary_entries = Vec::new();
    let mut history_entries = Vec::new();

    for entry in entries {
        match entry.kind {
            MemoryContextKind::Profile => profile_entries.push(entry),
            MemoryContextKind::Derived => derived_entries.push(entry),
            MemoryContextKind::RetrievedMemory => retrieved_entries.push(entry),
            MemoryContextKind::Summary => summary_entries.push(entry),
            MemoryContextKind::Turn => history_entries.push(entry),
        }
    }

    let has_retrieved_entries = !retrieved_entries.is_empty();
    let mut ranked_entries = Vec::new();
    ranked_entries.extend(profile_entries);
    ranked_entries.extend(derived_entries);
    ranked_entries.extend(retrieved_entries);
    if !has_retrieved_entries {
        ranked_entries.extend(summary_entries);
    }
    ranked_entries.extend(history_entries);

    ranked_entries
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
        .with_supported_stage_families([MemoryStageFamily::Compact])
        .with_runtime_fallback_kind(MemorySystemRuntimeFallbackKind::MetadataOnly)
        .with_supported_recall_modes([
            MemoryRecallMode::PromptAssembly,
            MemoryRecallMode::OperatorInspection,
        ])
    }

    fn create_runtime(
        &self,
        config: &MemoryRuntimeConfig,
    ) -> CliResult<Option<Box<dyn MemorySystemRuntime>>> {
        let runtime_config = config.clone();
        let metadata = self.metadata();
        let system: std::sync::Arc<dyn MemorySystem> = std::sync::Arc::new(BuiltinMemorySystem);
        let runtime = BuiltinMemorySystemRuntime::new(runtime_config, metadata, system);
        let boxed_runtime: Box<dyn MemorySystemRuntime> = Box::new(runtime);

        Ok(Some(boxed_runtime))
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
        let ranked_entries = rank_builtin_entries(entries);

        Ok(Some(ranked_entries))
    }
}

#[cfg(feature = "memory-sqlite")]
fn cross_session_recall_search_limit(request: &MemoryRetrievalRequest) -> usize {
    let requested_budget = request.budget_items.max(1);
    let bounded_budget = requested_budget.min(MAX_CROSS_SESSION_RECALL_SEARCH_CANDIDATES);
    let has_scope_filter = !request.scopes.is_empty();
    let has_kind_filter = !request.allowed_kinds.is_empty();
    let has_filter = has_scope_filter || has_kind_filter;

    if has_filter {
        return MAX_CROSS_SESSION_RECALL_SEARCH_CANDIDATES;
    }

    bounded_budget
}

fn filter_cross_session_recall_hits(
    request: &MemoryRetrievalRequest,
    hits: Vec<CanonicalMemorySearchHit>,
) -> Vec<CanonicalMemorySearchHit> {
    hits.into_iter()
        .filter(|hit| request.scopes.is_empty() || request.scopes.contains(&hit.record.scope))
        .filter(|hit| {
            request.allowed_kinds.is_empty()
                || request
                    .allowed_kinds
                    .contains(&derived_memory_kind_for_canonical_kind(hit.record.kind))
        })
        .collect()
}

fn derived_memory_kind_for_canonical_kind(kind: CanonicalMemoryKind) -> DerivedMemoryKind {
    match kind {
        CanonicalMemoryKind::ImportedProfile => DerivedMemoryKind::Profile,
        CanonicalMemoryKind::ToolDecision | CanonicalMemoryKind::ToolOutcome => {
            DerivedMemoryKind::Procedure
        }
        CanonicalMemoryKind::ConversationEvent
        | CanonicalMemoryKind::AcpRuntimeEvent
        | CanonicalMemoryKind::AcpFinalEvent => DerivedMemoryKind::Fact,
        CanonicalMemoryKind::UserTurn | CanonicalMemoryKind::AssistantTurn => {
            DerivedMemoryKind::Episode
        }
    }
}

fn build_cross_session_recall_entries(
    memory_system_id: &str,
    recall_mode: MemoryRecallMode,
    hits: &[CanonicalMemorySearchHit],
) -> Vec<MemoryContextEntry> {
    let mut entries = Vec::new();

    for hit in hits {
        let content = render_cross_session_recall_entry(hit);
        let provenance = build_cross_session_recall_provenance(memory_system_id, recall_mode, hit);
        let entry = MemoryContextEntry {
            kind: MemoryContextKind::RetrievedMemory,
            role: "system".to_owned(),
            content,
            provenance: vec![provenance],
        };
        entries.push(entry);
    }

    entries
}

fn render_cross_session_recall_entry(hit: &CanonicalMemorySearchHit) -> String {
    let turn_label = hit
        .session_turn_index
        .map(|value| format!("turn {value}"))
        .unwrap_or_else(|| "turn ?".to_owned());
    let header = "## Advisory Durable Recall".to_owned();
    let source_line = format!(
        "Cross-session source: {} · {} · {} · {}",
        hit.record.session_id,
        turn_label,
        hit.record.scope.as_str(),
        hit.record.kind.as_str()
    );
    let content = super::orchestrator::truncate_recall_content(hit.record.content.as_str(), 280);
    let recall_line = match hit.record.role.as_deref() {
        Some(role) => format!("{role}: {content}"),
        None => content,
    };

    [header, source_line, recall_line].join("\n\n")
}

fn build_cross_session_recall_provenance(
    memory_system_id: &str,
    recall_mode: MemoryRecallMode,
    hit: &CanonicalMemorySearchHit,
) -> MemoryContextProvenance {
    let source_label = Some(format!(
        "{}:{}:{}",
        hit.record.session_id,
        hit.record.scope.as_str(),
        hit.record.kind.as_str()
    ));

    MemoryContextProvenance::new(
        memory_system_id,
        MemoryProvenanceSourceKind::CanonicalMemoryRecord,
        source_label,
        None,
        Some(hit.record.scope),
        recall_mode,
    )
    .with_trust_level(MemoryTrustLevel::Session)
    .with_authority(MemoryAuthority::Advisory)
    .with_derived_kind(derived_memory_kind_for_canonical_kind(hit.record.kind))
    .with_record_status(MemoryRecordStatus::Active)
}

fn derive_session_overview_entry(
    session_id: &str,
    recent_window: &[WindowTurn],
    memory_system_id: &str,
) -> Option<MemoryContextEntry> {
    let records = collect_structured_canonical_records(session_id, recent_window);
    if records.is_empty() {
        return None;
    }

    let content = render_session_overview_block(records.as_slice());
    let maybe_freshness_ts = recent_window.iter().filter_map(|turn| turn.ts).max();
    let mut provenance = MemoryContextProvenance::new(
        memory_system_id,
        MemoryProvenanceSourceKind::DerivedSessionOverview,
        Some("session_local_overview".to_owned()),
        None,
        Some(MemoryScope::Session),
        MemoryRecallMode::PromptAssembly,
    )
    .with_trust_level(MemoryTrustLevel::Derived)
    .with_authority(MemoryAuthority::Advisory)
    .with_derived_kind(DerivedMemoryKind::Overview)
    .with_record_status(MemoryRecordStatus::Active);

    if let Some(freshness_ts) = maybe_freshness_ts {
        provenance = provenance.with_freshness_ts(freshness_ts);
    }

    let entry = MemoryContextEntry {
        kind: MemoryContextKind::Derived,
        role: "system".to_owned(),
        content,
        provenance: vec![provenance],
    };

    Some(entry)
}

fn collect_structured_canonical_records(
    session_id: &str,
    recent_window: &[WindowTurn],
) -> Vec<super::CanonicalMemoryRecord> {
    let mut records = Vec::new();

    for turn in recent_window {
        let record = super::canonical_memory_record_from_persisted_turn(
            session_id,
            turn.role.as_str(),
            turn.content.as_str(),
        );
        let is_structured_kind = matches!(
            record.kind,
            CanonicalMemoryKind::ToolDecision
                | CanonicalMemoryKind::ToolOutcome
                | CanonicalMemoryKind::ConversationEvent
                | CanonicalMemoryKind::AcpRuntimeEvent
                | CanonicalMemoryKind::AcpFinalEvent
        );
        if !is_structured_kind {
            continue;
        }

        records.push(record);
    }

    records
}

fn render_session_overview_block(records: &[super::CanonicalMemoryRecord]) -> String {
    let mut sections = Vec::new();
    let mut lines = Vec::new();
    let tool_decision_count = count_canonical_kind(records, CanonicalMemoryKind::ToolDecision);
    let tool_outcome_count = count_canonical_kind(records, CanonicalMemoryKind::ToolOutcome);
    let conversation_event_count =
        count_canonical_kind(records, CanonicalMemoryKind::ConversationEvent);
    let acp_runtime_event_count =
        count_canonical_kind(records, CanonicalMemoryKind::AcpRuntimeEvent);
    let acp_final_event_count = count_canonical_kind(records, CanonicalMemoryKind::AcpFinalEvent);
    let record_kinds = collect_record_kind_names(records);

    sections.push("## Session Local Overview".to_owned());
    sections.push(
        "Advisory session-local overview derived from persisted internal records. It preserves runtime continuity without replacing runtime-self guidance, resolved runtime identity, or the session profile."
            .to_owned(),
    );

    if tool_decision_count > 0 {
        lines.push(format!("- tool_decisions: {tool_decision_count}"));
    }
    if tool_outcome_count > 0 {
        lines.push(format!("- tool_outcomes: {tool_outcome_count}"));
    }
    if conversation_event_count > 0 {
        lines.push(format!("- conversation_events: {conversation_event_count}"));
    }
    if acp_runtime_event_count > 0 {
        lines.push(format!("- acp_runtime_events: {acp_runtime_event_count}"));
    }
    if acp_final_event_count > 0 {
        lines.push(format!("- acp_final_events: {acp_final_event_count}"));
    }
    if !record_kinds.is_empty() {
        let record_kind_summary = record_kinds.join(", ");
        lines.push(format!("- observed_record_kinds: {record_kind_summary}"));
    }

    sections.push(lines.join("\n"));

    sections.join("\n\n")
}

fn count_canonical_kind(
    records: &[super::CanonicalMemoryRecord],
    kind: CanonicalMemoryKind,
) -> usize {
    records.iter().filter(|record| record.kind == kind).count()
}

fn collect_record_kind_names(records: &[super::CanonicalMemoryRecord]) -> Vec<String> {
    let mut names = BTreeSet::new();

    for record in records {
        names.insert(record.kind.as_str().to_owned());
    }

    names.into_iter().collect()
}

fn rank_builtin_entries(entries: Vec<MemoryContextEntry>) -> Vec<MemoryContextEntry> {
    let mut advisory_entries = Vec::new();
    let mut turn_entries = Vec::new();

    for entry in entries {
        let is_turn = entry.kind == MemoryContextKind::Turn;
        if is_turn {
            turn_entries.push(entry);
            continue;
        }

        if !memory_entry_is_active(&entry) {
            continue;
        }

        advisory_entries.push(entry);
    }

    advisory_entries.sort_by(rank_builtin_entry_cmp);

    let mut ranked_entries = advisory_entries;
    ranked_entries.extend(turn_entries);

    ranked_entries
}

fn memory_entry_is_active(entry: &MemoryContextEntry) -> bool {
    let maybe_status = entry
        .provenance
        .first()
        .and_then(|provenance| provenance.record_status);

    match maybe_status {
        Some(status) => status.is_active(),
        None => true,
    }
}

fn rank_builtin_entry_cmp(
    left: &MemoryContextEntry,
    right: &MemoryContextEntry,
) -> std::cmp::Ordering {
    let left_kind_priority = memory_entry_kind_priority(left.kind);
    let right_kind_priority = memory_entry_kind_priority(right.kind);
    let kind_order = left_kind_priority.cmp(&right_kind_priority);
    if kind_order != std::cmp::Ordering::Equal {
        return kind_order;
    }

    let left_trust_priority = memory_entry_trust_priority(left);
    let right_trust_priority = memory_entry_trust_priority(right);
    let trust_order = left_trust_priority.cmp(&right_trust_priority);
    if trust_order != std::cmp::Ordering::Equal {
        return trust_order;
    }

    let left_freshness = memory_entry_freshness(left);
    let right_freshness = memory_entry_freshness(right);
    let freshness_order = right_freshness.cmp(&left_freshness);
    if freshness_order != std::cmp::Ordering::Equal {
        return freshness_order;
    }

    let left_label = memory_entry_label(left);
    let right_label = memory_entry_label(right);
    left_label.cmp(right_label)
}

fn memory_entry_kind_priority(kind: MemoryContextKind) -> u8 {
    match kind {
        MemoryContextKind::Profile => 0,
        MemoryContextKind::Summary => 1,
        MemoryContextKind::Derived => 2,
        MemoryContextKind::RetrievedMemory => 3,
        MemoryContextKind::Turn => 4,
    }
}

fn memory_entry_trust_priority(entry: &MemoryContextEntry) -> u8 {
    let maybe_trust_level = entry
        .provenance
        .first()
        .and_then(|provenance| provenance.trust_level);
    let trust_level = maybe_trust_level.unwrap_or(MemoryTrustLevel::Derived);

    match trust_level {
        MemoryTrustLevel::WorkspaceCurated => 0,
        MemoryTrustLevel::Derived => 1,
        MemoryTrustLevel::WorkspaceLog => 2,
        MemoryTrustLevel::Session => 3,
    }
}

fn memory_entry_freshness(entry: &MemoryContextEntry) -> i64 {
    let maybe_freshness = entry
        .provenance
        .first()
        .and_then(|provenance| provenance.freshness_ts);

    maybe_freshness.unwrap_or_default()
}

fn memory_entry_label(entry: &MemoryContextEntry) -> &str {
    entry
        .provenance
        .first()
        .and_then(|provenance| provenance.source_label.as_deref())
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    const TEST_SESSION_ID: &str = "session-123";
    const TEST_SECONDARY_SESSION_ID: &str = "session-1";
    const RELEASE_FREEZE_TIMING: &str = "release freeze timing";
    const DIAGNOSE_RELEASE_FREEZE_TEXT: &str = "diagnose release freeze";
    const REGISTRY_REQUEST_ONLY_ID: &str = "result-wrapping-registry";
    const PLAN_RESULT_ONLY_ID: &str = "plan-result-only-registry";
    const REGISTRY_REQUEST_NOTE: &str = "registry request";
    const PLAN_RESULT_ONLY_NOTE: &str = "plan result only";
    const ROOT_SESSION_ID: &str = "root-session";
    const ROOT_SESSION_LABEL: &str = "Root Session";
    const DELEGATE_CHILD_KIND: &str = "delegate_child";
    const DELEGATE_CHILD_LABEL: &str = "Release Child";
    const WORKFLOW_TASK_TEXT: &str = "investigate release freeze";
    const WORKFLOW_PHASE_FAILED: &str = "failed";
    const WORKFLOW_PHASE_COMPLETE: &str = "complete";
    const WORKFLOW_OPERATION_KIND_APPROVAL: &str = "approval";
    const WORKFLOW_OPERATION_SCOPE_SESSION: &str = "session";
    const DELEGATE_STARTED_EVENT: &str = "delegate_started";
    const DELEGATE_COMPLETED_EVENT: &str = "delegate_completed";
    const SHELL_EXEC_TOOL_NAME: &str = "shell.exec";

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
            .with_runtime_fallback_kind(MemorySystemRuntimeFallbackKind::SystemBacked)
            .with_supported_pre_assembly_stage_families([MemoryStageFamily::Retrieve])
        }
    }

    struct ResultWrappingRegistryMemorySystem;

    struct PlanResultOnlyRegistryMemorySystem;

    impl MemorySystem for ResultWrappingRegistryMemorySystem {
        fn id(&self) -> &'static str {
            REGISTRY_REQUEST_ONLY_ID
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                REGISTRY_REQUEST_ONLY_ID,
                [MemorySystemCapability::PromptHydration],
                "Registry system with direct retrieval request",
            )
        }

        fn build_retrieval_request(
            &self,
            session_id: &str,
            _workspace_root: Option<&Path>,
            _config: &MemoryRuntimeConfig,
            _recent_window: &[WindowTurn],
        ) -> Option<MemoryRetrievalRequest> {
            Some(MemoryRetrievalRequest {
                session_id: session_id.to_owned(),
                memory_system_id: self.id().to_owned(),
                strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
                planning_notes: vec![REGISTRY_REQUEST_NOTE.to_owned()],
                query: None,
                recall_mode: MemoryRecallMode::PromptAssembly,
                scopes: vec![MemoryScope::Workspace],
                budget_items: 1,
                allowed_kinds: vec![DerivedMemoryKind::Reference],
            })
        }
    }

    impl MemorySystem for PlanResultOnlyRegistryMemorySystem {
        fn id(&self) -> &'static str {
            PLAN_RESULT_ONLY_ID
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                PLAN_RESULT_ONLY_ID,
                [MemorySystemCapability::PromptHydration],
                "Registry system with direct retrieval plan result",
            )
        }

        fn build_retrieval_plan_result(
            &self,
            session_id: &str,
            _workspace_root: Option<&Path>,
            _config: &MemoryRuntimeConfig,
            _recent_window: &[WindowTurn],
        ) -> Option<MemoryRetrievalPlanResult> {
            Some(
                MemoryRetrievalRequest {
                    session_id: session_id.to_owned(),
                    memory_system_id: self.id().to_owned(),
                    strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
                    planning_notes: vec![PLAN_RESULT_ONLY_NOTE.to_owned()],
                    query: None,
                    recall_mode: MemoryRecallMode::PromptAssembly,
                    scopes: vec![MemoryScope::Workspace],
                    budget_items: 1,
                    allowed_kinds: vec![DerivedMemoryKind::Reference],
                }
                .into_plan_result(),
            )
        }
    }

    fn release_freeze_recent_window() -> Vec<WindowTurn> {
        vec![WindowTurn {
            role: "user".to_owned(),
            content: RELEASE_FREEZE_TIMING.to_owned(),
            ts: None,
        }]
    }

    fn release_freeze_test_config() -> MemoryRuntimeConfig {
        MemoryRuntimeConfig::default()
    }

    fn default_test_config() -> MemoryRuntimeConfig {
        MemoryRuntimeConfig::default()
    }

    fn workspace_recall_test_config() -> MemoryRuntimeConfig {
        MemoryRuntimeConfig {
            sliding_window: 6,
            ..MemoryRuntimeConfig::default()
        }
    }

    fn assert_request_adapter_matches_plan_result_for_workspace_recall(system: &dyn MemorySystem) {
        let config = workspace_recall_test_config();
        assert_request_adapter_matches_plan_result(
            system,
            WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
            &config,
            &[],
        );
    }

    fn assert_request_helper_matches_request_adapter_for_workspace_recall(
        system: &dyn MemorySystem,
    ) {
        let config = workspace_recall_test_config();
        assert_request_helper_matches_request_adapter(system, &config, &[]);
    }

    fn assert_plan_result_helper_matches_default_for_workspace_recall(system: &dyn MemorySystem) {
        let config = workspace_recall_test_config();
        assert_plan_result_helper_matches_default(system, &config, &[]);
    }

    fn assert_boxed_request_helper_matches_request_adapter_for_workspace_recall(
        boxed: Box<dyn MemorySystem>,
    ) {
        let config = workspace_recall_test_config();
        assert_boxed_request_helper_matches_request_adapter(boxed, &config, &[]);
    }

    fn assert_boxed_request_adapter_matches_plan_result_for_workspace_recall(
        boxed: Box<dyn MemorySystem>,
    ) {
        let config = workspace_recall_test_config();
        assert_boxed_request_adapter_matches_plan_result(
            boxed,
            WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
            &config,
            &[],
        );
    }

    fn assert_boxed_plan_result_helper_matches_default_for_workspace_recall(
        boxed: Box<dyn MemorySystem>,
    ) {
        let config = workspace_recall_test_config();
        assert_boxed_plan_result_helper_matches_default(boxed, &config, &[]);
    }

    fn assert_boxed_workspace_recall_request_budget_items(
        boxed: Box<dyn MemorySystem>,
        expected_budget_items: usize,
    ) {
        assert_workspace_recall_request_budget_items(&*boxed, expected_budget_items);
    }

    fn assert_workspace_recall_request_budget_items(
        system: &dyn MemorySystem,
        expected_budget_items: usize,
    ) {
        let config = workspace_recall_test_config();
        let request = system
            .build_retrieval_request(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
            .expect("workspace recall retrieval request");
        assert_eq!(request.budget_items, expected_budget_items);
    }

    fn test_workspace_root() -> &'static Path {
        Path::new("/tmp/workspace")
    }

    fn boxed_request_only_registry_system() -> Box<dyn MemorySystem> {
        Box::new(ResultWrappingRegistryMemorySystem)
    }

    fn boxed_plan_result_only_registry_system() -> Box<dyn MemorySystem> {
        Box::new(PlanResultOnlyRegistryMemorySystem)
    }

    fn boxed_builtin_system() -> Box<dyn MemorySystem> {
        Box::new(BuiltinMemorySystem)
    }

    fn boxed_recall_first_system() -> Box<dyn MemorySystem> {
        Box::new(RecallFirstMemorySystem)
    }

    fn boxed_workspace_recall_system() -> Box<dyn MemorySystem> {
        Box::new(WorkspaceRecallMemorySystem)
    }

    fn delegate_child_metadata_hint(
        state: &str,
        workflow_task: Option<&str>,
        workflow_phase: Option<&str>,
        workflow_operation_kind: Option<&str>,
        workflow_operation_scope: Option<&str>,
    ) -> crate::memory::sqlite::SessionMetadataHint {
        crate::memory::sqlite::SessionMetadataHint {
            kind: DELEGATE_CHILD_KIND.to_owned(),
            state: state.to_owned(),
            parent_session_id: Some(ROOT_SESSION_ID.to_owned()),
            label: Some(DELEGATE_CHILD_LABEL.to_owned()),
            parent_label: Some(ROOT_SESSION_LABEL.to_owned()),
            lineage_root_session_id: Some(ROOT_SESSION_ID.to_owned()),
            lineage_root_label: Some(ROOT_SESSION_LABEL.to_owned()),
            lineage_depth: 1,
            workflow_task: workflow_task.map(str::to_owned),
            workflow_phase: workflow_phase.map(str::to_owned),
            workflow_operation_kind: workflow_operation_kind.map(str::to_owned),
            workflow_operation_scope: workflow_operation_scope.map(str::to_owned),
        }
    }

    fn expected_delegate_lineage_query() -> String {
        format!("{DELEGATE_CHILD_LABEL}\n{ROOT_SESSION_LABEL}")
    }

    fn expected_workflow_task_query(
        phase: &str,
        operation_kind: Option<&str>,
        operation_scope: Option<&str>,
    ) -> String {
        let mut parts = vec![WORKFLOW_TASK_TEXT.to_owned(), format!("phase: {phase}")];
        if let Some(operation_kind) = operation_kind {
            parts.push(format!("operation_kind: {operation_kind}"));
        }
        if let Some(operation_scope) = operation_scope {
            parts.push(format!("operation_scope: {operation_scope}"));
        }
        parts.join("\n")
    }

    fn expected_structured_signal_query() -> String {
        format!("{DELEGATE_STARTED_EVENT}\n{SHELL_EXEC_TOOL_NAME}")
    }

    #[cfg(feature = "memory-sqlite")]
    fn sqlite_test_workspace() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        MemoryRuntimeConfig,
        Connection,
    ) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp_dir.path().to_path_buf();
        let db_path = workspace_root.join("memory.sqlite3");
        let config = MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            ..MemoryRuntimeConfig::default()
        };

        crate::memory::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("ensure memory db ready");
        let conn = Connection::open(&db_path).expect("open sqlite db");

        (temp_dir, workspace_root, config, conn)
    }

    #[cfg(feature = "memory-sqlite")]
    fn insert_root_session(conn: &Connection) {
        conn.execute(
            "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
            VALUES (?1, 'root', NULL, ?2, 'ready', 1, 1, NULL)",
            rusqlite::params![ROOT_SESSION_ID, ROOT_SESSION_LABEL],
        )
        .expect("insert root session");
    }

    #[cfg(feature = "memory-sqlite")]
    fn insert_delegate_child_session(conn: &Connection, state: &str) {
        conn.execute(
            "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, NULL)",
            rusqlite::params![
                TEST_SESSION_ID,
                DELEGATE_CHILD_KIND,
                ROOT_SESSION_ID,
                DELEGATE_CHILD_LABEL,
                state,
            ],
        )
        .expect("insert delegate child session");
    }

    #[cfg(feature = "memory-sqlite")]
    fn insert_delegate_task_event(conn: &Connection, event_kind: &str, task: &str) {
        conn.execute(
            "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, search_text, ts)
             VALUES (?1, ?2, ?3, ?4, '', 1)",
            rusqlite::params![
                TEST_SESSION_ID,
                event_kind,
                ROOT_SESSION_ID,
                serde_json::to_string(&json!({ "task": task }))
                    .expect("encode delegate task payload"),
            ],
        )
        .expect("insert delegate task event");
    }

    fn assert_request_adapter_matches_plan_result(
        system: &dyn MemorySystem,
        expected_system_id: &str,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) {
        let request = system
            .build_retrieval_request(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                recent_window,
            )
            .expect("retrieval request");
        let result = system
            .build_retrieval_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                recent_window,
            )
            .expect("retrieval plan");

        assert_eq!(&request, result.request());
        assert_eq!(request.memory_system_id, expected_system_id);
    }

    fn assert_boxed_request_adapter_matches_plan_result(
        boxed: Box<dyn MemorySystem>,
        expected_system_id: &str,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) {
        assert_request_adapter_matches_plan_result(
            &*boxed,
            expected_system_id,
            config,
            recent_window,
        );
    }

    fn assert_request_helper_matches_request_adapter(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) {
        let request = system
            .build_retrieval_request(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                recent_window,
            )
            .expect("retrieval request");
        let helper_request = system
            .build_retrieval_request_via_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                recent_window,
            )
            .expect("retrieval request via plan result");

        assert_eq!(helper_request, request);
    }

    fn assert_plan_result_helper_matches_default(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) {
        let default_result = system
            .build_retrieval_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                recent_window,
            )
            .expect("default retrieval plan result");
        let helper_result = system
            .build_retrieval_plan_result_via_request(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                recent_window,
            )
            .expect("helper retrieval plan result");

        assert_eq!(helper_result, default_result);
    }

    fn assert_boxed_request_helper_matches_request_adapter(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) {
        assert_request_helper_matches_request_adapter(&*boxed, config, recent_window);
    }

    fn assert_boxed_plan_result_helper_matches_default(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) {
        assert_plan_result_helper_matches_default(&*boxed, config, recent_window);
    }

    fn assert_boxed_request_helper_matches_request_adapter_for_release_freeze(
        boxed: Box<dyn MemorySystem>,
    ) {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        assert_boxed_request_helper_matches_request_adapter(
            boxed,
            &config,
            recent_window.as_slice(),
        );
    }

    fn assert_request_adapter_matches_plan_result_for_release_freeze(
        system: &dyn MemorySystem,
        expected_system_id: &str,
    ) {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        assert_request_adapter_matches_plan_result(
            system,
            expected_system_id,
            &config,
            recent_window.as_slice(),
        );
    }

    fn assert_request_helper_matches_request_adapter_for_release_freeze(system: &dyn MemorySystem) {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        assert_request_helper_matches_request_adapter(system, &config, recent_window.as_slice());
    }

    fn assert_plan_result_helper_matches_default_for_release_freeze(system: &dyn MemorySystem) {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        assert_plan_result_helper_matches_default(system, &config, recent_window.as_slice());
    }

    fn assert_release_freeze_request_adapter_matches_plan_result(
        system: &dyn MemorySystem,
        expected_system_id: &str,
    ) {
        assert_request_adapter_matches_plan_result_for_release_freeze(system, expected_system_id);
    }

    fn assert_release_freeze_request_helper_matches_request_adapter(system: &dyn MemorySystem) {
        assert_request_helper_matches_request_adapter_for_release_freeze(system);
    }

    fn assert_release_freeze_plan_result_helper_matches_default(system: &dyn MemorySystem) {
        assert_plan_result_helper_matches_default_for_release_freeze(system);
    }

    fn assert_boxed_request_adapter_matches_plan_result_for_release_freeze(
        boxed: Box<dyn MemorySystem>,
        expected_system_id: &str,
    ) {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        assert_boxed_request_adapter_matches_plan_result(
            boxed,
            expected_system_id,
            &config,
            recent_window.as_slice(),
        );
    }

    fn assert_boxed_plan_result_helper_matches_default_for_release_freeze(
        boxed: Box<dyn MemorySystem>,
    ) {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        assert_boxed_plan_result_helper_matches_default(boxed, &config, recent_window.as_slice());
    }

    fn assert_boxed_request_only_default_plan_result_matches_explicit_helper(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
    ) {
        assert_request_only_default_plan_result_matches_explicit_helper(&*boxed, config);
    }

    fn assert_boxed_request_only_plan_result_adapter_shape(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
    ) {
        assert_request_only_plan_result_adapter_shape(&*boxed, config);
    }

    fn assert_boxed_plan_result_only_helper_request_matches_default_plan_result_request(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
    ) {
        assert_plan_result_only_helper_request_matches_default_plan_result_request(&*boxed, config);
    }

    fn assert_boxed_plan_result_only_request_adapter_shape(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
    ) {
        assert_plan_result_only_request_adapter_shape(&*boxed, config);
    }

    fn assert_boxed_plan_result_only_request_gap(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
    ) {
        assert_plan_result_only_request_gap(&*boxed, config);
    }

    fn assert_retrieval_plan_shape(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
        expected_system_id: &str,
        expected_planning_notes: &[&str],
    ) {
        let result = system
            .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
            .expect("retrieval plan result");

        let expected_planning_notes = expected_planning_notes
            .iter()
            .map(|note| (*note).to_owned())
            .collect::<Vec<_>>();

        assert_plan_result_identity(&result, expected_system_id);
        assert_eq!(
            result.planner_snapshot.planning_notes,
            expected_planning_notes
        );
    }

    fn assert_plan_result_identity(result: &MemoryRetrievalPlanResult, expected_system_id: &str) {
        assert_eq!(result.request.memory_system_id, expected_system_id);
        assert_eq!(result.planner_snapshot.memory_system_id, expected_system_id);
    }

    fn assert_plan_result_snapshot_matches_request(
        result: &MemoryRetrievalPlanResult,
        expected_system_id: &str,
    ) {
        assert_plan_result_identity(result, expected_system_id);
        assert_eq!(result.planner_snapshot, result.request.planner_snapshot());
    }

    fn assert_plan_result_snapshot_matches_request_for_release_freeze(
        system: &dyn MemorySystem,
        expected_system_id: &str,
    ) -> MemoryRetrievalPlanResult {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        let result = system
            .build_retrieval_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                &config,
                recent_window.as_slice(),
            )
            .expect("retrieval plan result");
        assert_plan_result_snapshot_matches_request(&result, expected_system_id);
        result
    }

    fn assert_plan_result_snapshot_matches_request_for_workspace_recall(
        system: &dyn MemorySystem,
    ) -> MemoryRetrievalPlanResult {
        let config = workspace_recall_test_config();
        let result = system
            .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
            .expect("retrieval plan result");
        assert_plan_result_snapshot_matches_request(&result, WORKSPACE_RECALL_MEMORY_SYSTEM_ID);
        result
    }

    fn assert_boxed_plan_result_snapshot_matches_request(
        boxed: Box<dyn MemorySystem>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
        expected_system_id: &str,
    ) -> MemoryRetrievalPlanResult {
        let result = boxed
            .build_retrieval_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                recent_window,
            )
            .expect("boxed retrieval plan result");
        assert_plan_result_snapshot_matches_request(&result, expected_system_id);
        result
    }

    fn assert_boxed_plan_result_snapshot_matches_request_for_release_freeze(
        boxed: Box<dyn MemorySystem>,
        expected_system_id: &str,
    ) -> MemoryRetrievalPlanResult {
        let recent_window = release_freeze_recent_window();
        let config = release_freeze_test_config();
        assert_boxed_plan_result_snapshot_matches_request(
            boxed,
            &config,
            recent_window.as_slice(),
            expected_system_id,
        )
    }

    fn assert_boxed_plan_result_snapshot_matches_request_for_workspace_recall(
        boxed: Box<dyn MemorySystem>,
    ) -> MemoryRetrievalPlanResult {
        let config = workspace_recall_test_config();
        assert_boxed_plan_result_snapshot_matches_request(
            boxed,
            &config,
            &[],
            WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
        )
    }

    fn assert_workspace_recall_plan_result_snapshot_matches_request_and_budget(
        result: &MemoryRetrievalPlanResult,
    ) {
        assert_plan_result_snapshot_matches_request(result, WORKSPACE_RECALL_MEMORY_SYSTEM_ID);
        assert_eq!(result.request.budget_items, 4);
    }

    fn assert_retrieval_request_shape(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
        expected_system_id: &str,
        expected_planning_notes: &[&str],
    ) {
        let request = system
            .build_retrieval_request_via_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                &[],
            )
            .expect("retrieval request via plan result");

        let expected_planning_notes = expected_planning_notes
            .iter()
            .map(|note| (*note).to_owned())
            .collect::<Vec<_>>();

        assert_eq!(request.memory_system_id, expected_system_id);
        assert_eq!(request.planning_notes, expected_planning_notes);
    }

    fn assert_request_only_default_plan_result_matches_explicit_helper(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
    ) {
        let default_result = system
            .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
            .expect("default retrieval plan result");
        let helper_result = system
            .build_retrieval_plan_result_via_request(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                &[],
            )
            .expect("helper retrieval plan result");

        assert_eq!(helper_result, default_result);
    }

    fn assert_request_only_plan_result_adapter_shape(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
    ) {
        let result = system
            .build_retrieval_plan_result_via_request(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                &[],
            )
            .expect("retrieval plan result via request");

        assert_retrieval_plan_shape(
            system,
            config,
            REGISTRY_REQUEST_ONLY_ID,
            &[REGISTRY_REQUEST_NOTE],
        );
        assert_eq!(
            result.planner_snapshot.planning_notes,
            vec![REGISTRY_REQUEST_NOTE.to_owned()]
        );
    }

    fn assert_plan_result_only_helper_request_matches_default_plan_result_request(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
    ) {
        let helper_request = system
            .build_retrieval_request_via_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                &[],
            )
            .expect("retrieval request via plan result");
        let default_result = system
            .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
            .expect("default retrieval plan result");

        assert_eq!(helper_request, default_result.request);
    }

    fn assert_plan_result_only_request_adapter_shape(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
    ) {
        let request = system
            .build_retrieval_request_via_plan_result(
                TEST_SESSION_ID,
                Some(test_workspace_root()),
                config,
                &[],
            )
            .expect("retrieval request via plan result");

        assert_retrieval_request_shape(
            system,
            config,
            PLAN_RESULT_ONLY_ID,
            &[PLAN_RESULT_ONLY_NOTE],
        );
        assert_eq!(
            request.planning_notes,
            vec![PLAN_RESULT_ONLY_NOTE.to_owned()]
        );
    }

    fn assert_plan_result_only_request_gap(
        system: &dyn MemorySystem,
        config: &MemoryRuntimeConfig,
    ) {
        let request = system.build_retrieval_request(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            config,
            &[],
        );
        let result = system
            .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), config, &[])
            .expect("retrieval plan result");

        assert_eq!(request, None);
        assert_plan_result_identity(&result, PLAN_RESULT_ONLY_ID);
        assert_eq!(
            result.planner_snapshot.planning_notes,
            vec![PLAN_RESULT_ONLY_NOTE.to_owned()]
        );
    }

    #[test]
    fn builtin_memory_system_metadata_is_stable() {
        let metadata = BuiltinMemorySystem.metadata();
        assert_eq!(metadata.id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(metadata.api_version, MEMORY_SYSTEM_API_VERSION);
        assert_eq!(
            metadata.runtime_fallback_kind,
            MemorySystemRuntimeFallbackKind::MetadataOnly
        );
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
            vec![
                MemoryRecallMode::PromptAssembly,
                MemoryRecallMode::OperatorInspection
            ]
        );
    }

    #[test]
    fn build_retrieval_plan_result_wraps_non_builtin_request_into_snapshot() {
        let config = default_test_config();

        let result = ResultWrappingRegistryMemorySystem
            .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
            .expect("retrieval plan result");

        assert_retrieval_plan_shape(
            &ResultWrappingRegistryMemorySystem,
            &config,
            REGISTRY_REQUEST_ONLY_ID,
            &[REGISTRY_REQUEST_NOTE],
        );
        assert_eq!(
            result.planner_snapshot.strategy,
            MemoryRetrievalStrategy::WorkspaceReferenceOnly
        );
        assert_eq!(result.planner_snapshot.budget_items, 1);
        assert!(!result.planner_snapshot.query_present);
    }

    #[test]
    fn build_retrieval_plan_result_via_request_adapts_request_only_system() {
        let config = default_test_config();
        assert_request_only_plan_result_adapter_shape(&ResultWrappingRegistryMemorySystem, &config);
    }

    #[test]
    fn request_only_system_default_plan_result_matches_explicit_helper() {
        let config = default_test_config();
        assert_request_only_default_plan_result_matches_explicit_helper(
            &ResultWrappingRegistryMemorySystem,
            &config,
        );
    }

    #[test]
    fn build_retrieval_request_default_does_not_backfill_from_plan_result() {
        let config = default_test_config();
        assert_plan_result_only_request_gap(&PlanResultOnlyRegistryMemorySystem, &config);
    }

    #[test]
    fn plan_result_only_system_helper_request_matches_default_plan_result_request() {
        let config = default_test_config();
        assert_plan_result_only_helper_request_matches_default_plan_result_request(
            &PlanResultOnlyRegistryMemorySystem,
            &config,
        );
    }

    #[test]
    fn build_retrieval_request_via_plan_result_adapts_plan_result_only_system() {
        let config = default_test_config();
        assert_plan_result_only_request_adapter_shape(&PlanResultOnlyRegistryMemorySystem, &config);
    }

    #[test]
    fn boxed_memory_system_preserves_request_to_plan_result_compatibility() {
        let config = default_test_config();
        let boxed = boxed_request_only_registry_system();

        let result = boxed
            .build_retrieval_plan_result(TEST_SESSION_ID, Some(test_workspace_root()), &config, &[])
            .expect("retrieval plan result");

        assert_retrieval_plan_shape(
            &*boxed,
            &config,
            REGISTRY_REQUEST_ONLY_ID,
            &[REGISTRY_REQUEST_NOTE],
        );
        assert_eq!(
            result.planner_snapshot.planning_notes,
            vec![REGISTRY_REQUEST_NOTE.to_owned()]
        );
    }

    #[test]
    fn boxed_memory_system_preserves_request_only_plan_result_adapter() {
        let config = default_test_config();
        assert_boxed_request_only_plan_result_adapter_shape(
            boxed_request_only_registry_system(),
            &config,
        );
    }

    #[test]
    fn boxed_request_only_system_default_plan_result_matches_explicit_helper() {
        let config = default_test_config();
        assert_boxed_request_only_default_plan_result_matches_explicit_helper(
            boxed_request_only_registry_system(),
            &config,
        );
    }

    #[test]
    fn boxed_memory_system_preserves_plan_result_only_request_adapter() {
        let config = default_test_config();
        assert_boxed_plan_result_only_request_adapter_shape(
            boxed_plan_result_only_registry_system(),
            &config,
        );
    }

    #[test]
    fn boxed_plan_result_only_system_helper_request_matches_default_plan_result_request() {
        let config = default_test_config();
        assert_boxed_plan_result_only_helper_request_matches_default_plan_result_request(
            boxed_plan_result_only_registry_system(),
            &config,
        );
    }

    #[test]
    fn boxed_memory_system_preserves_plan_result_only_request_gap() {
        let config = default_test_config();
        assert_boxed_plan_result_only_request_gap(
            boxed_plan_result_only_registry_system(),
            &config,
        );
    }

    #[test]
    fn memory_system_field_exposes_builtin_pre_assembly_stage_families() {
        let metadata = BuiltinMemorySystem.metadata();
        assert_eq!(
            metadata.supported_pre_assembly_stage_families,
            builtin_pre_assembly_stage_families()
        );
        assert_eq!(
            metadata.supported_stage_families,
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::Rank,
                MemoryStageFamily::Compact,
            ]
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
        assert_eq!(
            custom.supported_stage_families,
            vec![MemoryStageFamily::Retrieve]
        );
        assert_eq!(
            custom.runtime_fallback_kind,
            MemorySystemRuntimeFallbackKind::SystemBacked
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
        assert!(
            metadata
                .iter()
                .any(|entry| entry.id == RECALL_FIRST_MEMORY_SYSTEM_ID)
        );
    }

    #[test]
    fn recall_first_memory_system_metadata_is_stable() {
        let metadata = RecallFirstMemorySystem.metadata();

        assert_eq!(metadata.id, RECALL_FIRST_MEMORY_SYSTEM_ID);
        assert_eq!(metadata.api_version, MEMORY_SYSTEM_API_VERSION);
        assert_eq!(
            metadata.runtime_fallback_kind,
            MemorySystemRuntimeFallbackKind::SystemBacked
        );
        assert_eq!(
            metadata.capability_names(),
            vec!["prompt_hydration", "retrieval_provenance"]
        );
        assert_eq!(
            metadata.supported_pre_assembly_stage_families,
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::Rank,
            ]
        );
        assert_eq!(
            metadata.supported_stage_families,
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::Rank,
            ]
        );
        assert_eq!(
            metadata.supported_recall_modes,
            vec![MemoryRecallMode::PromptAssembly]
        );
    }

    #[test]
    fn recall_first_retrieval_plan_result_preserves_selected_system_id() {
        let result = assert_plan_result_snapshot_matches_request_for_release_freeze(
            &RecallFirstMemorySystem,
            RECALL_FIRST_MEMORY_SYSTEM_ID,
        );
        assert_eq!(result.planner_snapshot.strategy, result.request.strategy);
    }

    #[test]
    fn recall_first_plan_result_helper_matches_default_plan_result() {
        assert_release_freeze_plan_result_helper_matches_default(&RecallFirstMemorySystem);
    }

    #[test]
    fn recall_first_request_adapter_matches_plan_result_request() {
        assert_release_freeze_request_adapter_matches_plan_result(
            &RecallFirstMemorySystem,
            RECALL_FIRST_MEMORY_SYSTEM_ID,
        );
    }

    #[test]
    fn recall_first_request_helper_matches_request_adapter() {
        assert_release_freeze_request_helper_matches_request_adapter(&RecallFirstMemorySystem);
    }

    #[test]
    fn boxed_recall_first_request_helper_matches_request_adapter() {
        assert_boxed_request_helper_matches_request_adapter_for_release_freeze(
            boxed_recall_first_system(),
        );
    }

    #[test]
    fn boxed_recall_first_request_adapter_matches_plan_result_request() {
        assert_boxed_request_adapter_matches_plan_result_for_release_freeze(
            boxed_recall_first_system(),
            RECALL_FIRST_MEMORY_SYSTEM_ID,
        );
    }

    #[test]
    fn boxed_recall_first_plan_result_helper_matches_default_plan_result() {
        assert_boxed_plan_result_helper_matches_default_for_release_freeze(
            boxed_recall_first_system(),
        );
    }

    #[test]
    fn boxed_recall_first_retrieval_plan_result_preserves_selected_system_id() {
        let result = assert_boxed_plan_result_snapshot_matches_request_for_release_freeze(
            boxed_recall_first_system(),
            RECALL_FIRST_MEMORY_SYSTEM_ID,
        );

        assert_eq!(result.planner_snapshot.strategy, result.request.strategy);
    }

    #[test]
    fn builtin_request_adapter_matches_plan_result_request() {
        assert_release_freeze_request_adapter_matches_plan_result(
            &BuiltinMemorySystem,
            DEFAULT_MEMORY_SYSTEM_ID,
        );
    }

    #[test]
    fn builtin_plan_result_helper_matches_default_plan_result() {
        assert_release_freeze_plan_result_helper_matches_default(&BuiltinMemorySystem);
    }

    #[test]
    fn builtin_request_helper_matches_request_adapter() {
        assert_release_freeze_request_helper_matches_request_adapter(&BuiltinMemorySystem);
    }

    #[test]
    fn boxed_builtin_request_helper_matches_request_adapter() {
        assert_boxed_request_helper_matches_request_adapter_for_release_freeze(
            boxed_builtin_system(),
        );
    }

    #[test]
    fn boxed_builtin_request_adapter_matches_plan_result_request() {
        assert_boxed_request_adapter_matches_plan_result_for_release_freeze(
            boxed_builtin_system(),
            DEFAULT_MEMORY_SYSTEM_ID,
        );
    }

    #[test]
    fn boxed_builtin_plan_result_helper_matches_default_plan_result() {
        assert_boxed_plan_result_helper_matches_default_for_release_freeze(boxed_builtin_system());
    }

    #[test]
    fn builtin_retrieval_plan_result_matches_request_snapshot() {
        let _result = assert_plan_result_snapshot_matches_request_for_release_freeze(
            &BuiltinMemorySystem,
            DEFAULT_MEMORY_SYSTEM_ID,
        );
    }

    #[test]
    fn boxed_builtin_retrieval_plan_result_matches_request_snapshot() {
        let _result = assert_boxed_plan_result_snapshot_matches_request_for_release_freeze(
            boxed_builtin_system(),
            DEFAULT_MEMORY_SYSTEM_ID,
        );
    }

    #[test]
    fn builtin_retrieval_plan_combines_query_and_workspace_reference_scope() {
        let recent_window = release_freeze_recent_window();
        let config = MemoryRuntimeConfig {
            profile: crate::config::MemoryProfile::WindowPlusSummary,
            mode: crate::config::MemoryMode::WindowPlusSummary,
            sliding_window: 4,
            ..MemoryRuntimeConfig::default()
        };

        let plan = BuiltinRetrievalPlan::from_runtime_inputs(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            &config,
            recent_window.as_slice(),
        )
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
        );
        assert_eq!(plan.query.as_deref(), Some(RELEASE_FREEZE_TIMING));
        assert_eq!(plan.budget_items, 4);
        assert_eq!(
            plan.scopes,
            vec![
                MemoryScope::Session,
                MemoryScope::Workspace,
                MemoryScope::Agent,
                MemoryScope::User,
            ]
        );
        assert!(plan.allowed_kinds.contains(&DerivedMemoryKind::Reference));
    }

    #[test]
    fn builtin_retrieval_plan_prioritizes_recent_user_query_over_other_hints() {
        let plan = BuiltinRetrievalPlan::from_planner_inputs(BuiltinRetrievalPlannerInputs {
            has_workspace_root: true,
            recent_user_query: Some("recent user ask".to_owned()),
            recent_user_budget_items: 4,
            structured_query: Some("structured seed".to_owned()),
            task_progress_plan: Some(SeededWorkspaceRetrievalPlan {
                strategy: MemoryRetrievalStrategy::TaskProgressIntentQueryWithWorkspace,
                planning_notes: vec!["task progress".to_owned()],
                query: "task progress seed".to_owned(),
                budget_items: 2,
                allowed_kinds: vec![DerivedMemoryKind::Procedure],
            }),
            workflow_task_plan: Some(SeededWorkspaceRetrievalPlan {
                strategy: MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace,
                planning_notes: vec!["workflow task".to_owned()],
                query: "workflow seed".to_owned(),
                budget_items: 2,
                allowed_kinds: vec![DerivedMemoryKind::Overview],
            }),
            delegate_lineage_plan: Some(SeededWorkspaceRetrievalPlan::new(
                MemoryRetrievalStrategy::DelegateLabelQueryWithWorkspace,
                vec!["delegate label".to_owned()],
                "delegate label".to_owned(),
                1,
                vec![DerivedMemoryKind::Reference],
            )),
            has_structured_session_signals: true,
        })
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
        );
        assert_eq!(plan.query.as_deref(), Some("recent user ask"));
        assert_eq!(plan.budget_items, 4);
    }

    #[test]
    fn builtin_retrieval_plan_into_result_populates_planner_snapshot() {
        let plan = BuiltinRetrievalPlan::recent_user_query(true, "recent user ask".to_owned(), 4);

        let result = plan.into_result(DEFAULT_MEMORY_SYSTEM_ID, TEST_SESSION_ID);

        assert_eq!(result.request.session_id, TEST_SESSION_ID);
        assert_plan_result_identity(&result, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(
            result.planner_snapshot.strategy,
            MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
        );
        assert_eq!(result.planner_snapshot.budget_items, 4);
        assert!(result.planner_snapshot.query_present);
        assert!(
            result
                .planner_snapshot
                .planning_notes
                .iter()
                .any(|note: &String| note.contains("recent_user_query seed"))
        );
    }

    #[test]
    fn builtin_retrieval_plan_workspace_only_mode_requests_reference_recall() {
        let config = MemoryRuntimeConfig::default();

        let plan = BuiltinRetrievalPlan::from_runtime_inputs(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            &config,
            &[],
        )
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::WorkspaceReferenceOnly
        );
        assert_eq!(plan.query, None);
        assert_eq!(plan.budget_items, 1);
        assert_eq!(
            plan.scopes,
            vec![MemoryScope::Workspace, MemoryScope::Session]
        );
        assert_eq!(plan.allowed_kinds, vec![DerivedMemoryKind::Reference]);
    }

    #[test]
    fn builtin_retrieval_plan_workspace_mode_uses_structured_signal_strategy() {
        let recent_window = vec![WindowTurn {
            role: "assistant".to_owned(),
            content: crate::memory::build_tool_decision_content(
                "turn-1",
                "tool-1",
                json!({"tool_name": SHELL_EXEC_TOOL_NAME}),
            ),
            ts: None,
        }];
        let config = MemoryRuntimeConfig::default();

        let plan = BuiltinRetrievalPlan::from_runtime_inputs(
            TEST_SESSION_ID,
            Some(test_workspace_root()),
            &config,
            recent_window.as_slice(),
        )
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::StructuredSignalQueryWithWorkspace
        );
        assert_eq!(plan.query.as_deref(), Some(SHELL_EXEC_TOOL_NAME));
        assert_eq!(
            plan.allowed_kinds,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Fact,
            ]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn builtin_retrieval_plan_workspace_mode_uses_task_progress_intent_query() {
        let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
        conn.execute(
            "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, search_text, ts)
             VALUES (?1, ?2, NULL, ?3, '', 1)",
            rusqlite::params![
                TEST_SESSION_ID,
                crate::task_progress::TASK_PROGRESS_EVENT_KIND,
                serde_json::to_string(&crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: TEST_SESSION_ID.to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status: crate::task_progress::TaskProgressStatus::Waiting,
                        intent_summary: Some(DIAGNOSE_RELEASE_FREEZE_TEXT.to_owned()),
                        verification_state: None,
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at: 1,
                    },
                ))
                .expect("encode task progress payload"),
            ],
        )
        .expect("insert task progress event");

        let plan = BuiltinRetrievalPlan::from_runtime_inputs(
            TEST_SESSION_ID,
            Some(workspace_root.as_path()),
            &config,
            &[],
        )
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::TaskProgressIntentQueryWithWorkspace
        );
        assert_eq!(plan.query.as_deref(), Some(DIAGNOSE_RELEASE_FREEZE_TEXT));
        assert_eq!(plan.budget_items, 2);
        assert_eq!(
            plan.allowed_kinds,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Overview,
                DerivedMemoryKind::Fact,
            ]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn builtin_retrieval_plan_workspace_mode_uses_delegate_label_query() {
        let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
        insert_root_session(&conn);
        insert_delegate_child_session(&conn, "ready");

        let plan = BuiltinRetrievalPlan::from_runtime_inputs(
            TEST_SESSION_ID,
            Some(workspace_root.as_path()),
            &config,
            &[],
        )
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::DelegateLineageQueryWithWorkspace
        );
        let expected_query = expected_delegate_lineage_query();
        assert_eq!(plan.query.as_deref(), Some(expected_query.as_str()));
        assert_eq!(
            plan.allowed_kinds,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Overview,
            ]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn builtin_retrieval_plan_workspace_mode_uses_workflow_task_query() {
        let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
        insert_delegate_child_session(&conn, "ready");
        insert_delegate_task_event(&conn, DELEGATE_STARTED_EVENT, WORKFLOW_TASK_TEXT);

        let plan = BuiltinRetrievalPlan::from_runtime_inputs(
            TEST_SESSION_ID,
            Some(workspace_root.as_path()),
            &config,
            &[],
        )
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
        );
        assert_eq!(plan.query.as_deref(), Some(WORKFLOW_TASK_TEXT));
        assert_eq!(plan.budget_items, 2);
        assert!(
            plan.planning_notes
                .iter()
                .any(|note| note.contains("workflow task seed"))
        );
        assert_eq!(
            plan.allowed_kinds,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Overview,
                DerivedMemoryKind::Fact,
            ]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn builtin_retrieval_plan_completed_workflow_task_uses_lower_budget() {
        let (_temp_dir, workspace_root, config, conn) = sqlite_test_workspace();
        insert_delegate_child_session(&conn, "completed");
        insert_delegate_task_event(&conn, DELEGATE_COMPLETED_EVENT, WORKFLOW_TASK_TEXT);

        let plan = BuiltinRetrievalPlan::from_runtime_inputs(
            TEST_SESSION_ID,
            Some(workspace_root.as_path()),
            &config,
            &[],
        )
        .expect("retrieval plan");

        assert_eq!(
            plan.strategy,
            MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
        );
        let expected_query = expected_workflow_task_query(WORKFLOW_PHASE_COMPLETE, None, None);
        assert_eq!(plan.query.as_deref(), Some(expected_query.as_str()));
        assert_eq!(plan.budget_items, 1);
        assert_eq!(
            plan.allowed_kinds,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Overview,
                DerivedMemoryKind::Fact,
            ]
        );
    }

    #[test]
    fn workflow_task_query_seed_from_metadata_includes_non_execute_phase_and_non_task_scope() {
        let metadata = delegate_child_metadata_hint(
            WORKFLOW_PHASE_FAILED,
            Some(WORKFLOW_TASK_TEXT),
            Some(WORKFLOW_PHASE_FAILED),
            Some(WORKFLOW_OPERATION_KIND_APPROVAL),
            Some(WORKFLOW_OPERATION_SCOPE_SESSION),
        );

        let query =
            workflow_task_query_seed_from_metadata(&metadata).expect("workflow task query seed");

        let expected_query = expected_workflow_task_query(
            WORKFLOW_PHASE_FAILED,
            Some(WORKFLOW_OPERATION_KIND_APPROVAL),
            Some(WORKFLOW_OPERATION_SCOPE_SESSION),
        );
        assert_eq!(query, expected_query);
    }

    #[test]
    fn structured_signal_query_seed_from_records_dedupes_terms_and_keeps_order() {
        let records = vec![
            crate::memory::CanonicalMemoryRecord {
                session_id: TEST_SECONDARY_SESSION_ID.to_owned(),
                scope: MemoryScope::Session,
                kind: CanonicalMemoryKind::ToolDecision,
                role: Some("assistant".to_owned()),
                content: String::new(),
                metadata: json!({
                    "decision": {
                        "tool_name": SHELL_EXEC_TOOL_NAME
                    }
                }),
            },
            crate::memory::CanonicalMemoryRecord {
                session_id: TEST_SECONDARY_SESSION_ID.to_owned(),
                scope: MemoryScope::Session,
                kind: CanonicalMemoryKind::ConversationEvent,
                role: Some("assistant".to_owned()),
                content: String::new(),
                metadata: json!({
                    "event": DELEGATE_STARTED_EVENT
                }),
            },
            crate::memory::CanonicalMemoryRecord {
                session_id: TEST_SECONDARY_SESSION_ID.to_owned(),
                scope: MemoryScope::Session,
                kind: CanonicalMemoryKind::ToolOutcome,
                role: Some("assistant".to_owned()),
                content: String::new(),
                metadata: json!({
                    "outcome": {
                        "tool_name": SHELL_EXEC_TOOL_NAME
                    }
                }),
            },
        ];

        let query =
            structured_signal_query_seed_from_records(records.as_slice()).expect("structured seed");

        assert_eq!(query, expected_structured_signal_query());
    }

    #[test]
    fn delegate_lineage_query_seed_from_metadata_uses_labels_when_present() {
        let metadata = delegate_child_metadata_hint("running", None, None, None, None);

        let query =
            delegate_lineage_query_seed_from_metadata(&metadata).expect("delegate lineage seed");

        assert_eq!(query, expected_delegate_lineage_query());
    }

    #[test]
    fn workflow_task_policy_from_metadata_prefers_procedure_for_approval_kind() {
        let metadata = delegate_child_metadata_hint(
            WORKFLOW_PHASE_FAILED,
            Some(WORKFLOW_TASK_TEXT),
            Some(WORKFLOW_PHASE_FAILED),
            Some(WORKFLOW_OPERATION_KIND_APPROVAL),
            Some(WORKFLOW_OPERATION_SCOPE_SESSION),
        );

        let policy = workflow_task_policy_from_metadata(&metadata);

        assert_eq!(
            policy.allowed_kinds,
            vec![
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Fact,
            ]
        );
        assert_eq!(policy.budget_items, 1);
    }

    #[test]
    fn workspace_recall_memory_system_metadata_is_stable() {
        let metadata = WorkspaceRecallMemorySystem.metadata();
        assert_eq!(metadata.id, WORKSPACE_RECALL_MEMORY_SYSTEM_ID);
        assert_eq!(
            metadata.runtime_fallback_kind,
            MemorySystemRuntimeFallbackKind::SystemBacked
        );
        assert_eq!(
            metadata.capability_names(),
            vec!["prompt_hydration", "retrieval_provenance"]
        );
        assert_eq!(
            metadata.supported_pre_assembly_stage_families,
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::Rank,
            ]
        );
        assert_eq!(
            metadata.supported_stage_families,
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::Rank,
            ]
        );
        assert_eq!(
            metadata.supported_recall_modes,
            vec![
                MemoryRecallMode::PromptAssembly,
                MemoryRecallMode::OperatorInspection
            ]
        );
    }

    #[test]
    fn workspace_recall_retrieval_plan_result_matches_request_snapshot() {
        let result = assert_plan_result_snapshot_matches_request_for_workspace_recall(
            &WorkspaceRecallMemorySystem,
        );
        assert_workspace_recall_plan_result_snapshot_matches_request_and_budget(&result);
    }

    #[test]
    fn workspace_recall_plan_result_helper_matches_default_plan_result() {
        assert_plan_result_helper_matches_default_for_workspace_recall(
            &WorkspaceRecallMemorySystem,
        );
    }

    #[test]
    fn workspace_recall_request_adapter_matches_plan_result_request() {
        assert_request_adapter_matches_plan_result_for_workspace_recall(
            &WorkspaceRecallMemorySystem,
        );
        assert_workspace_recall_request_budget_items(&WorkspaceRecallMemorySystem, 4);
    }

    #[test]
    fn workspace_recall_request_helper_matches_request_adapter() {
        assert_request_helper_matches_request_adapter_for_workspace_recall(
            &WorkspaceRecallMemorySystem,
        );
    }

    #[test]
    fn boxed_workspace_recall_request_helper_matches_request_adapter() {
        assert_boxed_request_helper_matches_request_adapter_for_workspace_recall(
            boxed_workspace_recall_system(),
        );
    }

    #[test]
    fn boxed_workspace_recall_request_adapter_matches_plan_result_request() {
        assert_boxed_request_adapter_matches_plan_result_for_workspace_recall(
            boxed_workspace_recall_system(),
        );
    }

    #[test]
    fn boxed_workspace_recall_request_budget_items_match_expected() {
        assert_boxed_workspace_recall_request_budget_items(boxed_workspace_recall_system(), 4);
    }

    #[test]
    fn boxed_workspace_recall_plan_result_helper_matches_default_plan_result() {
        assert_boxed_plan_result_helper_matches_default_for_workspace_recall(
            boxed_workspace_recall_system(),
        );
    }

    #[test]
    fn boxed_workspace_recall_retrieval_plan_result_matches_request_snapshot() {
        let result = assert_boxed_plan_result_snapshot_matches_request_for_workspace_recall(
            boxed_workspace_recall_system(),
        );
        assert_workspace_recall_plan_result_snapshot_matches_request_and_budget(&result);
    }

    #[test]
    fn supported_stage_families_replace_previous_explicit_stage_set() {
        let metadata = MemorySystemMetadata::new(
            "stage-reset",
            [MemorySystemCapability::PromptHydration],
            "Stage reset test system",
        )
        .with_supported_pre_assembly_stage_families([
            MemoryStageFamily::Derive,
            MemoryStageFamily::Retrieve,
        ])
        .with_supported_stage_families([MemoryStageFamily::Compact])
        .with_supported_stage_families([MemoryStageFamily::AfterTurn]);

        assert_eq!(
            metadata.supported_stage_families,
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::AfterTurn,
            ]
        );
    }

    #[test]
    fn supported_stage_builders_auto_promote_runtime_fallback_to_system_backed() {
        let metadata = MemorySystemMetadata::new(
            "auto-promote",
            [MemorySystemCapability::PromptHydration],
            "Auto promote test system",
        )
        .with_supported_pre_assembly_stage_families([MemoryStageFamily::Retrieve]);

        assert_eq!(
            metadata.runtime_fallback_kind,
            MemorySystemRuntimeFallbackKind::SystemBacked
        );
    }

    #[test]
    fn memory_system_runtime_snapshot_defaults_to_builtin() {
        let config = crate::config::LoongConfig::default();
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

    #[test]
    fn recall_first_rank_stage_drops_summary_when_retrieved_entries_exist() {
        let entries = vec![
            MemoryContextEntry {
                kind: MemoryContextKind::Summary,
                role: "system".to_owned(),
                content: "summary".to_owned(),
                provenance: Vec::new(),
            },
            MemoryContextEntry {
                kind: MemoryContextKind::Profile,
                role: "system".to_owned(),
                content: "profile".to_owned(),
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

        let runtime_config = MemoryRuntimeConfig::default();
        let ranked_entries_result =
            RecallFirstMemorySystem.run_rank_stage(entries, &runtime_config);
        let ranked_entries_option = ranked_entries_result.expect("rank stage should succeed");
        let ranked_entries =
            ranked_entries_option.expect("recall-first rank stage should return entries");

        let kinds = ranked_entries
            .into_iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                MemoryContextKind::Profile,
                MemoryContextKind::RetrievedMemory,
                MemoryContextKind::Turn,
            ]
        );
    }

    #[test]
    fn filter_cross_session_recall_hits_respects_scopes_and_allowed_kinds() {
        let profile_hit = CanonicalMemorySearchHit {
            record: crate::memory::CanonicalMemoryRecord {
                session_id: "profile-session".to_owned(),
                scope: MemoryScope::Workspace,
                kind: CanonicalMemoryKind::ImportedProfile,
                role: None,
                content: "release checklist".to_owned(),
                metadata: json!({}),
            },
            session_turn_index: Some(1),
        };
        let turn_hit = CanonicalMemorySearchHit {
            record: crate::memory::CanonicalMemoryRecord {
                session_id: "turn-session".to_owned(),
                scope: MemoryScope::Session,
                kind: CanonicalMemoryKind::AssistantTurn,
                role: Some("assistant".to_owned()),
                content: "deployment cutoff is 17:00".to_owned(),
                metadata: json!({}),
            },
            session_turn_index: Some(2),
        };
        let request = MemoryRetrievalRequest {
            session_id: "active-session".to_owned(),
            memory_system_id: DEFAULT_MEMORY_SYSTEM_ID.to_owned(),
            strategy: MemoryRetrievalStrategy::RecentUserQuery,
            planning_notes: Vec::new(),
            query: Some("deployment release".to_owned()),
            recall_mode: MemoryRecallMode::PromptAssembly,
            scopes: vec![MemoryScope::Workspace],
            budget_items: 4,
            allowed_kinds: vec![DerivedMemoryKind::Profile],
        };

        let filtered_hits = filter_cross_session_recall_hits(&request, vec![profile_hit, turn_hit]);

        assert_eq!(filtered_hits.len(), 1);
        assert_eq!(filtered_hits[0].record.session_id, "profile-session");
        assert_eq!(
            filtered_hits[0].record.kind,
            CanonicalMemoryKind::ImportedProfile
        );
    }

    #[test]
    fn build_cross_session_recall_entries_attach_canonical_record_provenance() {
        let hit = CanonicalMemorySearchHit {
            record: crate::memory::CanonicalMemoryRecord {
                session_id: "prior-session".to_owned(),
                scope: MemoryScope::Session,
                kind: CanonicalMemoryKind::AssistantTurn,
                role: Some("assistant".to_owned()),
                content: "deployment cutoff is 17:00 Beijing time".to_owned(),
                metadata: json!({}),
            },
            session_turn_index: Some(3),
        };

        let entries = build_cross_session_recall_entries(
            DEFAULT_MEMORY_SYSTEM_ID,
            MemoryRecallMode::PromptAssembly,
            &[hit],
        );

        assert_eq!(entries.len(), 1);
        assert!(
            entries[0]
                .content
                .contains("Cross-session source: prior-session")
        );
        assert_eq!(entries[0].provenance.len(), 1);
        assert_eq!(
            entries[0].provenance[0].source_kind,
            MemoryProvenanceSourceKind::CanonicalMemoryRecord
        );
        assert_eq!(entries[0].provenance[0].scope, Some(MemoryScope::Session));
        assert_eq!(
            entries[0].provenance[0].trust_level,
            Some(MemoryTrustLevel::Session)
        );
        assert_eq!(
            entries[0].provenance[0].record_status,
            Some(MemoryRecordStatus::Active)
        );
    }

    #[test]
    fn builtin_memory_system_derives_session_local_overview_from_structured_turns() {
        let recent_window = vec![
            WindowTurn {
                role: "assistant".to_owned(),
                content: crate::memory::build_tool_decision_content(
                    "turn-1",
                    "call-1",
                    json!({"tool": "memory_search"}),
                ),
                ts: Some(10),
            },
            WindowTurn {
                role: "assistant".to_owned(),
                content: crate::memory::build_conversation_event_content(
                    "tool_discovery",
                    json!({"state": "visible"}),
                ),
                ts: Some(20),
            },
        ];

        let derived_entries = BuiltinMemorySystem
            .run_derive_stage(
                "session-local-overview-session",
                &MemoryRuntimeConfig::default(),
                recent_window.as_slice(),
            )
            .expect("derive stage should succeed")
            .expect("derive stage should return entries");

        assert_eq!(derived_entries.len(), 1);
        assert_eq!(derived_entries[0].kind, MemoryContextKind::Derived);
        assert!(
            derived_entries[0]
                .content
                .contains("## Session Local Overview")
        );
        assert_eq!(
            derived_entries[0].provenance[0].source_kind,
            MemoryProvenanceSourceKind::DerivedSessionOverview
        );
        assert_eq!(
            derived_entries[0].provenance[0].derived_kind,
            Some(DerivedMemoryKind::Overview)
        );
    }

    #[test]
    fn builtin_rank_stage_filters_inactive_workspace_entries_and_orders_advisory_blocks() {
        let inactive_entry = MemoryContextEntry {
            kind: MemoryContextKind::RetrievedMemory,
            role: "system".to_owned(),
            content: "inactive".to_owned(),
            provenance: vec![
                MemoryContextProvenance::new(
                    DEFAULT_MEMORY_SYSTEM_ID,
                    MemoryProvenanceSourceKind::WorkspaceDocument,
                    Some("MEMORY.md".to_owned()),
                    None,
                    Some(MemoryScope::Workspace),
                    MemoryRecallMode::PromptAssembly,
                )
                .with_trust_level(MemoryTrustLevel::WorkspaceCurated)
                .with_record_status(MemoryRecordStatus::Tombstoned),
            ],
        };
        let summary_entry = MemoryContextEntry {
            kind: MemoryContextKind::Summary,
            role: "system".to_owned(),
            content: "summary".to_owned(),
            provenance: vec![
                MemoryContextProvenance::new(
                    DEFAULT_MEMORY_SYSTEM_ID,
                    MemoryProvenanceSourceKind::SummaryCheckpoint,
                    Some("summary_checkpoint".to_owned()),
                    None,
                    Some(MemoryScope::Session),
                    MemoryRecallMode::PromptAssembly,
                )
                .with_trust_level(MemoryTrustLevel::Derived)
                .with_record_status(MemoryRecordStatus::Active),
            ],
        };
        let derived_entry = MemoryContextEntry {
            kind: MemoryContextKind::Derived,
            role: "system".to_owned(),
            content: "derived".to_owned(),
            provenance: vec![
                MemoryContextProvenance::new(
                    DEFAULT_MEMORY_SYSTEM_ID,
                    MemoryProvenanceSourceKind::DerivedSessionOverview,
                    Some("session_local_overview".to_owned()),
                    None,
                    Some(MemoryScope::Session),
                    MemoryRecallMode::PromptAssembly,
                )
                .with_trust_level(MemoryTrustLevel::Derived)
                .with_record_status(MemoryRecordStatus::Active),
            ],
        };
        let turn_entry = MemoryContextEntry {
            kind: MemoryContextKind::Turn,
            role: "user".to_owned(),
            content: "turn".to_owned(),
            provenance: Vec::new(),
        };

        let ranked_entries = BuiltinMemorySystem
            .run_rank_stage(
                vec![turn_entry, inactive_entry, derived_entry, summary_entry],
                &MemoryRuntimeConfig::default(),
            )
            .expect("rank stage should succeed")
            .expect("rank stage should return entries");

        let kinds = ranked_entries
            .into_iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                MemoryContextKind::Summary,
                MemoryContextKind::Derived,
                MemoryContextKind::Turn,
            ]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn builtin_retrieve_stage_keeps_allowed_hits_when_top_match_is_filtered_out() {
        let (_temp_dir, _workspace_root, config, _conn) = sqlite_test_workspace();
        let allowed_payload = json!({
            "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "release checklist",
            "metadata": {
                "source": "workspace-import"
            },
        })
        .to_string();
        let recent_window = Vec::new();
        let request = MemoryRetrievalRequest {
            session_id: "active-session".to_owned(),
            memory_system_id: DEFAULT_MEMORY_SYSTEM_ID.to_owned(),
            strategy: MemoryRetrievalStrategy::RecentUserQuery,
            planning_notes: Vec::new(),
            query: Some("release checklist".to_owned()),
            recall_mode: MemoryRecallMode::PromptAssembly,
            scopes: vec![MemoryScope::Workspace],
            budget_items: 1,
            allowed_kinds: vec![DerivedMemoryKind::Profile],
        };

        crate::memory::append_turn_direct(
            "workspace-session",
            "assistant",
            allowed_payload.as_str(),
            &config,
        )
        .expect("append allowed canonical payload");
        crate::memory::append_turn_direct(
            "session-session",
            "assistant",
            "release checklist",
            &config,
        )
        .expect("append disallowed session hit");

        let entries = BuiltinMemorySystem
            .run_retrieve_stage(&request, None, &config, recent_window.as_slice())
            .expect("retrieve stage should succeed")
            .expect("retrieve stage should return entries");

        assert_eq!(entries.len(), 1);
        assert!(entries[0].content.contains("workspace-session"));
        assert!(!entries[0].content.contains("session-session"));
        assert_eq!(entries[0].provenance.len(), 1);
        assert_eq!(entries[0].provenance[0].scope, Some(MemoryScope::Workspace));
    }
}
