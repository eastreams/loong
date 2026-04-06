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
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRetrievalRequest {
    pub session_id: String,
    pub query: Option<String>,
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
                query: None,
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
