use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::CliResult;

use super::ContextCompactionDiagnostics;
use super::analytics::{
    TurnCheckpointEventSummary,
    TurnCheckpointProgressStatus as AnalyticsTurnCheckpointProgressStatus,
    TurnCheckpointRecoveryAction, TurnCheckpointRepairManualReason, TurnCheckpointRepairPlan,
    TurnCheckpointSessionState, build_turn_checkpoint_repair_plan,
};
use super::context_engine::AssembledConversationContext;
use super::lane_arbiter::ExecutionLane;
use super::persistence::persist_conversation_event;
use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_coordinator::SafeLaneFailureRoute;
use super::turn_engine::TurnResult;
use super::turn_shared::{
    ReplyPersistenceMode, ReplyResolutionMode, ToolDrivenContinuationState, ToolDrivenFollowupKind,
    ToolDrivenReplyPhase,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCheckpointTailRepairStatus {
    NoCheckpoint,
    NotNeeded,
    Repaired,
    ManualRequired,
}

impl TurnCheckpointTailRepairStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCheckpoint => "no_checkpoint",
            Self::NotNeeded => "not_needed",
            Self::Repaired => "repaired",
            Self::ManualRequired => "manual_required",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCheckpointTailRepairReason {
    NoCheckpoint,
    NotNeeded,
    Repaired,
    CheckpointIdentityMissing,
    SafeLaneBackpressureTerminalRequiresManualInspection,
    SafeLaneSessionGovernorTerminalRequiresManualInspection,
    CheckpointPreparationMalformed,
    CheckpointPreparationMismatch,
    CheckpointPreparationFingerprintMismatch,
    CheckpointStateRequiresManualInspection,
    VisibleTurnPairMissing,
    CheckpointIdentityMismatch,
}

impl TurnCheckpointTailRepairReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCheckpoint => "no_checkpoint",
            Self::NotNeeded => "not_needed",
            Self::Repaired => "repaired",
            Self::CheckpointIdentityMissing => "checkpoint_identity_missing",
            Self::SafeLaneBackpressureTerminalRequiresManualInspection => {
                "safe_lane_backpressure_terminal_requires_manual_inspection"
            }
            Self::SafeLaneSessionGovernorTerminalRequiresManualInspection => {
                "safe_lane_session_governor_terminal_requires_manual_inspection"
            }
            Self::CheckpointPreparationMalformed => "checkpoint_preparation_malformed",
            Self::CheckpointPreparationMismatch => "checkpoint_preparation_mismatch",
            Self::CheckpointPreparationFingerprintMismatch => {
                "checkpoint_preparation_fingerprint_mismatch"
            }
            Self::CheckpointStateRequiresManualInspection => {
                "checkpoint_state_requires_manual_inspection"
            }
            Self::VisibleTurnPairMissing => "visible_turn_pair_missing",
            Self::CheckpointIdentityMismatch => "checkpoint_identity_mismatch",
        }
    }
}

