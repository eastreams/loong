use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "memory-sqlite")]
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use futures_util::FutureExt;
use loong_contracts::{AuditEventKind, ExecutionPlane, PlaneTier};
use serde::Serialize;
use serde_json::{Value, json};
#[cfg(feature = "memory-sqlite")]
use tokio::runtime::Handle;
use tokio::sync::Mutex;
#[cfg(feature = "memory-sqlite")]
use tokio::time::{Duration, Instant, timeout};

#[path = "turn_coordinator/app_tools.rs"]
mod app_tools;
#[path = "turn_coordinator/checkpoint_api.rs"]
mod checkpoint_api;
#[path = "turn_coordinator/checkpoint_tail.rs"]
mod checkpoint_tail;
#[path = "turn_coordinator/compact.rs"]
mod compact;
#[path = "turn_coordinator/control_turn.rs"]
mod control_turn;
#[path = "turn_coordinator/delegate.rs"]
mod delegate;
#[path = "turn_coordinator/discovery.rs"]
mod discovery;
#[path = "turn_coordinator/entry.rs"]
mod entry;
#[path = "turn_coordinator/finalize.rs"]
mod finalize;
#[path = "turn_coordinator/flow.rs"]
mod flow;
#[path = "turn_coordinator/lane.rs"]
mod lane;
#[path = "turn_coordinator/observer.rs"]
mod observer;
#[path = "turn_coordinator/outcome.rs"]
mod outcome;
#[path = "turn_coordinator/pending_approval.rs"]
mod pending_approval;
#[path = "turn_coordinator/reply.rs"]
mod reply;
#[path = "turn_coordinator/safe_lane_events.rs"]
mod safe_lane_events;
#[path = "turn_coordinator/safe_lane_execution.rs"]
mod safe_lane_execution;
#[path = "turn_coordinator/safe_lane_governor.rs"]
mod safe_lane_governor;
#[path = "turn_coordinator/safe_lane_routing.rs"]
mod safe_lane_routing;
#[path = "turn_coordinator/safe_lane_state.rs"]
mod safe_lane_state;
#[path = "turn_coordinator/setup.rs"]
mod setup;
#[path = "turn_coordinator/skill_activation.rs"]
mod skill_activation;
#[path = "turn_coordinator/state.rs"]
mod state;
#[path = "turn_coordinator/support.rs"]
mod support;

use crate::CliResult;
#[cfg(test)]
use crate::KernelContext;
use crate::acp::{
    AcpConversationTurnEntryDecision, AcpConversationTurnOptions,
    consume_finalized_acp_conversation_turn, evaluate_acp_conversation_turn_entry_for_address,
    execute_acp_conversation_turn_for_address,
};
#[cfg(feature = "memory-sqlite")]
use crate::operator::delegate_runtime::{
    DelegateChildExecutionPolicy, build_delegate_child_lifecycle_seed,
};
use crate::runtime_self_continuity;
use crate::session::store::{self, SessionStoreConfig};
#[cfg(feature = "memory-sqlite")]
use crate::task_progress::{
    TASK_PROGRESS_EVENT_KIND, TaskActiveHandleRecord, TaskProgressRecord, TaskProgressStatus,
    TaskResumeRecipeRecord, TaskVerificationState, resolve_canonical_task_id_for_session,
    task_progress_event_payload, unix_ts_now,
};

