pub mod analytics;
mod context_engine;
mod context_engine_registry;
mod ingress;
mod lane_arbiter;
mod persistence;
pub mod plan_executor;
pub mod plan_ir;
pub mod plan_verifier;
mod runtime;
mod safe_lane_failure;
mod session_address;
mod session_history;
mod turn_budget;
mod turn_coordinator;
pub mod turn_engine;
mod turn_loop;
mod turn_shared;

pub use analytics::{
    ConversationEventRecord, SafeLaneEventSummary, SafeLaneFinalStatus,
    SafeLaneHealthSignalSnapshot, SafeLaneMetricsSnapshot, SafeLaneToolOutputSnapshot,
    TurnCheckpointEventSummary, TurnCheckpointFailureStep, TurnCheckpointProgressStatus,
    TurnCheckpointRecoveryAction, TurnCheckpointRepairManualReason, TurnCheckpointRepairPlan,
    TurnCheckpointSessionState, TurnCheckpointStage, build_turn_checkpoint_repair_plan,
    parse_conversation_event, plan_turn_checkpoint_recovery, summarize_safe_lane_events,
    summarize_turn_checkpoint_events,
};
pub use context_engine::{
    AssembledConversationContext, CONTEXT_ENGINE_API_VERSION, ContextEngineBootstrapResult,
    ContextEngineCapability, ContextEngineIngestResult, ContextEngineMetadata,
    ConversationContextEngine, DefaultContextEngine, LegacyContextEngine,
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
    ConversationRuntime, DefaultConversationRuntime, SessionContext,
    collect_context_engine_runtime_snapshot, resolve_context_engine_selection,
};
pub use safe_lane_failure::{
    SafeLaneFailureCode, SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource,
    SafeLaneTerminalRouteSnapshot, classify_safe_lane_plan_failure,
    is_safe_lane_backpressure_failure_code, is_safe_lane_backpressure_route_reason,
    is_safe_lane_terminal_instability_failure_code,
};
pub use session_address::ConversationSessionAddress;
pub use session_history::{load_safe_lane_event_summary, load_turn_checkpoint_event_summary};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorMode {
    #[cfg_attr(
        not(any(feature = "channel-telegram", feature = "channel-feishu")),
        allow(dead_code)
    )]
    Propagate,
    InlineMessage,
}

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;
