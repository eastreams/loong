use serde::{Deserialize, Serialize};

use crate::RuntimeSnapshotCompactionHygieneState;
use crate::mvp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrendDirection {
    Improving,
    Worsening,
    Steady,
    InsufficientHistory,
}

impl CompactionTrendDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Improving => "improving",
            Self::Worsening => "worsening",
            Self::Steady => "steady",
            Self::InsufficientHistory => "insufficient_history",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrendScope {
    Unavailable,
    Idle,
    RecentSessions,
    RecentSessionsFallback,
    PrimaryLineage,
}

impl CompactionTrendScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Idle => "idle",
            Self::RecentSessions => "recent_sessions",
            Self::RecentSessionsFallback => "recent_sessions_fallback",
            Self::PrimaryLineage => "primary_lineage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionContinuitySource {
    SessionEventsRecent,
}

impl CompactionContinuitySource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionEventsRecent => "session_events_recent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionContinuityHealth {
    Unavailable,
    Idle,
    ScopeLimited,
    InsufficientHistory,
    NoCompactionEvidence,
    Stable,
    Fragile,
    Broken,
}

impl CompactionContinuityHealth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Idle => "idle",
            Self::ScopeLimited => "scope_limited",
            Self::InsufficientHistory => "insufficient_history",
            Self::NoCompactionEvidence => "no_compaction_evidence",
            Self::Stable => "stable",
            Self::Fragile => "fragile",
            Self::Broken => "broken",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionRepairability {
    Unavailable,
    Idle,
    ScopeLimited,
    InsufficientHistory,
    NoCompactionEvidence,
    NotNeeded,
    Retryable,
    ManualInspection,
    Unknown,
}

impl CompactionRepairability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Idle => "idle",
            Self::ScopeLimited => "scope_limited",
            Self::InsufficientHistory => "insufficient_history",
            Self::NoCompactionEvidence => "no_compaction_evidence",
            Self::NotNeeded => "not_needed",
            Self::Retryable => "retryable",
            Self::ManualInspection => "manual_inspection",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionRecoveryPosture {
    Unavailable,
    Idle,
    ScopeLimited,
    InsufficientHistory,
    NoCompactionEvidence,
    Healthy,
    Watch,
    AutoRepairing,
    RetryExhausted,
    ManualLane,
    Fragile,
    Broken,
    Unknown,
}

