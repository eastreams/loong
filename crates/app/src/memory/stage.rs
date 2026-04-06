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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    ProfileNote,
    SummaryCheckpoint,
    RecentWindowTurn,
    MemorySystem,
}

impl MemoryProvenanceSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceDocument => "workspace_document",
            Self::ProfileNote => "profile_note",
            Self::SummaryCheckpoint => "summary_checkpoint",
            Self::RecentWindowTurn => "recent_window_turn",
            Self::MemorySystem => "memory_system",
        }
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRetrievalRequest {
    pub session_id: String,
    pub memory_system_id: String,
    pub query: Option<String>,
    pub recall_mode: MemoryRecallMode,
    pub scopes: Vec<MemoryScope>,
    pub budget_items: usize,
    pub allowed_kinds: Vec<DerivedMemoryKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDiagnostics {
    pub family: MemoryStageFamily,
    pub outcome: StageOutcome,
    pub budget_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub fallback_activated: bool,
    pub message: Option<String>,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageEnvelope {
    pub hydrated: HydratedMemoryContext,
    pub retrieval_request: Option<MemoryRetrievalRequest>,
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
                query: None,
                recall_mode: MemoryRecallMode::PromptAssembly,
                scopes: vec![MemoryScope::Session],
                budget_items: 8,
                allowed_kinds: vec![DerivedMemoryKind::Summary],
            }),
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
            diagnostics: vec![],
        };

        let payload = encode_stage_envelope_payload(&envelope);
        let decoded = decode_stage_envelope(&payload).expect("decode stage envelope");
        assert_eq!(decoded.hydrated.diagnostics.system_id, "lucid");
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
