pub mod analytics;
mod compaction;
mod context_engine;
mod context_engine_registry;
mod ingress;
mod lane_arbiter;
mod persistence;
pub mod plan_executor;
pub mod plan_ir;
pub mod plan_verifier;
mod runtime;
mod runtime_binding;
mod safe_lane_failure;
mod session_address;
mod session_history;
mod subagent;
mod turn_budget;
mod turn_coordinator;
pub mod turn_engine;
mod turn_loop;
mod turn_middleware;
mod turn_middleware_registry;
mod turn_shared;

pub use analytics::{
    ConversationEventRecord, DiscoveryFirstEventSummary, FastLaneToolBatchEventSummary,
    FastLaneToolBatchSegmentSnapshot, SafeLaneEventSummary, SafeLaneFinalStatus,
    SafeLaneHealthSignalSnapshot, SafeLaneMetricsSnapshot, SafeLaneToolOutputSnapshot,
    TurnCheckpointEventSummary, TurnCheckpointFailureStep, TurnCheckpointProgressStatus,
    TurnCheckpointRecoveryAction, TurnCheckpointRepairManualReason, TurnCheckpointRepairPlan,
    TurnCheckpointSessionState, TurnCheckpointStage, build_turn_checkpoint_repair_plan,
    parse_conversation_event, plan_turn_checkpoint_recovery, summarize_discovery_first_events,
    summarize_fast_lane_tool_batch_events, summarize_safe_lane_events,
    summarize_turn_checkpoint_events,
};
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
pub use ingress::{
    ConversationIngressChannel, ConversationIngressContext, ConversationIngressDelivery,
    ConversationIngressDeliveryResource, ConversationIngressFeishuCallbackContext,
    ConversationIngressPrivateContext,
};
pub use lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
#[allow(unused_imports)]
pub use runtime::{
    AsyncDelegateSpawnRequest, AsyncDelegateSpawner, ContextCompactionPolicySnapshot,
    ContextEngineRuntimeSnapshot, ContextEngineSelection, ContextEngineSelectionSource,
    ConversationRuntime, DefaultConversationRuntime, SessionContext, TurnMiddlewareRuntimeSnapshot,
    TurnMiddlewareSelection, TurnMiddlewareSelectionSource,
    collect_context_engine_runtime_snapshot, resolve_context_engine_selection,
    resolve_turn_middleware_selection,
};
pub use runtime_binding::ConversationRuntimeBinding;
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
pub use session_history::load_discovery_first_event_summary;
pub use session_history::{
    load_fast_lane_tool_batch_event_summary, load_safe_lane_event_summary,
    load_turn_checkpoint_event_summary,
};
pub use subagent::{
    ConstrainedSubagentExecution, ConstrainedSubagentMode, ConstrainedSubagentTerminalReason,
};
pub use turn_budget::SafeLaneFailureRouteReason;
pub use turn_coordinator::ConversationTurnCoordinator;
pub(crate) use turn_coordinator::{TurnCheckpointDiagnostics, TurnCheckpointRecoveryAssessment};
pub use turn_coordinator::{
    TurnCheckpointTailRepairOutcome, TurnCheckpointTailRepairReason,
    TurnCheckpointTailRepairRuntimeProbe, TurnCheckpointTailRepairSource,
    TurnCheckpointTailRepairStatus,
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
