pub mod analytics;
mod lane_arbiter;
mod persistence;
pub mod plan_executor;
pub mod plan_ir;
pub mod plan_verifier;
mod runtime;
mod turn_coordinator;
pub mod turn_engine;
mod turn_loop;

pub use analytics::{
    ConversationEventRecord, SafeLaneEventSummary, SafeLaneFinalStatus, SafeLaneMetricsSnapshot,
    parse_conversation_event, summarize_safe_lane_events,
};
pub use lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
pub type ConversationOrchestrator = ConversationTurnCoordinator;
#[allow(unused_imports)]
pub use runtime::{ConversationRuntime, DefaultConversationRuntime};
pub use turn_coordinator::ConversationTurnCoordinator;
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