impl From<TurnCheckpointRepairManualReason> for TurnCheckpointTailRepairReason {
    fn from(reason: TurnCheckpointRepairManualReason) -> Self {
        match reason {
            TurnCheckpointRepairManualReason::CheckpointIdentityMissing => {
                Self::CheckpointIdentityMissing
            }
            TurnCheckpointRepairManualReason::SafeLaneBackpressureTerminalRequiresManualInspection => {
                Self::SafeLaneBackpressureTerminalRequiresManualInspection
            }
            TurnCheckpointRepairManualReason::SafeLaneSessionGovernorTerminalRequiresManualInspection => {
                Self::SafeLaneSessionGovernorTerminalRequiresManualInspection
            }
            TurnCheckpointRepairManualReason::CheckpointStateRequiresManualInspection => {
                Self::CheckpointStateRequiresManualInspection
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCheckpointTailRepairOutcome {
    status: TurnCheckpointTailRepairStatus,
    action: TurnCheckpointRecoveryAction,
    source: Option<TurnCheckpointTailRepairSource>,
    reason: TurnCheckpointTailRepairReason,
    session_state: TurnCheckpointSessionState,
    checkpoint_events: u32,
    after_turn_status: Option<&'static str>,
    compaction_status: Option<&'static str>,
}

impl TurnCheckpointTailRepairOutcome {
    pub(super) fn no_checkpoint() -> Self {
        Self {
            status: TurnCheckpointTailRepairStatus::NoCheckpoint,
            action: TurnCheckpointRecoveryAction::None,
            source: None,
            reason: TurnCheckpointTailRepairReason::NoCheckpoint,
            session_state: TurnCheckpointSessionState::NotDurable,
            checkpoint_events: 0,
            after_turn_status: None,
            compaction_status: None,
        }
    }

    pub(crate) fn from_summary(
        status: TurnCheckpointTailRepairStatus,
        action: TurnCheckpointRecoveryAction,
        source: Option<TurnCheckpointTailRepairSource>,
        reason: TurnCheckpointTailRepairReason,
        summary: &TurnCheckpointEventSummary,
    ) -> Self {
        Self {
            status,
            action,
            source,
            reason,
            session_state: summary.session_state,
            checkpoint_events: summary.checkpoint_events,
            after_turn_status: summary
                .latest_after_turn
                .map(format_analytics_turn_checkpoint_progress_status),
            compaction_status: summary
                .latest_compaction
                .map(format_analytics_turn_checkpoint_progress_status),
        }
    }

    pub(super) fn repaired(
        action: TurnCheckpointRecoveryAction,
        summary: &TurnCheckpointEventSummary,
        after_turn_status: TurnCheckpointProgressStatus,
        compaction_status: TurnCheckpointProgressStatus,
    ) -> Self {
        Self {
            status: TurnCheckpointTailRepairStatus::Repaired,
            action,
            source: Some(TurnCheckpointTailRepairSource::Runtime),
            reason: TurnCheckpointTailRepairReason::Repaired,
            session_state: summary.session_state,
            checkpoint_events: summary.checkpoint_events,
            after_turn_status: Some(format_turn_checkpoint_progress_status(after_turn_status)),
            compaction_status: Some(format_turn_checkpoint_progress_status(compaction_status)),
        }
    }

    pub fn status(&self) -> TurnCheckpointTailRepairStatus {
        self.status
    }

    pub fn action(&self) -> TurnCheckpointRecoveryAction {
        self.action
    }

    pub fn source(&self) -> Option<TurnCheckpointTailRepairSource> {
        self.source
    }

    pub fn reason(&self) -> TurnCheckpointTailRepairReason {
        self.reason
    }

    pub fn session_state(&self) -> TurnCheckpointSessionState {
        self.session_state
    }

    pub fn checkpoint_events(&self) -> u32 {
        self.checkpoint_events
    }

    pub fn after_turn_status(&self) -> Option<&'static str> {
        self.after_turn_status
    }

    pub fn compaction_status(&self) -> Option<&'static str> {
        self.compaction_status
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCheckpointTailRepairSource {
    Summary,
    Runtime,
}

impl TurnCheckpointTailRepairSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnCheckpointTailRepairRuntimeProbe {
    action: TurnCheckpointRecoveryAction,
    source: TurnCheckpointTailRepairSource,
    reason: TurnCheckpointTailRepairReason,
}

impl TurnCheckpointTailRepairRuntimeProbe {
    pub(crate) fn new(
        action: TurnCheckpointRecoveryAction,
        source: TurnCheckpointTailRepairSource,
        reason: TurnCheckpointTailRepairReason,
    ) -> Self {
        Self {
            action,
            source,
            reason,
        }
    }

    pub fn action(&self) -> TurnCheckpointRecoveryAction {
        self.action
    }

    pub fn source(&self) -> TurnCheckpointTailRepairSource {
        self.source
    }

    pub fn reason(&self) -> TurnCheckpointTailRepairReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TurnCheckpointRecoveryAssessment {
    action: TurnCheckpointRecoveryAction,
    source: TurnCheckpointTailRepairSource,
    reason: Option<TurnCheckpointTailRepairReason>,
}

impl TurnCheckpointRecoveryAssessment {
    pub(crate) fn from_summary(summary: &TurnCheckpointEventSummary) -> Self {
        let repair_plan = build_turn_checkpoint_repair_plan(summary);
        let action = repair_plan.action();
        let reason = matches!(action, TurnCheckpointRecoveryAction::InspectManually).then(|| {
            repair_plan
                .manual_reason()
                .map(TurnCheckpointTailRepairReason::from)
                .unwrap_or(TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection)
        });
        Self {
            action,
            source: TurnCheckpointTailRepairSource::Summary,
            reason,
        }
    }

    pub fn action(self) -> TurnCheckpointRecoveryAction {
        self.action
    }

    pub fn source(self) -> TurnCheckpointTailRepairSource {
        self.source
    }

    pub fn reason(self) -> Option<TurnCheckpointTailRepairReason> {
        self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnCheckpointDiagnostics {
    summary: TurnCheckpointEventSummary,
    recovery: TurnCheckpointRecoveryAssessment,
    runtime_probe: Option<TurnCheckpointTailRepairRuntimeProbe>,
}

impl TurnCheckpointDiagnostics {
    pub(crate) fn new(
        summary: TurnCheckpointEventSummary,
        recovery: TurnCheckpointRecoveryAssessment,
        runtime_probe: Option<TurnCheckpointTailRepairRuntimeProbe>,
    ) -> Self {
        Self {
            summary,
            recovery,
            runtime_probe,
        }
    }

    pub fn summary(&self) -> &TurnCheckpointEventSummary {
        &self.summary
    }

    pub fn recovery(&self) -> TurnCheckpointRecoveryAssessment {
        self.recovery
    }

    pub fn runtime_probe(&self) -> Option<&TurnCheckpointTailRepairRuntimeProbe> {
        self.runtime_probe.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TurnCheckpointSnapshot {
    pub(super) identity: Option<TurnCheckpointIdentity>,
    pub(super) preparation: TurnPreparationSnapshot,
    pub(super) request: TurnCheckpointRequest,
    pub(super) lane: Option<TurnLaneExecutionSnapshot>,
    pub(super) reply: Option<TurnReplyCheckpoint>,
    pub(super) finalization: TurnFinalizationCheckpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TurnCheckpointIdentity {
    pub(super) user_input_sha256: String,
    pub(super) assistant_reply_sha256: String,
    pub(super) user_input_chars: usize,
    pub(super) assistant_reply_chars: usize,
}

impl TurnCheckpointIdentity {
    pub(super) fn from_turn(user_input: &str, assistant_reply: &str) -> Self {
        Self {
            user_input_sha256: sha256_hex(user_input),
            assistant_reply_sha256: sha256_hex(assistant_reply),
            user_input_chars: user_input.chars().count(),
            assistant_reply_chars: assistant_reply.chars().count(),
        }
    }

    fn matches_turn(&self, user_input: &str, assistant_reply: &str) -> bool {
        self.user_input_chars == user_input.chars().count()
            && self.assistant_reply_chars == assistant_reply.chars().count()
            && self.user_input_sha256 == sha256_hex(user_input)
            && self.assistant_reply_sha256 == sha256_hex(assistant_reply)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TurnPreparationSnapshot {
    pub(super) lane: ExecutionLane,
    pub(super) max_tool_steps: usize,
    pub(super) raw_tool_output_requested: bool,
    pub(super) context_message_count: usize,
    pub(super) context_fingerprint_sha256: String,
    pub(super) estimated_tokens: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum TurnCheckpointRequest {
    Continue { tool_intents: usize },
    FinalizeInlineProviderError,
    ReturnError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TurnLaneExecutionSnapshot {
    pub(super) lane: ExecutionLane,
    pub(super) had_tool_intents: bool,
    pub(super) tool_request_summary: Option<String>,
    pub(super) raw_tool_output_requested: bool,
    pub(super) result_kind: TurnCheckpointResultKind,
    pub(super) safe_lane_terminal_route: Option<SafeLaneFailureRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TurnCheckpointResultKind {
    FinalText,
    Streaming,
    NeedsApproval,
    ToolDenied,
    ToolError,
    ProviderError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TurnReplyCheckpoint {
    pub(super) decision: ReplyResolutionMode,
    pub(super) followup_kind: Option<ToolDrivenFollowupKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) continuation_state: Option<ToolDrivenContinuationState>,
}

impl TurnReplyCheckpoint {
    pub(super) fn from_phase(phase: &ToolDrivenReplyPhase) -> Self {
        Self::from_phase_with_continuation_state(phase, None)
    }

    pub(super) fn from_phase_with_continuation_state(
        phase: &ToolDrivenReplyPhase,
        continuation_state: Option<ToolDrivenContinuationState>,
    ) -> Self {
        Self {
            decision: phase.resolution_mode(),
            followup_kind: phase.followup_kind(),
            continuation_state,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum TurnFinalizationCheckpoint {
    PersistReply {
        persistence_mode: ReplyPersistenceMode,
        runs_after_turn: bool,
        attempts_context_compaction: bool,
    },
    ReturnError,
}

impl TurnFinalizationCheckpoint {
    pub(super) fn persist_reply(persistence_mode: ReplyPersistenceMode) -> Self {
        Self::PersistReply {
            persistence_mode,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    }

    pub(super) fn persistence_mode(self) -> Option<ReplyPersistenceMode> {
        match self {
            Self::PersistReply {
                persistence_mode, ..
            } => Some(persistence_mode),
            Self::ReturnError => None,
        }
    }

    pub(super) fn runs_after_turn(self) -> bool {
        match self {
            Self::PersistReply {
                runs_after_turn, ..
            } => runs_after_turn,
            Self::ReturnError => false,
        }
    }

    pub(super) fn attempts_context_compaction(self) -> bool {
        match self {
            Self::PersistReply {
                attempts_context_compaction,
                ..
            } => attempts_context_compaction,
            Self::ReturnError => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TurnCheckpointStage {
    PostPersist,
    Finalized,
    FinalizationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TurnCheckpointProgressStatus {
    Pending,
    Skipped,
    Completed,
    Failed,
    FailedOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct TurnCheckpointFinalizationProgress {
    pub(super) after_turn: TurnCheckpointProgressStatus,
    pub(super) compaction: TurnCheckpointProgressStatus,
}

impl TurnCheckpointFinalizationProgress {
    pub(super) fn pending(checkpoint: &TurnCheckpointSnapshot) -> Self {
        Self {
            after_turn: if checkpoint.finalization.runs_after_turn() {
                TurnCheckpointProgressStatus::Pending
            } else {
                TurnCheckpointProgressStatus::Skipped
            },
            compaction: if checkpoint.finalization.attempts_context_compaction() {
                TurnCheckpointProgressStatus::Pending
            } else {
                TurnCheckpointProgressStatus::Skipped
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextCompactionOutcome {
    Skipped,
    Completed,
    FailedOpen,
}

impl ContextCompactionOutcome {
    pub(super) fn checkpoint_status(self) -> TurnCheckpointProgressStatus {
        match self {
            Self::Skipped => TurnCheckpointProgressStatus::Skipped,
            Self::Completed => TurnCheckpointProgressStatus::Completed,
            Self::FailedOpen => TurnCheckpointProgressStatus::FailedOpen,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TurnCheckpointFailureStep {
    AfterTurn,
    Compaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct TurnCheckpointFailure {
    pub(super) step: TurnCheckpointFailureStep,
    pub(super) error: String,
}

pub(super) fn turn_checkpoint_result_kind(result: &TurnResult) -> TurnCheckpointResultKind {
    match result {
        TurnResult::FinalText(_) => TurnCheckpointResultKind::FinalText,
        TurnResult::StreamingText(_) | TurnResult::StreamingDone(_) => {
            TurnCheckpointResultKind::Streaming
        }
        TurnResult::NeedsApproval(_) => TurnCheckpointResultKind::NeedsApproval,
        TurnResult::ToolDenied(_) => TurnCheckpointResultKind::ToolDenied,
        TurnResult::ToolError(_) => TurnCheckpointResultKind::ToolError,
        TurnResult::ProviderError(_) => TurnCheckpointResultKind::ProviderError,
    }
}

fn format_turn_checkpoint_progress_status(status: TurnCheckpointProgressStatus) -> &'static str {
    match status {
        TurnCheckpointProgressStatus::Pending => "pending",
        TurnCheckpointProgressStatus::Skipped => "skipped",
        TurnCheckpointProgressStatus::Completed => "completed",
        TurnCheckpointProgressStatus::Failed => "failed",
        TurnCheckpointProgressStatus::FailedOpen => "failed_open",
    }
}

fn format_analytics_turn_checkpoint_progress_status(
    status: AnalyticsTurnCheckpointProgressStatus,
) -> &'static str {
    let canonical_status = TurnCheckpointProgressStatus::from(status);

    format_turn_checkpoint_progress_status(canonical_status)
}

pub(super) async fn persist_turn_checkpoint_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    checkpoint: &TurnCheckpointSnapshot,
    stage: TurnCheckpointStage,
    progress: TurnCheckpointFinalizationProgress,
    failure: Option<TurnCheckpointFailure>,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    let checkpoint = serde_json::to_value(checkpoint)
        .map_err(|error| format!("serialize turn checkpoint failed: {error}"))?;
    persist_turn_checkpoint_event_value_with_compaction_diagnostics(
        runtime,
        session_id,
        &checkpoint,
        stage,
        progress,
        failure,
        None,
        binding,
    )
    .await
}

pub(super) async fn persist_turn_checkpoint_event_with_compaction_diagnostics<
    R: ConversationRuntime + ?Sized,
>(
    runtime: &R,
    session_id: &str,
    checkpoint: &TurnCheckpointSnapshot,
    stage: TurnCheckpointStage,
    progress: TurnCheckpointFinalizationProgress,
    failure: Option<TurnCheckpointFailure>,
    compaction_diagnostics: Option<&ContextCompactionDiagnostics>,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    let checkpoint = serde_json::to_value(checkpoint)
        .map_err(|error| format!("serialize turn checkpoint failed: {error}"))?;
    persist_turn_checkpoint_event_value_with_compaction_diagnostics(
        runtime,
        session_id,
        &checkpoint,
        stage,
        progress,
        failure,
        compaction_diagnostics,
        binding,
    )
    .await
}

pub(super) async fn persist_turn_checkpoint_event_value_with_compaction_diagnostics<
    R: ConversationRuntime + ?Sized,
>(
    runtime: &R,
    session_id: &str,
    checkpoint: &Value,
    stage: TurnCheckpointStage,
    progress: TurnCheckpointFinalizationProgress,
    failure: Option<TurnCheckpointFailure>,
    compaction_diagnostics: Option<&ContextCompactionDiagnostics>,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_turn_checkpoint_event_payload(
        runtime,
        session_id,
        checkpoint,
        stage,
        progress,
        failure,
        compaction_diagnostics,
        binding,
    )
    .await
}

pub(super) async fn persist_turn_checkpoint_event_value<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    checkpoint: &Value,
    stage: TurnCheckpointStage,
    progress: TurnCheckpointFinalizationProgress,
    failure: Option<TurnCheckpointFailure>,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_turn_checkpoint_event_value_with_compaction_diagnostics(
        runtime, session_id, checkpoint, stage, progress, failure, None, binding,
    )
    .await
}

async fn persist_turn_checkpoint_event_payload<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    checkpoint: &Value,
    stage: TurnCheckpointStage,
    progress: TurnCheckpointFinalizationProgress,
    failure: Option<TurnCheckpointFailure>,
    compaction_diagnostics: Option<&ContextCompactionDiagnostics>,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_conversation_event(
        runtime,
        session_id,
        "turn_checkpoint",
        json!({
            "schema_version": 1,
            "stage": stage,
            "checkpoint": checkpoint,
            "finalization_progress": progress,
            "failure": failure,
            "compaction_diagnostics": compaction_diagnostics,
        }),
        binding,
    )
    .await
}

fn recover_latest_turn_pair(messages: &[Value]) -> Option<(usize, String, String)> {
    let assistant_index = messages.iter().rposition(|message| {
        message.get("role").and_then(Value::as_str) == Some("assistant")
            && message.get("content").and_then(Value::as_str).is_some()
    })?;
    let assistant_reply = messages
        .get(assistant_index)?
        .get("content")
        .and_then(Value::as_str)?
        .to_owned();
    let user_input = messages
        .get(..assistant_index)?
        .iter()
        .rposition(|message| {
            message.get("role").and_then(Value::as_str) == Some("user")
                && message.get("content").and_then(Value::as_str).is_some()
        })
        .and_then(|index| {
            messages
                .get(index)
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)?;
    Some((assistant_index, user_input, assistant_reply))
}

fn load_turn_checkpoint_identity(checkpoint: &Value) -> Option<TurnCheckpointIdentity> {
    checkpoint
        .get("identity")
        .cloned()
        .and_then(|identity| serde_json::from_value(identity).ok())
}

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

pub(super) fn checkpoint_context_fingerprint_sha256(messages: &[Value]) -> String {
    let serialized = Value::Array(messages.to_vec()).to_string();
    hex::encode(Sha256::digest(serialized.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
struct TurnCheckpointRepairPreparation {
    #[serde(default)]
    context_message_count: Option<usize>,
    #[serde(default)]
    context_fingerprint_sha256: Option<String>,
    #[serde(default)]
    estimated_tokens: Option<usize>,
}

fn load_turn_checkpoint_repair_preparation(
    checkpoint: &Value,
) -> Result<Option<TurnCheckpointRepairPreparation>, TurnCheckpointTailRepairReason> {
    let Some(preparation) = checkpoint.get("preparation") else {
        return Ok(None);
    };
    serde_json::from_value(preparation.clone())
        .map(Some)
        .map_err(|_error| TurnCheckpointTailRepairReason::CheckpointPreparationMalformed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TurnCheckpointRepairResumeInput {
    user_input: String,
    assistant_reply: String,
    messages: Vec<Value>,
    estimated_tokens: Option<usize>,
}

impl TurnCheckpointRepairResumeInput {
    pub(super) fn from_assembled_context(
        assembled: AssembledConversationContext,
        checkpoint: &Value,
    ) -> Result<Self, TurnCheckpointTailRepairReason> {
        let repair_preparation = load_turn_checkpoint_repair_preparation(checkpoint)?;
        let messages = assembled.messages;
        let estimated_tokens = assembled.estimated_tokens;
        let Some((assistant_index, user_input, assistant_reply)) =
            recover_latest_turn_pair(&messages)
        else {
            return Err(TurnCheckpointTailRepairReason::VisibleTurnPairMissing);
        };
        let assistant_tail_index = assistant_index + 1;
        if assistant_tail_index != messages.len() {
            return Err(TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection);
        }
        let Some(pre_assistant_messages) = messages.get(..assistant_index) else {
            return Err(TurnCheckpointTailRepairReason::VisibleTurnPairMissing);
        };
        let Some(identity) = load_turn_checkpoint_identity(checkpoint) else {
            return Err(TurnCheckpointTailRepairReason::CheckpointIdentityMissing);
        };
        if !identity.matches_turn(&user_input, &assistant_reply) {
            return Err(TurnCheckpointTailRepairReason::CheckpointIdentityMismatch);
        }
        let expected_context_message_count = repair_preparation
            .as_ref()
            .and_then(|preparation| preparation.context_message_count);
        if let Some(expected_context_message_count) = expected_context_message_count
            && pre_assistant_messages.len() != expected_context_message_count
        {
            return Err(TurnCheckpointTailRepairReason::CheckpointPreparationMismatch);
        }
        let expected_context_fingerprint_sha256 = repair_preparation
            .as_ref()
            .and_then(|preparation| preparation.context_fingerprint_sha256.as_deref());
        let actual_context_fingerprint_sha256 =
            checkpoint_context_fingerprint_sha256(pre_assistant_messages);
        if let Some(expected_context_fingerprint_sha256) = expected_context_fingerprint_sha256
            && actual_context_fingerprint_sha256 != expected_context_fingerprint_sha256
        {
            return Err(TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch);
        }

        Ok(Self {
            user_input,
            assistant_reply,
            messages,
            estimated_tokens: repair_preparation
                .and_then(|preparation| preparation.estimated_tokens)
                .or(estimated_tokens),
        })
    }

    pub(super) fn user_input(&self) -> &str {
        self.user_input.as_str()
    }

    pub(super) fn assistant_reply(&self) -> &str {
        self.assistant_reply.as_str()
    }

    pub(super) fn messages(&self) -> &[Value] {
        &self.messages
    }

    pub(super) fn estimated_tokens(&self) -> Option<usize> {
        self.estimated_tokens
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TurnCheckpointTailRuntimeEligibility {
    NotNeeded {
        action: TurnCheckpointRecoveryAction,
        reason: TurnCheckpointTailRepairReason,
    },
    Manual {
        action: TurnCheckpointRecoveryAction,
        reason: TurnCheckpointTailRepairReason,
        source: TurnCheckpointTailRepairSource,
    },
    Runnable {
        action: TurnCheckpointRecoveryAction,
        plan: TurnCheckpointRepairPlan,
        resume_input: TurnCheckpointRepairResumeInput,
    },
}

pub(super) fn restore_analytics_turn_checkpoint_progress_status(
    status: AnalyticsTurnCheckpointProgressStatus,
) -> TurnCheckpointProgressStatus {
    TurnCheckpointProgressStatus::from(status)
}

impl From<AnalyticsTurnCheckpointProgressStatus> for TurnCheckpointProgressStatus {
    fn from(status: AnalyticsTurnCheckpointProgressStatus) -> Self {
        match status {
            AnalyticsTurnCheckpointProgressStatus::Pending => TurnCheckpointProgressStatus::Pending,
            AnalyticsTurnCheckpointProgressStatus::Skipped => TurnCheckpointProgressStatus::Skipped,
            AnalyticsTurnCheckpointProgressStatus::Completed => {
                TurnCheckpointProgressStatus::Completed
            }
            AnalyticsTurnCheckpointProgressStatus::Failed => TurnCheckpointProgressStatus::Failed,
            AnalyticsTurnCheckpointProgressStatus::FailedOpen => {
                TurnCheckpointProgressStatus::FailedOpen
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        TurnCheckpointIdentity, TurnCheckpointRepairResumeInput, TurnCheckpointTailRepairReason,
        checkpoint_context_fingerprint_sha256,
    };
    use crate::conversation::context_engine::AssembledConversationContext;

    fn checkpoint_with_preparation(
        user_input: &str,
        assistant_reply: &str,
        pre_assistant_messages: &[Value],
        estimated_tokens: Option<usize>,
    ) -> Value {
        let identity = TurnCheckpointIdentity::from_turn(user_input, assistant_reply);
        let identity = serde_json::to_value(identity).expect("identity should serialize");
        let context_message_count = pre_assistant_messages.len();
        let context_fingerprint_sha256 =
            checkpoint_context_fingerprint_sha256(pre_assistant_messages);

        json!({
            "identity": identity,
            "preparation": {
                "context_message_count": context_message_count,
                "context_fingerprint_sha256": context_fingerprint_sha256,
                "estimated_tokens": estimated_tokens,
            }
        })
    }

    #[test]
    fn repair_resume_input_accepts_matching_tail_with_preparation() {
        let pre_assistant_messages = vec![
            json!({
                "role": "system",
                "content": "sys"
            }),
            json!({
                "role": "user",
                "content": "hello"
            }),
        ];
        let mut messages = pre_assistant_messages.clone();
        messages.push(json!({
            "role": "assistant",
            "content": "world"
        }));
        let checkpoint =
            checkpoint_with_preparation("hello", "world", &pre_assistant_messages, Some(7));
        let assembled = AssembledConversationContext {
            messages,
            artifacts: Vec::new(),
            estimated_tokens: Some(9),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        };
        let resume_input =
            TurnCheckpointRepairResumeInput::from_assembled_context(assembled, &checkpoint)
                .expect("matching checkpoint should resume");

        assert_eq!(resume_input.user_input(), "hello");
        assert_eq!(resume_input.assistant_reply(), "world");
        assert_eq!(resume_input.estimated_tokens(), Some(7));
    }

    #[test]
    fn repair_resume_input_accepts_matching_tail_without_preparation() {
        let messages = vec![
            json!({
                "role": "system",
                "content": "sys"
            }),
            json!({
                "role": "user",
                "content": "hello"
            }),
            json!({
                "role": "assistant",
                "content": "world"
            }),
        ];
        let identity = TurnCheckpointIdentity::from_turn("hello", "world");
        let identity = serde_json::to_value(identity).expect("identity should serialize");
        let checkpoint = json!({
            "identity": identity,
        });
        let assembled = AssembledConversationContext {
            messages,
            artifacts: Vec::new(),
            estimated_tokens: Some(9),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        };
        let resume_input =
            TurnCheckpointRepairResumeInput::from_assembled_context(assembled, &checkpoint)
                .expect("legacy checkpoints without preparation should remain repairable");

        assert_eq!(resume_input.user_input(), "hello");
        assert_eq!(resume_input.assistant_reply(), "world");
        assert_eq!(resume_input.estimated_tokens(), Some(9));
    }

    #[test]
    fn repair_resume_input_requires_assistant_to_be_the_tail_message() {
        let messages = vec![
            json!({
                "role": "system",
                "content": "sys"
            }),
            json!({
                "role": "user",
                "content": "hello"
            }),
        ];
        let mut messages = messages;
        messages.push(json!({
            "role": "assistant",
            "content": "world"
        }));
        messages.push(json!({
            "role": "tool",
            "content": "trailing"
        }));
        let identity = TurnCheckpointIdentity::from_turn("hello", "world");
        let identity = serde_json::to_value(identity).expect("identity should serialize");
        let checkpoint = json!({
            "identity": identity,
        });
        let assembled = AssembledConversationContext {
            messages,
            artifacts: Vec::new(),
            estimated_tokens: Some(9),
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        };
        let error = TurnCheckpointRepairResumeInput::from_assembled_context(assembled, &checkpoint)
            .expect_err("trailing messages should require manual inspection");

        assert_eq!(
            error,
            TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection
        );
    }
}
