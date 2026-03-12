pub mod analytics;
mod context_engine;
mod context_engine_registry;
mod lane_arbiter;
mod persistence;
pub mod plan_executor;
pub mod plan_ir;
pub mod plan_verifier;
mod runtime;
mod session_address;
mod turn_coordinator;
pub mod turn_engine;
mod turn_loop;
mod turn_shared;

pub use analytics::{
    ConversationEventRecord, SafeLaneEventSummary, SafeLaneFinalStatus,
    SafeLaneHealthSignalSnapshot, SafeLaneMetricsSnapshot, SafeLaneToolOutputSnapshot,
    parse_conversation_event, summarize_safe_lane_events,
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
pub use lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
#[allow(unused_imports)]
pub use runtime::{
    ContextCompactionPolicySnapshot, ContextEngineRuntimeSnapshot, ContextEngineSelection,
    ContextEngineSelectionSource, ConversationRuntime, DefaultConversationRuntime,
    collect_context_engine_runtime_snapshot, resolve_context_engine_selection,
};
pub use session_address::ConversationSessionAddress;
pub use turn_coordinator::ConversationTurnCoordinator;
pub type ConversationOrchestrator = ConversationTurnCoordinator;
pub use turn_engine::{
    ProviderTurn, ToolDecision, ToolIntent, ToolOutcome, TurnEngine, TurnFailure, TurnFailureKind,
    TurnResult,
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
