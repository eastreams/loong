mod acpx;
#[cfg(test)]
mod acpx_doctor_tests;
mod analytics;
mod backend;
mod binding;
mod manager;
mod registry;
mod runtime;
mod store;

pub use acpx::AcpxCliProbeBackend;
pub use analytics::{
    ACP_TURN_EVENT_RECORD, ACP_TURN_FINAL_RECORD, AcpTurnEventSummary,
    PersistedAcpConversationEventRecord, PersistedAcpRuntimeEventContext,
    build_persisted_runtime_event_records, build_persisted_turn_event_payload,
    build_persisted_turn_final_payload, merge_turn_events, summarize_turn_events,
};
pub use backend::{
    ACP_RUNTIME_API_VERSION, ACP_SESSION_METADATA_ACTIVATION_ORIGIN, ACP_TURN_METADATA_ACK_CURSOR,
    ACP_TURN_METADATA_ROUTING_INTENT, ACP_TURN_METADATA_ROUTING_ORIGIN,
    ACP_TURN_METADATA_SOURCE_MESSAGE_ID, ACP_TURN_METADATA_TRACE_ID, AcpAbortController,
    AcpAbortSignal, AcpBackendMetadata, AcpCapability, AcpConfigPatch, AcpConversationTurnOptions,
    AcpDoctorReport, AcpRoutingIntent, AcpRoutingOrigin, AcpRuntimeBackend, AcpSessionBootstrap,
    AcpSessionHandle, AcpSessionMetadata, AcpSessionMode, AcpSessionState, AcpSessionStatus,
    AcpTurnEventSink, AcpTurnProvenance, AcpTurnRequest, AcpTurnResult, AcpTurnStopReason,
    BufferedAcpTurnEventSink, CompositeAcpTurnEventSink, JsonlAcpTurnEventSink,
    PlanningStubAcpBackend, StreamingTokenEvent, TokenDelta, ToolCallDelta,
};
pub use binding::AcpSessionBindingScope;
pub use manager::{
    AcpManagerActorSnapshot, AcpManagerObservabilitySnapshot, AcpManagerRuntimeCacheSnapshot,
    AcpManagerSessionSnapshot, AcpManagerTurnSnapshot, AcpSessionManager,
};
pub use registry::{
    ACP_BACKEND_ENV, DEFAULT_ACP_BACKEND_ID, acp_backend_id_from_env, describe_acp_backend,
    list_acp_backend_ids, list_acp_backend_metadata, register_acp_backend, resolve_acp_backend,
};
pub use runtime::{
    AcpBackendSelection, AcpBackendSelectionSource, AcpControlPlaneSnapshot,
    AcpConversationDispatchDecision, AcpConversationDispatchReason, AcpConversationDispatchTarget,
    AcpConversationRoute, AcpConversationTurnEntryDecision, AcpRuntimeSnapshot,
    PreparedAcpConversationTurn, collect_acp_runtime_snapshot, derive_acp_conversation_route,
    derive_acp_conversation_route_for_address, derive_automatic_acp_routing_origin_for_address,
    describe_acp_conversation_dispatch_target,
    describe_acp_conversation_dispatch_target_for_address,
    evaluate_acp_conversation_dispatch_for_address,
    evaluate_acp_conversation_turn_entry_for_address, prepare_acp_conversation_turn_for_address,
    resolve_acp_backend_selection, shared_acp_session_manager,
    should_route_conversation_turn_via_acp, should_route_conversation_turn_via_acp_for_address,
};
pub(crate) use runtime::{
    AcpConversationTurnExecutionOutcome, execute_acp_conversation_turn_for_address,
};
#[cfg(feature = "memory-sqlite")]
pub use store::AcpSqliteSessionStore;
pub use store::{AcpSessionStore, InMemoryAcpSessionStore};
