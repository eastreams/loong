use serde::{Deserialize, Serialize};

use super::{HydratedMemoryContext, MemoryScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryStageFamily {
    Derive,
    Retrieve,
    Rank,
    AfterTurn,
    Compact,
}

impl MemoryStageFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Derive => "derive",
            Self::Retrieve => "retrieve",
            Self::Rank => "rank",
            Self::AfterTurn => "after_turn",
            Self::Compact => "compact",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "derive" => Some(Self::Derive),
            "retrieve" => Some(Self::Retrieve),
            "rank" => Some(Self::Rank),
            "after_turn" => Some(Self::AfterTurn),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageOutcome {
    Succeeded,
    Fallback,
    Failed,
    Skipped,
}

impl StageOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Fallback => "fallback",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "succeeded" => Some(Self::Succeeded),
            "fallback" => Some(Self::Fallback),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedMemoryKind {
    Summary,
    Profile,
    Fact,
    Entity,
    Episode,
    Procedure,
    Overview,
    Reference,
}

impl DerivedMemoryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Profile => "profile",
            Self::Fact => "fact",
            Self::Entity => "entity",
            Self::Episode => "episode",
            Self::Procedure => "procedure",
            Self::Overview => "overview",
            Self::Reference => "reference",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "summary" => Some(Self::Summary),
            "profile" => Some(Self::Profile),
            "fact" => Some(Self::Fact),
            "entity" => Some(Self::Entity),
            "episode" => Some(Self::Episode),
            "procedure" => Some(Self::Procedure),
            "overview" => Some(Self::Overview),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRecallMode {
    #[default]
    PromptAssembly,
    OperatorInspection,
    EvaluationEvidence,
    BackgroundDerivation,
}

impl MemoryRecallMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PromptAssembly => "prompt_assembly",
            Self::OperatorInspection => "operator_inspection",
            Self::EvaluationEvidence => "evaluation_evidence",
            Self::BackgroundDerivation => "background_derivation",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "prompt_assembly" => Some(Self::PromptAssembly),
            "operator_inspection" => Some(Self::OperatorInspection),
            "evaluation_evidence" => Some(Self::EvaluationEvidence),
            "background_derivation" => Some(Self::BackgroundDerivation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProvenanceSourceKind {
    WorkspaceDocument,
    CanonicalMemoryRecord,
    DerivedSessionOverview,
    ProfileNote,
    SummaryCheckpoint,
    RecentWindowTurn,
    MemorySystem,
}

impl MemoryProvenanceSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceDocument => "workspace_document",
            Self::CanonicalMemoryRecord => "canonical_memory_record",
            Self::DerivedSessionOverview => "derived_session_overview",
            Self::ProfileNote => "profile_note",
            Self::SummaryCheckpoint => "summary_checkpoint",
            Self::RecentWindowTurn => "recent_window_turn",
            Self::MemorySystem => "memory_system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTrustLevel {
    Session,
    Derived,
    WorkspaceCurated,
    WorkspaceLog,
}

impl MemoryTrustLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Derived => "derived",
            Self::WorkspaceCurated => "workspace_curated",
            Self::WorkspaceLog => "workspace_log",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "session" => Some(Self::Session),
            "derived" => Some(Self::Derived),
            "workspace_curated" => Some(Self::WorkspaceCurated),
            "workspace_log" => Some(Self::WorkspaceLog),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAuthority {
    Advisory,
    IdentityForbidden,
}

impl MemoryAuthority {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Advisory => "advisory",
            Self::IdentityForbidden => "identity_forbidden",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "advisory" => Some(Self::Advisory),
            "identity_forbidden" => Some(Self::IdentityForbidden),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRecordStatus {
    Active,
    Superseded,
    Tombstoned,
    Archived,
}

impl MemoryRecordStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Tombstoned => "tombstoned",
            Self::Archived => "archived",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "tombstoned" => Some(Self::Tombstoned),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }

    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryContextProvenance {
    pub memory_system_id: String,
    pub source_kind: MemoryProvenanceSourceKind,
    pub source_label: Option<String>,
    pub source_path: Option<String>,
    pub scope: Option<MemoryScope>,
    pub recall_mode: MemoryRecallMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_level: Option<MemoryTrustLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<MemoryAuthority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_kind: Option<DerivedMemoryKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ts: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_status: Option<MemoryRecordStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

impl MemoryContextProvenance {
    pub fn new(
        memory_system_id: &str,
        source_kind: MemoryProvenanceSourceKind,
        source_label: Option<String>,
        source_path: Option<String>,
        scope: Option<MemoryScope>,
        recall_mode: MemoryRecallMode,
    ) -> Self {
        let normalized_system_id = super::normalize_system_id(memory_system_id)
            .unwrap_or_else(|| memory_system_id.to_owned());

        Self {
            memory_system_id: normalized_system_id,
            source_kind,
            source_label,
            source_path,
            scope,
            recall_mode,
            trust_level: None,
            authority: None,
            derived_kind: None,
            freshness_ts: None,
            content_hash: None,
            record_status: None,
            superseded_by: None,
        }
    }

    pub fn with_trust_level(mut self, trust_level: MemoryTrustLevel) -> Self {
        self.trust_level = Some(trust_level);
        self
    }

    pub fn with_authority(mut self, authority: MemoryAuthority) -> Self {
        self.authority = Some(authority);
        self
    }

    pub fn with_derived_kind(mut self, derived_kind: DerivedMemoryKind) -> Self {
        self.derived_kind = Some(derived_kind);
        self
    }

    pub fn with_freshness_ts(mut self, freshness_ts: i64) -> Self {
        self.freshness_ts = Some(freshness_ts);
        self
    }

    pub fn with_content_hash(mut self, content_hash: impl Into<String>) -> Self {
        self.content_hash = Some(content_hash.into());
        self
    }

    pub fn with_record_status(mut self, record_status: MemoryRecordStatus) -> Self {
        self.record_status = Some(record_status);
        self
    }

    pub fn with_superseded_by(mut self, superseded_by: impl Into<String>) -> Self {
        self.superseded_by = Some(superseded_by.into());
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRetrievalStrategy {
    #[default]
    Unspecified,
    RecentUserQuery,
    RecentUserQueryWithWorkspace,
    WorkspaceReferenceOnly,
    WorkspaceReferenceWithStructuredSignals,
    StructuredSignalQueryWithWorkspace,
    TaskProgressIntentQueryWithWorkspace,
    DelegateLabelQueryWithWorkspace,
    DelegateLineageQueryWithWorkspace,
    DelegateTaskQueryWithWorkspace,
    WorkflowTaskQueryWithWorkspace,
}

impl MemoryRetrievalStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::RecentUserQuery => "recent_user_query",
            Self::RecentUserQueryWithWorkspace => "recent_user_query_with_workspace",
            Self::WorkspaceReferenceOnly => "workspace_reference_only",
            Self::WorkspaceReferenceWithStructuredSignals => {
                "workspace_reference_with_structured_signals"
            }
            Self::StructuredSignalQueryWithWorkspace => "structured_signal_query_with_workspace",
            Self::TaskProgressIntentQueryWithWorkspace => {
                "task_progress_intent_query_with_workspace"
            }
            Self::DelegateLabelQueryWithWorkspace => "delegate_label_query_with_workspace",
            Self::DelegateLineageQueryWithWorkspace => "delegate_lineage_query_with_workspace",
            Self::DelegateTaskQueryWithWorkspace => "delegate_task_query_with_workspace",
            Self::WorkflowTaskQueryWithWorkspace => "workflow_task_query_with_workspace",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "unspecified" => Some(Self::Unspecified),
            "recent_user_query" => Some(Self::RecentUserQuery),
            "recent_user_query_with_workspace" => Some(Self::RecentUserQueryWithWorkspace),
            "workspace_reference_only" => Some(Self::WorkspaceReferenceOnly),
            "workspace_reference_with_structured_signals" => {
                Some(Self::WorkspaceReferenceWithStructuredSignals)
            }
            "structured_signal_query_with_workspace" => {
                Some(Self::StructuredSignalQueryWithWorkspace)
            }
            "task_progress_intent_query_with_workspace" => {
                Some(Self::TaskProgressIntentQueryWithWorkspace)
            }
            "delegate_label_query_with_workspace" => Some(Self::DelegateLabelQueryWithWorkspace),
            "delegate_lineage_query_with_workspace" => {
                Some(Self::DelegateLineageQueryWithWorkspace)
            }
            "delegate_task_query_with_workspace" => Some(Self::DelegateTaskQueryWithWorkspace),
            "workflow_task_query_with_workspace" => Some(Self::WorkflowTaskQueryWithWorkspace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRetrievalRequest {
    pub session_id: String,
    pub memory_system_id: String,
    pub strategy: MemoryRetrievalStrategy,
    pub planning_notes: Vec<String>,
    pub query: Option<String>,
    pub recall_mode: MemoryRecallMode,
    pub scopes: Vec<MemoryScope>,
    pub budget_items: usize,
    pub allowed_kinds: Vec<DerivedMemoryKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRetrievalPlanResult {
    pub request: MemoryRetrievalRequest,
    pub planner_snapshot: PlannerDiagnosticsSnapshot,
}

impl MemoryRetrievalRequest {
    pub fn planner_snapshot(&self) -> PlannerDiagnosticsSnapshot {
        PlannerDiagnosticsSnapshot {
            memory_system_id: self.memory_system_id.clone(),
            strategy: self.strategy,
            budget_items: self.budget_items,
            query_present: self.query.is_some(),
            planning_notes: self.planning_notes.clone(),
        }
    }

    pub fn into_plan_result(self) -> MemoryRetrievalPlanResult {
        let planner_snapshot = self.planner_snapshot();
        MemoryRetrievalPlanResult {
            request: self,
            planner_snapshot,
        }
    }
}

impl MemoryRetrievalPlanResult {
    pub fn request(&self) -> &MemoryRetrievalRequest {
        &self.request
    }

    pub fn planner_snapshot(&self) -> &PlannerDiagnosticsSnapshot {
        &self.planner_snapshot
    }

    pub fn into_request(self) -> MemoryRetrievalRequest {
        self.request
    }

    pub fn into_parts(self) -> (MemoryRetrievalRequest, PlannerDiagnosticsSnapshot) {
        (self.request, self.planner_snapshot)
    }

    pub fn planner_summary_message(&self) -> String {
        let query_state = if self.planner_snapshot.query_present {
            "present"
        } else {
            "absent"
        };
        let notes = if self.planner_snapshot.planning_notes.is_empty() {
            "none".to_owned()
        } else {
            self.planner_snapshot.planning_notes.join("; ")
        };

        format!(
            "planner system={} strategy={} budget={} query={} notes={}",
            self.planner_snapshot.memory_system_id,
            self.planner_snapshot.strategy.as_str(),
            self.planner_snapshot.budget_items,
            query_state,
            notes
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDiagnostics {
    pub family: MemoryStageFamily,
    pub outcome: StageOutcome,
    pub budget_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub fallback_activated: bool,
    pub message: Option<String>,
    pub planner_snapshot: Option<PlannerDiagnosticsSnapshot>,
}

impl StageDiagnostics {
    pub fn succeeded(family: MemoryStageFamily) -> Self {
        Self {
            family,
            outcome: StageOutcome::Succeeded,
            budget_ms: None,
            elapsed_ms: None,
            fallback_activated: false,
            message: None,
            planner_snapshot: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerDiagnosticsSnapshot {
    #[serde(default)]
    pub memory_system_id: String,
    pub strategy: MemoryRetrievalStrategy,
    pub budget_items: usize,
    pub query_present: bool,
    pub planning_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageEnvelope {
    pub hydrated: HydratedMemoryContext,
    pub retrieval_request: Option<MemoryRetrievalRequest>,
    pub retrieval_planner_snapshot: Option<PlannerDiagnosticsSnapshot>,
    pub diagnostics: Vec<StageDiagnostics>,
}

pub fn builtin_pre_assembly_stage_families() -> Vec<MemoryStageFamily> {
    // `Compact` stays part of the declared vocabulary but is intentionally inactive in slice 1.
    vec![
        MemoryStageFamily::Derive,
        MemoryStageFamily::Retrieve,
        MemoryStageFamily::Rank,
    ]
}

pub fn builtin_post_turn_stage_families() -> Vec<MemoryStageFamily> {
    vec![MemoryStageFamily::AfterTurn]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{
        HydratedMemoryContext, MemoryDiagnostics, MemoryScope, decode_stage_envelope,
        encode_stage_envelope_payload,
    };

    #[test]
    fn stage_families_have_stable_builtin_order() {
        assert_eq!(
            builtin_pre_assembly_stage_families(),
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::Rank,
            ]
        );
        assert_eq!(
            builtin_post_turn_stage_families(),
            vec![MemoryStageFamily::AfterTurn]
        );
    }

    #[test]
    fn stage_envelope_round_trips_through_protocol_payload() {
        let envelope = StageEnvelope {
            hydrated: HydratedMemoryContext {
                entries: vec![],
                recent_window: vec![],
                diagnostics: MemoryDiagnostics {
                    system_id: "builtin".to_owned(),
                    fail_open: true,
                    strict_mode_requested: false,
                    strict_mode_active: false,
                    degraded: false,
                    derivation_error: None,
                    retrieval_error: None,
                    rank_error: None,
                    recent_window_count: 0,
                    entry_count: 0,
                },
            },
            retrieval_request: Some(MemoryRetrievalRequest {
                session_id: "session-123".to_owned(),
                memory_system_id: "builtin".to_owned(),
                strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
                planning_notes: vec!["workspace_root present".to_owned()],
                query: None,
                recall_mode: MemoryRecallMode::PromptAssembly,
                scopes: vec![MemoryScope::Session],
                budget_items: 8,
                allowed_kinds: vec![DerivedMemoryKind::Summary],
            }),
            retrieval_planner_snapshot: None,
            diagnostics: vec![StageDiagnostics::succeeded(MemoryStageFamily::Derive)],
        };

        let payload = encode_stage_envelope_payload(&envelope);
        assert_eq!(decode_stage_envelope(&payload), Some(envelope));
    }

    #[test]
    fn stage_envelope_round_trips_non_builtin_system_id_through_protocol_payload() {
        let envelope = StageEnvelope {
            hydrated: HydratedMemoryContext {
                entries: vec![],
                recent_window: vec![],
                diagnostics: MemoryDiagnostics {
                    system_id: "Lucid".to_owned(),
                    fail_open: false,
                    strict_mode_requested: false,
                    strict_mode_active: false,
                    degraded: false,
                    derivation_error: None,
                    retrieval_error: None,
                    rank_error: None,
                    recent_window_count: 0,
                    entry_count: 0,
                },
            },
            retrieval_request: None,
            retrieval_planner_snapshot: None,
            diagnostics: vec![],
        };

        let payload = encode_stage_envelope_payload(&envelope);
        let decoded = decode_stage_envelope(&payload).expect("decode stage envelope");
        assert_eq!(decoded.hydrated.diagnostics.system_id, "lucid");
    }

    #[test]
    fn retrieval_plan_result_into_parts_round_trips_request_and_snapshot() {
        let request = MemoryRetrievalRequest {
            session_id: "session-123".to_owned(),
            memory_system_id: "recall_first".to_owned(),
            strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
            planning_notes: vec!["workspace recall system".to_owned()],
            query: None,
            recall_mode: MemoryRecallMode::PromptAssembly,
            scopes: vec![MemoryScope::Workspace],
            budget_items: 1,
            allowed_kinds: vec![DerivedMemoryKind::Reference],
        };
        let expected_snapshot = request.planner_snapshot();
        let result = request.clone().into_plan_result();

        let (actual_request, actual_snapshot) = result.into_parts();

        assert_eq!(actual_request, request);
        assert_eq!(actual_snapshot, expected_snapshot);
    }

    #[test]
    fn retrieval_plan_result_summary_message_uses_snapshot_state() {
        let result = MemoryRetrievalPlanResult {
            request: MemoryRetrievalRequest {
                session_id: "session-123".to_owned(),
                memory_system_id: "builtin".to_owned(),
                strategy: MemoryRetrievalStrategy::RecentUserQueryWithWorkspace,
                planning_notes: vec!["request note".to_owned()],
                query: Some("request query".to_owned()),
                recall_mode: MemoryRecallMode::PromptAssembly,
                scopes: vec![MemoryScope::Workspace],
                budget_items: 4,
                allowed_kinds: vec![DerivedMemoryKind::Reference],
            },
            planner_snapshot: PlannerDiagnosticsSnapshot {
                memory_system_id: "recall_first".to_owned(),
                strategy: MemoryRetrievalStrategy::WorkspaceReferenceOnly,
                budget_items: 1,
                query_present: false,
                planning_notes: vec!["snapshot note".to_owned()],
            },
        };

        let summary = result.planner_summary_message();

        assert_eq!(
            summary,
            "planner system=recall_first strategy=workspace_reference_only budget=1 query=absent notes=snapshot note"
        );
    }

    #[test]
    fn compact_stage_family_is_reserved_but_not_in_builtin_slice_one_ordering() {
        assert_eq!(
            MemoryStageFamily::parse_id("compact"),
            Some(MemoryStageFamily::Compact)
        );
        assert!(!builtin_pre_assembly_stage_families().contains(&MemoryStageFamily::Compact));
        assert!(!builtin_post_turn_stage_families().contains(&MemoryStageFamily::Compact));
    }
}