use self::app_tools::CoordinatorAppToolDispatcher;
use self::checkpoint_tail::{
    probe_turn_checkpoint_tail_runtime_gate_entry,
    probe_turn_checkpoint_tail_runtime_gate_entry_with_limit, repair_turn_checkpoint_tail_entry,
};
use self::compact::{
    analytics_turn_checkpoint_progress_status, effective_runtime_self_continuity_for_session,
    ensure_session_exists_for_runtime_self_continuity, estimate_tokens, maybe_compact_context,
};
#[cfg(feature = "memory-sqlite")]
pub(crate) use self::delegate::run_started_delegate_child_turn_with_runtime;
pub use self::delegate::spawn_background_delegate_with_runtime;
pub(crate) use self::delegate::{execute_delegate_async_tool, execute_delegate_tool};
use self::discovery::persist_tool_discovery_refresh_event_if_needed;
use self::finalize::{
    apply_resolved_provider_turn, finalize_provider_turn_reply,
    persist_resolved_provider_error_checkpoint,
};
use self::flow::{
    build_turn_loop_circuit_breaker_resolved_turn, prepare_provider_turn_continue_phase,
    provider_turn_usage, resolve_provider_turn, scope_provider_turn_tool_intents,
};
use self::lane::{assistant_preface_signals_provider_turn_followup, execute_provider_turn_lane};
#[cfg(test)]
use self::observer::summarize_tool_event_request;
use self::observer::{
    build_provider_turn_tool_terminal_events, observe_non_provider_turn_terminal_success_phases,
    observe_provider_turn_tool_batch_started, observe_provider_turn_tool_batch_terminal,
    observe_turn_phase, request_provider_turn_with_observer,
};
use self::pending_approval::*;
#[cfg(test)]
use self::reply::build_turn_reply_followup_messages;
use self::reply::{
    build_turn_reply_followup_messages_with_warning,
    persist_active_external_skills_from_followup_payload_if_needed, resolve_provider_turn_reply,
};
use self::safe_lane_events::*;
use self::safe_lane_execution::*;
use self::safe_lane_governor::*;
pub(crate) use self::safe_lane_routing::SafeLaneFailureRoute;
use self::safe_lane_routing::*;
use self::safe_lane_state::{SafeLaneExecutionMetrics, SafeLanePlanLoopState};
use self::setup::{lane_policy_from_config, require_production_kernel_binding};
use self::skill_activation::{
    explicit_skill_activation_tool_call_id, parse_explicit_skill_activation_input,
};
pub use self::state::ConversationTurnOutcome;
use self::state::*;
use super::super::config::{LoongConfig, ToolConsentMode};
use super::ConversationSessionAddress;
use super::ProviderErrorMode;
#[cfg(feature = "memory-sqlite")]
use super::active_external_skills;
use super::analytics::{
    SafeLaneEventSummary, TurnCheckpointProgressStatus as AnalyticsTurnCheckpointProgressStatus,
    TurnCheckpointRecoveryAction, build_turn_checkpoint_repair_plan, summarize_safe_lane_history,
};
#[cfg(feature = "memory-sqlite")]
use super::announce::DelegateAnnounceSettings;
#[cfg(feature = "memory-sqlite")]
use super::approval_resolution::CoordinatorApprovalResolutionRuntime;
use super::context_engine::{AssembledConversationContext, ConversationContextEngine};
#[cfg(feature = "memory-sqlite")]
use super::delegate_support::{
    enqueue_delegate_result_announce_with_memory_config,
    finalize_and_announce_delegate_child_terminal,
    finalize_async_delegate_spawn_failure_with_recovery, format_delegate_child_panic,
    next_delegate_child_depth_for_delegate, spawn_async_delegate_detached,
};
use super::ingress::ConversationIngressContext;
use super::lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
#[cfg(feature = "memory-sqlite")]
use super::mailbox_for_session;
use super::persistence::{
    format_provider_error_reply, persist_acp_runtime_events, persist_conversation_event,
    persist_reply_turns_raw_with_mode, persist_reply_turns_with_mode, persist_tool_decision,
    persist_tool_outcome, provider_error_reply_body,
};
use super::plan_executor::{
    PlanExecutor, PlanNodeError, PlanNodeErrorKind, PlanNodeExecutor, PlanRunFailure,
    PlanRunReport, PlanRunStatus,
};
pub(super) use super::plan_ir::{
    PLAN_GRAPH_VERSION, PlanBudget, PlanEdge, PlanGraph, PlanNode, PlanNodeKind, RiskTier,
};
use super::plan_verifier::{
    PlanVerificationContext, PlanVerificationFailureCode, PlanVerificationPolicy,
    PlanVerificationReport, verify_output,
};
use super::runtime::{
    AsyncDelegateSpawnRequest, ConversationRuntime, DefaultConversationRuntime, SessionContext,
};
use super::runtime_binding::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};
use super::safe_lane_failure::{
    SafeLaneFailureCode, SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource,
    classify_safe_lane_plan_failure,
};
#[cfg(feature = "memory-sqlite")]
use super::session_history::{
    AssistantHistoryLoadErrorCode, load_assistant_contents_from_session_window_detailed,
    load_latest_turn_checkpoint_entry, load_turn_checkpoint_history_snapshot,
};
#[cfg(feature = "memory-sqlite")]
use super::subagent::{
    ConstrainedSubagentExecution, ConstrainedSubagentMode, ConstrainedSubagentTerminalReason,
};
use super::trust_projection::{
    emit_provider_failover_trust_event_if_needed, emit_runtime_binding_trust_event_if_needed,
};
use super::turn_budget::{
    EscalatingAttemptBudget, SafeLaneBackpressureBudget, SafeLaneContinuationBudgetDecision,
    SafeLaneFailureRouteReason, SafeLaneReplanBudget,
};

