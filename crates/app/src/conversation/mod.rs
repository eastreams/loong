mod active_external_skills;
pub mod analytics;
mod announce;
mod approval_resolution;
mod autonomy_policy;
mod compaction;
mod compaction_diagnostics;
mod compaction_preparation;
mod compaction_pruning;
mod compaction_retention;
mod compaction_snapshot;
mod context_engine;
mod context_engine_registry;
mod delegate_support;
mod ingress;
mod lane_arbiter;
mod persistence;
pub mod plan_executor;
pub mod plan_ir;
pub mod plan_verifier;
mod prompt_fragments;
mod prompt_frame;
mod prompt_orchestrator;
mod runtime;
mod runtime_binding;
mod safe_lane_failure;
mod session_address;
mod session_history;
mod session_state;
mod subagent;
mod tool_discovery_state;
mod tool_input_contract;
mod tool_loop_supervisor;
mod tool_result_compaction;
mod tool_result_line;
mod tool_result_reduction;
mod trust_projection;
mod turn_budget;
mod turn_checkpoint;
mod turn_coordinator;
pub mod turn_engine;
mod turn_loop;
mod turn_middleware;
mod turn_middleware_registry;
mod turn_observer;
mod turn_shared;
pub(crate) mod workspace_isolation;

pub(crate) const FAST_LANE_PARALLEL_TOOL_EXECUTION_ENABLED: bool = true;
pub(crate) const FAST_LANE_PARALLEL_TOOL_EXECUTION_MAX_IN_FLIGHT: usize = 4;
pub(crate) const FAST_LANE_MAX_TOOL_STEPS_PER_TURN: usize = 5;
pub(crate) const SAFE_LANE_MAX_TOOL_STEPS_PER_TURN: usize = 2;
pub(crate) const SAFE_LANE_NODE_MAX_ATTEMPTS: u8 = 1;
pub(crate) const SAFE_LANE_REPLAN_MAX_ROUNDS: u8 = 16;
pub(crate) const SAFE_LANE_REPLAN_MAX_NODE_ATTEMPTS: u8 = 4;
pub(crate) const TURN_LOOP_MAX_CONSECUTIVE_SAME_TOOL: usize = 10;
pub(crate) const TURN_LOOP_MAX_DISCOVERY_FOLLOWUP_ROUNDS: usize = 12;
pub(crate) const TURN_LOOP_MAX_PING_PONG_CYCLES: usize = 2;
pub(crate) const TURN_LOOP_MAX_REPEATED_TOOL_CALL_ROUNDS: usize = 2;
pub(crate) const TURN_LOOP_MAX_ROUNDS: usize = 6;
pub(crate) const TURN_LOOP_MAX_SAME_TOOL_FAILURE_ROUNDS: usize = 3;
pub(crate) const TURN_LOOP_MAX_TOTAL_TOOL_CALLS: usize = 200;
pub(crate) const TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 2_048;