impl CompactionRecoveryPosture {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Idle => "idle",
            Self::ScopeLimited => "scope_limited",
            Self::InsufficientHistory => "insufficient_history",
            Self::NoCompactionEvidence => "no_compaction_evidence",
            Self::Healthy => "healthy",
            Self::Watch => "watch",
            Self::AutoRepairing => "auto_repairing",
            Self::RetryExhausted => "retry_exhausted",
            Self::ManualLane => "manual_lane",
            Self::Fragile => "fragile",
            Self::Broken => "broken",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionSampleOrder {
    UpdatedAtDesc,
}

impl CompactionSampleOrder {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UpdatedAtDesc => "updated_at_desc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionHygienePosture {
    Unavailable,
    Idle,
    Healthy,
    Attention,
    Degraded,
}

impl CompactionHygienePosture {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Idle => "idle",
            Self::Healthy => "healthy",
            Self::Attention => "attention",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSnapshotCompactionAssessment {
    pub posture: CompactionHygienePosture,
    pub sample_order: CompactionSampleOrder,
    pub trend_scope: CompactionTrendScope,
    pub continuity_source: CompactionContinuitySource,
    pub continuity_health: CompactionContinuityHealth,
    pub continuity_repairability: CompactionRepairability,
    pub recovery_posture: CompactionRecoveryPosture,
    pub reliability_trend: CompactionTrendDirection,
    pub coverage_trend: CompactionTrendDirection,
    pub pressure_trend: CompactionTrendDirection,
}

pub(crate) fn assess_compaction_hygiene(
    state: &RuntimeSnapshotCompactionHygieneState,
) -> RuntimeSnapshotCompactionAssessment {
    let trend_scope = match state.trend_scope() {
        "unavailable" => CompactionTrendScope::Unavailable,
        "idle" => CompactionTrendScope::Idle,
        "recent_sessions" => CompactionTrendScope::RecentSessions,
        "recent_sessions_fallback" => CompactionTrendScope::RecentSessionsFallback,
        "primary_lineage" => CompactionTrendScope::PrimaryLineage,
        _ => CompactionTrendScope::RecentSessions,
    };

    let continuity_health =
        if state.evidence_status == "unavailable" || state.evidence_status == "read_error" {
            CompactionContinuityHealth::Unavailable
        } else if state.sampled_session_count() == 0 {
            CompactionContinuityHealth::Idle
        } else if trend_scope != CompactionTrendScope::PrimaryLineage {
            CompactionContinuityHealth::ScopeLimited
        } else if state.primary_lineage.sampled_session_count < 2 {
            CompactionContinuityHealth::InsufficientHistory
        } else if state.primary_lineage.checkpoint_event_count == 0 {
            CompactionContinuityHealth::NoCompactionEvidence
        } else if state.primary_lineage.checkpoint_failure_streak >= 2 {
            CompactionContinuityHealth::Broken
        } else {
            match state.primary_lineage.latest_compaction_status {
                Some(
                    mvp::conversation::TurnCheckpointProgressStatus::Failed
                    | mvp::conversation::TurnCheckpointProgressStatus::FailedOpen
                    | mvp::conversation::TurnCheckpointProgressStatus::Pending,
                ) => CompactionContinuityHealth::Fragile,
                _ => CompactionContinuityHealth::Stable,
            }
        };

    let continuity_repairability = match continuity_health {
        CompactionContinuityHealth::Unavailable => CompactionRepairability::Unavailable,
        CompactionContinuityHealth::Idle => CompactionRepairability::Idle,
        CompactionContinuityHealth::ScopeLimited => CompactionRepairability::ScopeLimited,
        CompactionContinuityHealth::InsufficientHistory => {
            CompactionRepairability::InsufficientHistory
        }
        CompactionContinuityHealth::NoCompactionEvidence => {
            CompactionRepairability::NoCompactionEvidence
        }
        CompactionContinuityHealth::Stable
        | CompactionContinuityHealth::Fragile
        | CompactionContinuityHealth::Broken => {
            match state.primary_lineage.checkpoint_repair_action {
                Some(mvp::conversation::TurnCheckpointRecoveryAction::None) => {
                    CompactionRepairability::NotNeeded
                }
                Some(mvp::conversation::TurnCheckpointRecoveryAction::InspectManually) => {
                    CompactionRepairability::ManualInspection
                }
                Some(
                    mvp::conversation::TurnCheckpointRecoveryAction::RunAfterTurn
                    | mvp::conversation::TurnCheckpointRecoveryAction::RunCompaction
                    | mvp::conversation::TurnCheckpointRecoveryAction::RunAfterTurnAndCompaction,
                ) => CompactionRepairability::Retryable,
                None => CompactionRepairability::Unknown,
            }
        }
    };

    let recovery_posture = match (continuity_health, continuity_repairability) {
        (CompactionContinuityHealth::Unavailable, _) => CompactionRecoveryPosture::Unavailable,
        (CompactionContinuityHealth::Idle, _) => CompactionRecoveryPosture::Idle,
        (CompactionContinuityHealth::ScopeLimited, _) => CompactionRecoveryPosture::ScopeLimited,
        (CompactionContinuityHealth::InsufficientHistory, _) => {
            CompactionRecoveryPosture::InsufficientHistory
        }
        (CompactionContinuityHealth::NoCompactionEvidence, _) => {
            CompactionRecoveryPosture::NoCompactionEvidence
        }
        (_, CompactionRepairability::ManualInspection) => CompactionRecoveryPosture::ManualLane,
        (CompactionContinuityHealth::Broken, CompactionRepairability::Retryable) => {
            CompactionRecoveryPosture::RetryExhausted
        }
        (CompactionContinuityHealth::Fragile, CompactionRepairability::Retryable) => {
            CompactionRecoveryPosture::AutoRepairing
        }
        (CompactionContinuityHealth::Stable, CompactionRepairability::Retryable) => {
            CompactionRecoveryPosture::Watch
        }
        (CompactionContinuityHealth::Stable, CompactionRepairability::NotNeeded) => {
            CompactionRecoveryPosture::Healthy
        }
        (CompactionContinuityHealth::Fragile, CompactionRepairability::NotNeeded) => {
            CompactionRecoveryPosture::Fragile
        }
        (CompactionContinuityHealth::Broken, CompactionRepairability::NotNeeded) => {
            CompactionRecoveryPosture::Broken
        }
        _ => CompactionRecoveryPosture::Unknown,
    };

    let posture = match state.evidence_status.as_str() {
        "unavailable" | "read_error" => CompactionHygienePosture::Unavailable,
        "idle" | "no_evidence" => CompactionHygienePosture::Idle,
        _ => {
            if state
                .failed_open_rate_milli()
                .is_some_and(|value| value >= 500)
            {
                CompactionHygienePosture::Degraded
            } else if state.evidence_status == "partial" || state.failed_open_session_count() > 0 {
                CompactionHygienePosture::Attention
            } else {
                CompactionHygienePosture::Healthy
            }
        }
    };

    RuntimeSnapshotCompactionAssessment {
        posture,
        sample_order: CompactionSampleOrder::UpdatedAtDesc,
        trend_scope,
        continuity_source: CompactionContinuitySource::SessionEventsRecent,
        continuity_health,
        continuity_repairability,
        recovery_posture,
        reliability_trend: trend_direction_from_label(state.reliability_trend()),
        coverage_trend: trend_direction_from_label(state.coverage_trend()),
        pressure_trend: trend_direction_from_label(state.pressure_trend()),
    }
}

fn trend_direction_from_label(label: &str) -> CompactionTrendDirection {
    match label {
        "improving" => CompactionTrendDirection::Improving,
        "worsening" => CompactionTrendDirection::Worsening,
        "steady" => CompactionTrendDirection::Steady,
        _ => CompactionTrendDirection::InsufficientHistory,
    }
}