type DefaultTurnRuntime = DefaultConversationRuntime<Box<dyn ConversationContextEngine>>;
#[cfg(feature = "memory-sqlite")]
use self::support::active_task_progress_record;
#[cfg(feature = "memory-sqlite")]
pub(crate) use self::support::emit_async_delegate_child_terminal_event;
use self::support::{
    ProviderTurnPreparation, ProviderTurnReplyTailPhase, ProviderTurnSessionState,
    checkpoint_requires_verification_phase, checkpoint_waits_for_external_resolution,
    completed_task_progress_record, emit_async_delegate_child_queued_event,
    emit_discovery_first_event, emit_prompt_frame_event, estimate_tokens_for_messages,
    failed_task_progress_record, inject_delegate_workspace_metadata,
    persist_task_progress_event_best_effort, split_delegate_workspace_cleanup,
    summarize_discovery_first_followup_turn, verifying_task_progress_record,
    waiting_task_progress_record,
};
use super::turn_checkpoint::{
    ContextCompactionOutcome, TurnCheckpointDiagnostics, TurnCheckpointFailure,
    TurnCheckpointFailureStep, TurnCheckpointFinalizationProgress, TurnCheckpointIdentity,
    TurnCheckpointProgressStatus, TurnCheckpointRecoveryAssessment,
    TurnCheckpointRepairResumeInput, TurnCheckpointRequest, TurnCheckpointResultKind,
    TurnCheckpointSnapshot, TurnCheckpointStage, TurnCheckpointTailRepairOutcome,
    TurnCheckpointTailRepairReason, TurnCheckpointTailRepairRuntimeProbe,
    TurnCheckpointTailRepairSource, TurnCheckpointTailRepairStatus,
    TurnCheckpointTailRuntimeEligibility, TurnFinalizationCheckpoint, TurnLaneExecutionSnapshot,
    TurnPreparationSnapshot, TurnReplyCheckpoint, checkpoint_context_fingerprint_sha256,
    persist_turn_checkpoint_event, persist_turn_checkpoint_event_value,
    persist_turn_checkpoint_event_with_compaction_diagnostics,
    restore_analytics_turn_checkpoint_progress_status, turn_checkpoint_result_kind,
};
use super::turn_engine::{
    AppToolDispatcher, DefaultAppToolDispatcher, ProviderTurn, ToolBatchExecutionIntentStatus,
    ToolBatchExecutionTrace, ToolExecutionPreflight, ToolIntent, TurnEngine, TurnFailure,
    TurnFailureKind, TurnResult, TurnValidation, effective_result_tool_name,
};
use super::turn_observer::{
    ConversationTurnObserverHandle, ConversationTurnPhase, ConversationTurnPhaseEvent,
    ConversationTurnToolEvent,
};
#[cfg(test)]
use super::turn_shared::ReplyResolutionMode;
#[cfg(feature = "memory-sqlite")]
use super::turn_shared::{ApprovalPromptActionId, parse_approval_prompt_action_input};
pub(super) use super::turn_shared::{
    ParsedToolDrivenContinuationReply, ProviderTurnRequestAction, ReplyPersistenceMode,
    ToolDrivenContinuationState, ToolDrivenFollowupContractMode, ToolDrivenFollowupKind,
    ToolDrivenFollowupPayload, ToolDrivenReplyBaseDecision, ToolDrivenReplyPhase,
    build_tool_driven_followup_tail_with_request_summary_and_contract,
    build_tool_followup_user_prompt_with_context, build_tool_loop_guard_tail,
    decide_provider_turn_request_action, effective_followup_tool_name,
    effective_followup_visible_tool_name, format_approval_required_reply,
    missing_tool_call_followup_payload, next_conversation_turn_id,
    parse_tool_driven_continuation_reply, reduce_followup_payload_for_model,
    render_tool_followup_continuation_contract, request_completion_with_raw_fallback,
    request_completion_with_raw_fallback_detailed, summarize_provider_lane_tool_request,
    summarize_single_tool_followup_request, tool_driven_followup_payload,
    tool_loop_circuit_breaker_reply, tool_result_contains_truncation_signal,
    user_requested_raw_tool_output,
};
#[cfg(feature = "memory-sqlite")]
use crate::conversation::workspace_isolation::{
    DelegateWorkspaceCleanupResult, cleanup_delegate_workspace_root,
    cleanup_prepared_delegate_workspace_root, prepare_delegate_workspace_root,
};
#[cfg(all(feature = "memory-sqlite", test))]
use crate::session::recovery::RECOVERY_EVENT_KIND;
#[cfg(all(test, feature = "memory-sqlite"))]
use crate::session::repository::FinalizeSessionTerminalRequest;
#[cfg(all(test, feature = "memory-sqlite"))]
use crate::session::repository::TransitionApprovalRequestIfCurrentRequest;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalDecision, ApprovalRequestStatus, NewSessionEvent, NewSessionRecord, SessionKind,
    SessionRepository, SessionState,
};
#[cfg(feature = "memory-sqlite")]
use loong_kernel::mailbox::{AgentPath, InterAgentMessage, MailboxContent};

#[derive(Default)]
pub struct ConversationTurnCoordinator;

const PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING: &str =
    "production conversation runtime requires kernel-bound execution";
pub use self::compact::ContextCompactionReport;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests;