pub use analytics::{
    ConversationEventRecord, DiscoveryFirstEventSummary, FastLaneToolBatchEventSummary,
    FastLaneToolBatchSegmentSnapshot, PromptFrameEventSummary, SafeLaneEventSummary,
    SafeLaneFinalStatus, SafeLaneHealthSignalSnapshot, SafeLaneMetricsSnapshot,
    SafeLaneToolOutputSnapshot, TurnCheckpointEventSummary, TurnCheckpointFailureStep,
    TurnCheckpointProgressStatus, TurnCheckpointRecoveryAction, TurnCheckpointRepairManualReason,
    TurnCheckpointRepairPlan, TurnCheckpointSessionState, TurnCheckpointStage,
    build_turn_checkpoint_repair_plan, parse_conversation_event, plan_turn_checkpoint_recovery,
    summarize_discovery_first_events, summarize_fast_lane_tool_batch_events,
    summarize_prompt_frame_events, summarize_safe_lane_events, summarize_turn_checkpoint_events,
};
pub(crate) use compaction::{COMPACTED_SUMMARY_PREFIX, is_compacted_summary_content};
pub(crate) use compaction_diagnostics::ContextCompactionDiagnostics;
#[cfg(feature = "memory-sqlite")]
pub(crate) use compaction_snapshot::{CompactionSessionSnapshot, load_compaction_session_snapshot};
pub use context_engine::{
    AssembledConversationContext, CONTEXT_ENGINE_API_VERSION, ContextArtifactDescriptor,
    ContextArtifactKind, ContextEngineBootstrapResult, ContextEngineCapability,
    ContextEngineIngestResult, ContextEngineMetadata, ConversationContextEngine,
    DefaultContextEngine, LegacyContextEngine, ToolOutputStreamingPolicy,
};
pub use context_engine_registry::{
    CONTEXT_ENGINE_ENV, DEFAULT_CONTEXT_ENGINE_ID, LEGACY_CONTEXT_ENGINE_ID,
    context_engine_id_from_env, describe_context_engine, list_context_engine_ids,
    list_context_engine_metadata, register_context_engine, resolve_context_engine,
};
#[cfg(feature = "memory-sqlite")]
pub(crate) use delegate_support::with_prepared_subagent_spawn_cleanup_if_kernel_bound;
pub use ingress::{
    ConversationIngressChannel, ConversationIngressContext, ConversationIngressDelivery,
    ConversationIngressDeliveryResource, ConversationIngressFeishuCallbackContext,
    ConversationIngressPrivateContext,
};
pub use lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
pub use prompt_fragments::{
    PromptFragment, PromptFrameAuthority, PromptFrameLayer, PromptLane, PromptRenderPolicy,
};
pub use prompt_frame::{
    PromptFrame, PromptFrameBucketSummary, PromptFrameFragmentSummary, PromptFrameLayerStats,
    PromptFrameMessageSummary, PromptFrameSummary, summarize_assembled_prompt_frame,
    summarize_followup_prompt_frame,
};
pub use prompt_orchestrator::{PromptCompilation, PromptCompiler};
#[allow(unused_imports)]
pub use runtime::{
    AsyncDelegateSpawnRequest, AsyncDelegateSpawner, ContextCompactionPolicySnapshot,
    ContextEngineRuntimeSnapshot, ContextEngineSelection, ContextEngineSelectionSource,
    ConversationRuntime, DefaultConversationRuntime, HostedConversationRuntime, SessionContext,
    TurnMiddlewareRuntimeSnapshot, TurnMiddlewareSelection, TurnMiddlewareSelectionSource,
    async_delegate_spawn_request_from_serialized_parts, collect_context_engine_runtime_snapshot,
    execute_async_delegate_spawn_request, load_default_conversation_runtime,
    load_hosted_default_conversation_runtime, resolve_context_engine_selection,
    resolve_turn_middleware_selection,
};
pub use runtime_binding::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};
pub use safe_lane_failure::{
    SafeLaneFailureCode, SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource,
    SafeLaneTerminalRouteSnapshot, classify_safe_lane_plan_failure,
    is_safe_lane_backpressure_failure_code, is_safe_lane_backpressure_route_reason,
    is_safe_lane_terminal_instability_failure_code,
};
pub use session_address::{
    ConversationSessionAddress, decode_route_session_segment, encode_route_session_segment,
    parse_route_session_id,
};
pub use session_history::{
    load_discovery_first_event_summary, load_discovery_first_event_summary_with_kernel_context,
    load_prompt_frame_event_summary,
};
pub use session_history::{
    load_fast_lane_tool_batch_event_summary, load_safe_lane_event_summary,
    load_turn_checkpoint_event_summary,
};
pub(crate) use session_state::{InterAgentMessage, mailbox_for_session};
pub use subagent::{
    ConstrainedSubagentBudgetSnapshot, ConstrainedSubagentContractView,
    ConstrainedSubagentControlScope, ConstrainedSubagentCoordinationAction,
    ConstrainedSubagentCoordinationActionKind, ConstrainedSubagentExecution,
    ConstrainedSubagentHandle, ConstrainedSubagentIdentity, ConstrainedSubagentIsolation,
    ConstrainedSubagentMode, ConstrainedSubagentOwnerKind, ConstrainedSubagentProfile,
    ConstrainedSubagentRole, ConstrainedSubagentRuntimeBinding, ConstrainedSubagentTerminalReason,
    DelegateBuiltinProfile, coordination_actions_for_subagent_handle, subagent_surface_fields,
};
pub(crate) use tool_discovery_state::latest_tool_discovery_state_from_assistant_contents;
pub use turn_budget::SafeLaneFailureRouteReason;
pub(crate) use turn_checkpoint::{TurnCheckpointDiagnostics, TurnCheckpointRecoveryAssessment};
pub use turn_checkpoint::{
    TurnCheckpointTailRepairOutcome, TurnCheckpointTailRepairReason,
    TurnCheckpointTailRepairRuntimeProbe, TurnCheckpointTailRepairSource,
    TurnCheckpointTailRepairStatus,
};
pub(crate) use turn_coordinator::ContextCompactionReport;
#[cfg(feature = "memory-sqlite")]
pub(crate) use turn_coordinator::run_started_delegate_child_turn_with_runtime;
pub use turn_coordinator::{
    ConversationTurnCoordinator, ConversationTurnOutcome, spawn_background_delegate_with_runtime,
};
pub use turn_engine::{
    AppToolDispatcher, DefaultAppToolDispatcher, NoopAppToolDispatcher, ProviderTurn, ToolDecision,
    ToolIntent, ToolOutcome, TurnEngine, TurnFailure, TurnFailureKind, TurnResult,
};
pub use turn_loop::ConversationTurnLoop;
pub use turn_middleware::{
    ConversationTurnMiddleware, SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID,
    SYSTEM_PROMPT_TOOL_VIEW_TURN_MIDDLEWARE_ID, TURN_MIDDLEWARE_API_VERSION,
    TurnMiddlewareCapability, TurnMiddlewareMetadata,
};
pub use turn_middleware_registry::{
    TURN_MIDDLEWARE_ENV, default_turn_middleware_ids, describe_turn_middlewares,
    list_turn_middleware_ids, list_turn_middleware_metadata, register_turn_middleware,
    resolve_turn_middleware, resolve_turn_middlewares, turn_middleware_ids_from_env,
};
pub(crate) use turn_observer::ConversationTurnRuntimeEvent;
pub use turn_observer::{
    ConversationTurnObserver, ConversationTurnObserverHandle, ConversationTurnPhase,
    ConversationTurnPhaseEvent, ConversationTurnToolEvent, ConversationTurnToolState,
};
pub use turn_shared::{
    ApprovalPromptActionId, ApprovalPromptActionView, ApprovalPromptLocale, ApprovalPromptMarker,
    ApprovalPromptView, parse_approval_prompt_action_input, parse_approval_prompt_view,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorMode {
    #[cfg_attr(
        not(any(
            feature = "channel-telegram",
            feature = "channel-feishu",
            feature = "channel-matrix"
        )),
        allow(dead_code)
    )]
    Propagate,
    InlineMessage,
}

#[cfg(test)]
mod tests;
