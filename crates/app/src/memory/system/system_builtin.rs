use super::*;
use crate::memory::{
    CanonicalMemoryRecord, canonical_memory_record_from_persisted_turn, orchestrator, sqlite,
};

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

pub(super) fn build_workspace_retrieval_plan_result(
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
pub(super) struct BuiltinRetrievalPlan {
    pub(super) strategy: MemoryRetrievalStrategy,
    pub(super) planning_notes: Vec<String>,
    pub(super) query: Option<String>,
    pub(super) scopes: Vec<MemoryScope>,
    pub(super) budget_items: usize,
    pub(super) allowed_kinds: Vec<DerivedMemoryKind>,
}

pub(super) struct BuiltinRetrievalPlannerInputs {
    pub(super) has_workspace_root: bool,
    pub(super) recent_user_query: Option<String>,
    pub(super) recent_user_budget_items: usize,
    pub(super) structured_query: Option<String>,
    pub(super) task_progress_plan: Option<SeededWorkspaceRetrievalPlan>,
    pub(super) workflow_task_plan: Option<SeededWorkspaceRetrievalPlan>,
    pub(super) delegate_lineage_plan: Option<SeededWorkspaceRetrievalPlan>,
    pub(super) has_structured_session_signals: bool,
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

    pub(super) fn recent_user_query(
        has_workspace_root: bool,
        query: String,
        budget_items: usize,
    ) -> Self {
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

    pub(super) fn from_runtime_inputs(
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
    ) -> Option<Self> {
        let has_workspace_root = workspace_root.is_some();
        let supports_query_recall =
            matches!(config.mode, crate::config::MemoryMode::WindowPlusSummary);
        let recent_user_query = if supports_query_recall {
            orchestrator::retrieval_query_from_recent_window(recent_window)
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

    pub(super) fn from_planner_inputs(inputs: BuiltinRetrievalPlannerInputs) -> Option<Self> {
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

    pub(super) fn into_result(
        self,
        memory_system_id: &str,
        session_id: &str,
    ) -> MemoryRetrievalPlanResult {
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

pub(super) struct SeededWorkspaceRetrievalPlan {
    pub(super) strategy: MemoryRetrievalStrategy,
    pub(super) planning_notes: Vec<String>,
    pub(super) query: String,
    pub(super) budget_items: usize,
    pub(super) allowed_kinds: Vec<DerivedMemoryKind>,
}

impl SeededWorkspaceRetrievalPlan {
    pub(super) fn new(
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
pub(super) struct WorkspaceRetrievalPolicy {
    pub(super) budget_items: usize,
    pub(super) allowed_kinds: Vec<DerivedMemoryKind>,
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
        let record = sqlite::load_latest_task_progress_record(session_id, config)
            .ok()
            .flatten()?;
        let query = record
            .intent_summary
            .map(|value: String| value.trim().to_owned())
            .filter(|value: &String| !value.is_empty())?;
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
        let metadata = sqlite::load_session_metadata_hint(session_id, config)
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

pub(super) fn workflow_task_query_seed_from_metadata(
    metadata: &sqlite::SessionMetadataHint,
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
            .filter(|value: &&str| value.trim() != "execute"),
        Some(|value: &str| format!("phase: {}", value.trim())),
    );
    append_unique_query_line(
        &mut lines,
        metadata
            .workflow_operation_kind
            .as_deref()
            .filter(|value: &&str| value.trim() != "task"),
        Some(|value: &str| format!("operation_kind: {}", value.trim())),
    );
    append_unique_query_line(
        &mut lines,
        metadata
            .workflow_operation_scope
            .as_deref()
            .filter(|value: &&str| value.trim() != "task"),
        Some(|value: &str| format!("operation_scope: {}", value.trim())),
    );

    finalize_query_lines(lines)
}

pub(super) fn structured_signal_query_seed_from_records(
    records: &[CanonicalMemoryRecord],
) -> Option<String> {
    const MAX_SIGNAL_TERMS: usize = 3;

    let mut terms = Vec::new();

    for record in records.iter().rev() {
        let maybe_term = match record.kind {
            CanonicalMemoryKind::ToolDecision => record
                .metadata
                .get("decision")
                .and_then(|value: &Value| value.get("tool_name"))
                .and_then(Value::as_str),
            CanonicalMemoryKind::ToolOutcome => record
                .metadata
                .get("outcome")
                .and_then(|value: &Value| value.get("tool_name"))
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

pub(super) fn workflow_task_policy_from_metadata(
    metadata: &sqlite::SessionMetadataHint,
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

pub(super) fn delegate_lineage_query_seed_from_metadata(
    metadata: &sqlite::SessionMetadataHint,
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
        let metadata = sqlite::load_session_metadata_hint(session_id, config)
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

pub(super) fn run_builtin_retrieve_stage(
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
        let hits = sqlite::search_canonical_records_for_recall(
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

pub(super) fn rank_recall_first_entries(
    entries: Vec<MemoryContextEntry>,
) -> Vec<MemoryContextEntry> {
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

pub(super) fn filter_cross_session_recall_hits(
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

pub(super) fn build_cross_session_recall_entries(
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
    let content = orchestrator::truncate_recall_content(hit.record.content.as_str(), 280);
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

pub(super) fn derive_session_overview_entry(
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
) -> Vec<CanonicalMemoryRecord> {
    let mut records = Vec::new();

    for turn in recent_window {
        let record = canonical_memory_record_from_persisted_turn(
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

fn render_session_overview_block(records: &[CanonicalMemoryRecord]) -> String {
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

fn count_canonical_kind(records: &[CanonicalMemoryRecord], kind: CanonicalMemoryKind) -> usize {
    records.iter().filter(|record| record.kind == kind).count()
}

fn collect_record_kind_names(records: &[CanonicalMemoryRecord]) -> Vec<String> {
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
