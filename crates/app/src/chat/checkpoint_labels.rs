use super::*;

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_stage(stage: Option<TurnCheckpointStage>) -> &'static str {
    match stage {
        Some(TurnCheckpointStage::PostPersist) => "post_persist",
        Some(TurnCheckpointStage::Finalized) => "finalized",
        Some(TurnCheckpointStage::FinalizationFailed) => "finalization_failed",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_progress(status: Option<TurnCheckpointProgressStatus>) -> &'static str {
    match status {
        Some(TurnCheckpointProgressStatus::Pending) => "pending",
        Some(TurnCheckpointProgressStatus::Skipped) => "skipped",
        Some(TurnCheckpointProgressStatus::Completed) => "completed",
        Some(TurnCheckpointProgressStatus::Failed) => "failed",
        Some(TurnCheckpointProgressStatus::FailedOpen) => "failed_open",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn format_turn_checkpoint_failure_step(
    step: Option<TurnCheckpointFailureStep>,
) -> &'static str {
    match step {
        Some(TurnCheckpointFailureStep::AfterTurn) => "after_turn",
        Some(TurnCheckpointFailureStep::Compaction) => "compaction",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_identity_presence(identity_present: Option<bool>) -> &'static str {
    match identity_present {
        Some(true) => "present",
        Some(false) => "missing",
        None => "-",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_session_state(state: TurnCheckpointSessionState) -> &'static str {
    match state {
        TurnCheckpointSessionState::NotDurable => "not_durable",
        TurnCheckpointSessionState::PendingFinalization => "pending_finalization",
        TurnCheckpointSessionState::Finalized => "finalized",
        TurnCheckpointSessionState::FinalizationFailed => "finalization_failed",
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_recovery_action(action: TurnCheckpointRecoveryAction) -> &'static str {
    action.as_str()
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn format_turn_checkpoint_recovery_reason(
    reason: Option<TurnCheckpointTailRepairReason>,
) -> &'static str {
    reason
        .map(TurnCheckpointTailRepairReason::as_str)
        .unwrap_or("-")
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TurnCheckpointRecoveryRenderLabels {
    pub(super) action: &'static str,
    pub(super) source: &'static str,
    pub(super) reason: &'static str,
}

#[cfg(any(test, feature = "memory-sqlite"))]
impl TurnCheckpointRecoveryRenderLabels {
    pub(super) fn from_assessment(assessment: TurnCheckpointRecoveryAssessment) -> Self {
        Self {
            action: format_turn_checkpoint_recovery_action(assessment.action()),
            source: assessment.source().as_str(),
            reason: format_turn_checkpoint_recovery_reason(assessment.reason()),
        }
    }

    #[cfg(test)]
    pub(super) fn from_outcome(outcome: &TurnCheckpointTailRepairOutcome) -> Self {
        Self {
            action: outcome.action().as_str(),
            source: outcome.source().map(|value| value.as_str()).unwrap_or("-"),
            reason: outcome.reason().as_str(),
        }
    }

    #[cfg(test)]
    pub(super) fn from_probe(probe: &TurnCheckpointTailRepairRuntimeProbe) -> Self {
        Self {
            action: probe.action().as_str(),
            source: probe.source().as_str(),
            reason: probe.reason().as_str(),
        }
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TurnCheckpointSummaryRenderLabels<'a> {
    pub(super) session_state: &'static str,
    pub(super) stage: &'static str,
    pub(super) after_turn: &'static str,
    pub(super) compaction: &'static str,
    pub(super) lane: &'a str,
    pub(super) result_kind: &'a str,
    pub(super) persistence_mode: &'a str,
    pub(super) safe_lane_route_decision: &'static str,
    pub(super) safe_lane_route_reason: &'static str,
    pub(super) safe_lane_route_source: &'static str,
    pub(super) identity: &'static str,
}

#[cfg(any(test, feature = "memory-sqlite"))]
impl<'a> TurnCheckpointSummaryRenderLabels<'a> {
    pub(super) fn from_summary(summary: &'a TurnCheckpointEventSummary) -> Self {
        let (safe_lane_route_decision, safe_lane_route_reason, safe_lane_route_source) =
            summary.latest_safe_lane_route_labels_or_default();
        Self {
            session_state: format_turn_checkpoint_session_state(summary.session_state),
            stage: format_turn_checkpoint_stage(summary.latest_stage),
            after_turn: format_turn_checkpoint_progress(summary.latest_after_turn),
            compaction: format_turn_checkpoint_progress(summary.latest_compaction),
            lane: summary.latest_lane.as_deref().unwrap_or("-"),
            result_kind: summary.latest_result_kind.as_deref().unwrap_or("-"),
            persistence_mode: summary.latest_persistence_mode.as_deref().unwrap_or("-"),
            safe_lane_route_decision,
            safe_lane_route_reason,
            safe_lane_route_source,
            identity: format_turn_checkpoint_identity_presence(summary.latest_identity_present),
        }
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TurnCheckpointDurabilityRenderLabels {
    pub(super) checkpoint_durable: u8,
    pub(super) reply_durable: u8,
    pub(super) durability: &'static str,
}

#[cfg(any(test, feature = "memory-sqlite"))]
impl TurnCheckpointDurabilityRenderLabels {
    pub(super) fn from_summary(summary: &TurnCheckpointEventSummary) -> Self {
        let checkpoint_durable = u8::from(summary.checkpoint_durable);
        let reply_durable = u8::from(summary.reply_durable);
        let durability = if checkpoint_durable == 0 {
            "not_durable"
        } else if reply_durable == 1 {
            "reply"
        } else {
            "checkpoint_only"
        };
        Self {
            checkpoint_durable,
            reply_durable,
            durability,
        }
    }
}
