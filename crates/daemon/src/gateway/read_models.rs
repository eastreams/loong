use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::CHANNELS_CLI_JSON_LEGACY_VIEWS;
use crate::CHANNELS_CLI_JSON_SCHEMA_VERSION;
use crate::RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION;
use crate::RuntimeSnapshotCliState;
use crate::mvp;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayChannelInventorySchema {
    pub version: u32,
    pub primary_channel_view: &'static str,
    pub catalog_view: &'static str,
    pub legacy_channel_views: &'static [&'static str],
}

pub type ChannelsCliJsonSchema = GatewayChannelInventorySchema;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayChannelInventoryReadModel {
    pub config: String,
    pub schema: GatewayChannelInventorySchema,
    pub channels: Vec<mvp::channel::ChannelStatusSnapshot>,
    pub catalog_only_channels: Vec<mvp::channel::ChannelCatalogEntry>,
    pub channel_catalog: Vec<mvp::channel::ChannelCatalogEntry>,
    pub channel_surfaces: Vec<mvp::channel::ChannelSurface>,
}

pub type ChannelsCliJsonPayload = GatewayChannelInventoryReadModel;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpBindingScopeReadModel {
    pub route_session_id: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpSessionActivationProvenanceReadModel {
    pub surface: &'static str,
    pub activation_origin: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpSessionStatusReadModel {
    pub session_key: String,
    pub backend_id: String,
    pub conversation_id: Option<String>,
    pub binding: Option<GatewayAcpBindingScopeReadModel>,
    pub activation_origin: Option<&'static str>,
    pub provenance: GatewayAcpSessionActivationProvenanceReadModel,
    pub state: &'static str,
    pub mode: Option<&'static str>,
    pub pending_turns: usize,
    pub active_turn_id: Option<String>,
    pub last_activity_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpStatusReadModel {
    pub config: String,
    pub requested_session: Option<String>,
    pub requested_conversation_id: Option<String>,
    pub requested_route_session_id: Option<String>,
    pub resolved_session_key: String,
    pub status: GatewayAcpSessionStatusReadModel,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpActivationAggregateProvenanceReadModel {
    pub surface: &'static str,
    pub activation_origin_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpRuntimeCacheReadModel {
    pub active_sessions: usize,
    pub idle_ttl_ms: u64,
    pub evicted_total: u64,
    pub last_evicted_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpSessionAggregateReadModel {
    pub bound: usize,
    pub unbound: usize,
    pub activation_origin_counts: BTreeMap<String, usize>,
    pub provenance: GatewayAcpActivationAggregateProvenanceReadModel,
    pub backend_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpActorReadModel {
    pub active: usize,
    pub queue_depth: usize,
    pub waiting: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpTurnReadModel {
    pub active: usize,
    pub queue_depth: usize,
    pub completed: u64,
    pub failed: u64,
    pub average_latency_ms: u64,
    pub max_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpObservabilitySnapshotReadModel {
    pub runtime_cache: GatewayAcpRuntimeCacheReadModel,
    pub sessions: GatewayAcpSessionAggregateReadModel,
    pub actors: GatewayAcpActorReadModel,
    pub turns: GatewayAcpTurnReadModel,
    pub errors_by_code: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpObservabilityReadModel {
    pub config: String,
    pub snapshot: GatewayAcpObservabilitySnapshotReadModel,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayConversationAddressReadModel {
    pub session_id: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpDispatchPredictionProvenanceReadModel {
    pub surface: &'static str,
    pub automatic_routing_origin: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpDispatchTargetReadModel {
    pub original_session_id: String,
    pub route_session_id: String,
    pub prefixed_agent_id: Option<String>,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<String>,
    pub channel_path: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpDispatchDecisionDetailsReadModel {
    pub route_via_acp: bool,
    pub reason: &'static str,
    pub automatic_routing_origin: Option<&'static str>,
    pub provenance: GatewayAcpDispatchPredictionProvenanceReadModel,
    pub target: GatewayAcpDispatchTargetReadModel,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpDispatchDecisionReadModel {
    pub session: String,
    pub decision: GatewayAcpDispatchDecisionDetailsReadModel,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpDispatchReadModel {
    pub config: String,
    pub address: GatewayConversationAddressReadModel,
    pub dispatch: GatewayAcpDispatchDecisionReadModel,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayRuntimeSnapshotSchema {
    pub version: u32,
    pub surface: &'static str,
    pub purpose: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayRuntimeSnapshotChannelsReadModel {
    pub enabled_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub inventory: GatewayChannelInventoryReadModel,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayRuntimeSnapshotToolsReadModel {
    pub visible_tool_count: usize,
    pub visible_tool_names: Vec<String>,
    pub capability_snapshot_sha256: String,
    pub capability_snapshot: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayRuntimeSnapshotReadModel {
    pub config: String,
    pub schema: GatewayRuntimeSnapshotSchema,
    pub provider: Value,
    pub context_engine: Value,
    pub memory_system: Value,
    pub acp: Value,
    pub channels: GatewayRuntimeSnapshotChannelsReadModel,
    pub tool_runtime: Value,
    pub tools: GatewayRuntimeSnapshotToolsReadModel,
    pub external_skills: Value,
}

pub fn build_channel_inventory_read_model(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> GatewayChannelInventoryReadModel {
    let config = config_path.to_owned();
    let schema = GatewayChannelInventorySchema {
        version: CHANNELS_CLI_JSON_SCHEMA_VERSION,
        primary_channel_view: "channel_surfaces",
        catalog_view: "channel_catalog",
        legacy_channel_views: CHANNELS_CLI_JSON_LEGACY_VIEWS,
    };
    let channels = inventory.channels.clone();
    let catalog_only_channels = inventory.catalog_only_channels.clone();
    let channel_catalog = inventory.channel_catalog.clone();
    let channel_surfaces = inventory.channel_surfaces.clone();

    GatewayChannelInventoryReadModel {
        config,
        schema,
        channels,
        catalog_only_channels,
        channel_catalog,
        channel_surfaces,
    }
}

pub fn build_acp_status_read_model(
    config_path: &str,
    requested_session: Option<&str>,
    requested_conversation_id: Option<&str>,
    requested_route_session_id: Option<&str>,
    resolved_session_key: &str,
    status: &mvp::acp::AcpSessionStatus,
) -> GatewayAcpStatusReadModel {
    let config = config_path.to_owned();
    let requested_session = requested_session.map(str::to_owned);
    let requested_conversation_id = requested_conversation_id.map(str::to_owned);
    let requested_route_session_id = requested_route_session_id.map(str::to_owned);
    let resolved_session_key = resolved_session_key.to_owned();
    let status = build_acp_session_status_read_model(status);

    GatewayAcpStatusReadModel {
        config,
        requested_session,
        requested_conversation_id,
        requested_route_session_id,
        resolved_session_key,
        status,
    }
}

pub fn build_acp_observability_read_model(
    config_path: &str,
    snapshot: &mvp::acp::AcpManagerObservabilitySnapshot,
) -> GatewayAcpObservabilityReadModel {
    let config = config_path.to_owned();
    let snapshot = build_acp_observability_snapshot_read_model(snapshot);

    GatewayAcpObservabilityReadModel { config, snapshot }
}

pub fn build_acp_dispatch_read_model(
    config_path: &str,
    address: &mvp::conversation::ConversationSessionAddress,
    session_id: &str,
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> GatewayAcpDispatchReadModel {
    let config = config_path.to_owned();
    let address = build_conversation_address_read_model(address);
    let dispatch = build_acp_dispatch_decision_read_model(session_id, decision);

    GatewayAcpDispatchReadModel {
        config,
        address,
        dispatch,
    }
}

pub fn build_runtime_snapshot_read_model(
    snapshot: &RuntimeSnapshotCliState,
) -> GatewayRuntimeSnapshotReadModel {
    let config = snapshot.config.clone();
    let schema = GatewayRuntimeSnapshotSchema {
        version: RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION,
        surface: "runtime_snapshot",
        purpose: "experiment_reproducibility",
    };
    let provider = crate::runtime_snapshot_provider_json(&snapshot.provider);
    let context_engine = crate::runtime_snapshot_context_engine_json(&snapshot.context_engine);
    let memory_system = crate::runtime_snapshot_memory_system_json(&snapshot.memory_system);
    let acp = crate::runtime_snapshot_acp_json(&snapshot.acp);
    let inventory = build_channel_inventory_read_model(config.as_str(), &snapshot.channels);
    let enabled_channel_ids = snapshot.enabled_channel_ids.clone();
    let enabled_service_channel_ids = snapshot.enabled_service_channel_ids.clone();
    let channels = GatewayRuntimeSnapshotChannelsReadModel {
        enabled_channel_ids,
        enabled_service_channel_ids,
        inventory,
    };
    let tool_runtime = crate::runtime_snapshot_tool_runtime_json(&snapshot.tool_runtime);
    let visible_tool_count = snapshot.visible_tool_names.len();
    let visible_tool_names = snapshot.visible_tool_names.clone();
    let capability_snapshot_sha256 = snapshot.capability_snapshot_sha256.clone();
    let capability_snapshot = snapshot.capability_snapshot.clone();
    let tools = GatewayRuntimeSnapshotToolsReadModel {
        visible_tool_count,
        visible_tool_names,
        capability_snapshot_sha256,
        capability_snapshot,
    };
    let external_skills = crate::runtime_snapshot_external_skills_json(&snapshot.external_skills);

    GatewayRuntimeSnapshotReadModel {
        config,
        schema,
        provider,
        context_engine,
        memory_system,
        acp,
        channels,
        tool_runtime,
        tools,
        external_skills,
    }
}

fn build_acp_binding_scope_read_model(
    binding: &mvp::acp::AcpSessionBindingScope,
) -> GatewayAcpBindingScopeReadModel {
    let route_session_id = binding.route_session_id.clone();
    let channel_id = binding.channel_id.clone();
    let account_id = binding.account_id.clone();
    let conversation_id = binding.conversation_id.clone();
    let thread_id = binding.thread_id.clone();

    GatewayAcpBindingScopeReadModel {
        route_session_id,
        channel_id,
        account_id,
        conversation_id,
        thread_id,
    }
}

fn build_acp_session_activation_provenance_read_model(
    origin: Option<mvp::acp::AcpRoutingOrigin>,
) -> GatewayAcpSessionActivationProvenanceReadModel {
    let activation_origin = origin.map(mvp::acp::AcpRoutingOrigin::as_str);
    let surface = "session_activation";

    GatewayAcpSessionActivationProvenanceReadModel {
        surface,
        activation_origin,
    }
}

fn build_acp_session_status_read_model(
    status: &mvp::acp::AcpSessionStatus,
) -> GatewayAcpSessionStatusReadModel {
    let session_key = status.session_key.clone();
    let backend_id = status.backend_id.clone();
    let conversation_id = status.conversation_id.clone();
    let binding = status
        .binding
        .as_ref()
        .map(build_acp_binding_scope_read_model);
    let activation_origin = status
        .activation_origin
        .map(mvp::acp::AcpRoutingOrigin::as_str);
    let provenance = build_acp_session_activation_provenance_read_model(status.activation_origin);
    let state = crate::acp_session_state_label(status.state);
    let mode = status.mode.map(crate::acp_session_mode_label);
    let pending_turns = status.pending_turns;
    let active_turn_id = status.active_turn_id.clone();
    let last_activity_ms = status.last_activity_ms;
    let last_error = status.last_error.clone();

    GatewayAcpSessionStatusReadModel {
        session_key,
        backend_id,
        conversation_id,
        binding,
        activation_origin,
        provenance,
        state,
        mode,
        pending_turns,
        active_turn_id,
        last_activity_ms,
        last_error,
    }
}

fn build_acp_observability_snapshot_read_model(
    snapshot: &mvp::acp::AcpManagerObservabilitySnapshot,
) -> GatewayAcpObservabilitySnapshotReadModel {
    let active_sessions = snapshot.runtime_cache.active_sessions;
    let idle_ttl_ms = snapshot.runtime_cache.idle_ttl_ms;
    let evicted_total = snapshot.runtime_cache.evicted_total;
    let last_evicted_at_ms = snapshot.runtime_cache.last_evicted_at_ms;
    let runtime_cache = GatewayAcpRuntimeCacheReadModel {
        active_sessions,
        idle_ttl_ms,
        evicted_total,
        last_evicted_at_ms,
    };

    let bound = snapshot.sessions.bound;
    let unbound = snapshot.sessions.unbound;
    let activation_origin_counts = snapshot.sessions.activation_origin_counts.clone();
    let backend_counts = snapshot.sessions.backend_counts.clone();
    let provenance_counts = activation_origin_counts.clone();
    let provenance = GatewayAcpActivationAggregateProvenanceReadModel {
        surface: "session_activation_aggregate",
        activation_origin_counts: provenance_counts,
    };
    let sessions = GatewayAcpSessionAggregateReadModel {
        bound,
        unbound,
        activation_origin_counts,
        provenance,
        backend_counts,
    };

    let actor_active = snapshot.actors.active;
    let actor_queue_depth = snapshot.actors.queue_depth;
    let actor_waiting = snapshot.actors.waiting;
    let actors = GatewayAcpActorReadModel {
        active: actor_active,
        queue_depth: actor_queue_depth,
        waiting: actor_waiting,
    };

    let turn_active = snapshot.turns.active;
    let turn_queue_depth = snapshot.turns.queue_depth;
    let turn_completed = snapshot.turns.completed;
    let turn_failed = snapshot.turns.failed;
    let turn_average_latency_ms = snapshot.turns.average_latency_ms;
    let turn_max_latency_ms = snapshot.turns.max_latency_ms;
    let turns = GatewayAcpTurnReadModel {
        active: turn_active,
        queue_depth: turn_queue_depth,
        completed: turn_completed,
        failed: turn_failed,
        average_latency_ms: turn_average_latency_ms,
        max_latency_ms: turn_max_latency_ms,
    };

    let errors_by_code = snapshot.errors_by_code.clone();

    GatewayAcpObservabilitySnapshotReadModel {
        runtime_cache,
        sessions,
        actors,
        turns,
        errors_by_code,
    }
}

fn build_conversation_address_read_model(
    address: &mvp::conversation::ConversationSessionAddress,
) -> GatewayConversationAddressReadModel {
    let session_id = address.session_id.clone();
    let channel_id = address.channel_id.clone();
    let account_id = address.account_id.clone();
    let conversation_id = address.conversation_id.clone();
    let thread_id = address.thread_id.clone();

    GatewayConversationAddressReadModel {
        session_id,
        channel_id,
        account_id,
        conversation_id,
        thread_id,
    }
}

fn build_acp_dispatch_prediction_provenance_read_model(
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> GatewayAcpDispatchPredictionProvenanceReadModel {
    let surface = "dispatch_prediction";
    let automatic_routing_origin = decision
        .automatic_routing_origin
        .map(mvp::acp::AcpRoutingOrigin::as_str);

    GatewayAcpDispatchPredictionProvenanceReadModel {
        surface,
        automatic_routing_origin,
    }
}

fn build_acp_dispatch_target_read_model(
    target: &mvp::acp::AcpConversationDispatchTarget,
) -> GatewayAcpDispatchTargetReadModel {
    let original_session_id = target.original_session_id.clone();
    let route_session_id = target.route_session_id.clone();
    let prefixed_agent_id = target.prefixed_agent_id.clone();
    let channel_id = target.channel_id.clone();
    let account_id = target.account_id.clone();
    let conversation_id = target.conversation_id.clone();
    let thread_id = target.thread_id.clone();
    let channel_path = target.channel_path.clone();

    GatewayAcpDispatchTargetReadModel {
        original_session_id,
        route_session_id,
        prefixed_agent_id,
        channel_id,
        account_id,
        conversation_id,
        thread_id,
        channel_path,
    }
}

fn build_acp_dispatch_decision_read_model(
    session_id: &str,
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> GatewayAcpDispatchDecisionReadModel {
    let session = session_id.to_owned();
    let route_via_acp = decision.route_via_acp;
    let reason = decision.reason.as_str();
    let automatic_routing_origin = decision
        .automatic_routing_origin
        .map(mvp::acp::AcpRoutingOrigin::as_str);
    let provenance = build_acp_dispatch_prediction_provenance_read_model(decision);
    let target = build_acp_dispatch_target_read_model(&decision.target);
    let decision = GatewayAcpDispatchDecisionDetailsReadModel {
        route_via_acp,
        reason,
        automatic_routing_origin,
        provenance,
        target,
    };

    GatewayAcpDispatchDecisionReadModel { session, decision }
}
