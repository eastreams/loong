use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CHANNELS_CLI_JSON_LEGACY_VIEWS;
use crate::CHANNELS_CLI_JSON_SCHEMA_VERSION;
use crate::RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION;
use crate::RuntimeSnapshotCliState;
use crate::mvp;
use crate::plugin_bridge_account_summary::plugin_bridge_account_summary;

use super::state::GatewayOwnerStatus;

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
    pub channel_surfaces: Vec<GatewayChannelSurfaceReadModel>,
    pub channel_access_policies: Vec<mvp::channel::ChannelConfiguredAccountAccessPolicy>,
}

pub type ChannelsCliJsonPayload = GatewayChannelInventoryReadModel;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayChannelSurfaceReadModel {
    #[serde(flatten)]
    pub surface: mvp::channel::ChannelSurface,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_bridge_account_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpBindingScopeReadModel {
    pub route_session_id: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpSessionActivationProvenanceReadModel {
    pub surface: &'static str,
    pub activation_origin: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpSessionMetadataReadModel {
    pub session_key: String,
    pub conversation_id: Option<String>,
    pub binding: Option<GatewayAcpBindingScopeReadModel>,
    pub activation_origin: Option<&'static str>,
    pub provenance: GatewayAcpSessionActivationProvenanceReadModel,
    pub backend_id: String,
    pub runtime_session_name: String,
    pub working_directory: Option<String>,
    pub backend_session_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub mode: Option<&'static str>,
    pub state: &'static str,
    pub last_activity_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayAcpSessionListReadModel {
    pub config: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub sessions: Vec<GatewayAcpSessionMetadataReadModel>,
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
    pub participant_id: Option<String>,
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
    pub participant_id: Option<String>,
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
    pub tool_calling: GatewayToolCallingReadModel,
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
    pub runtime_plugins: Value,
    pub external_skills: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorControlSurfaceReadModel {
    pub base_url: Option<String>,
    pub loopback_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorChannelSurfaceReadModel {
    pub channel_id: String,
    pub label: String,
    pub implementation_status: String,
    pub configured_account_count: usize,
    pub enabled_account_count: usize,
    pub misconfigured_account_count: usize,
    pub ready_send_account_count: usize,
    pub ready_serve_account_count: usize,
    pub conversation_gated_account_count: usize,
    pub sender_gated_account_count: usize,
    pub mention_gated_account_count: usize,
    pub default_configured_account_id: Option<String>,
    pub plugin_bridge_account_summary: Option<String>,
    pub service_enabled: bool,
    pub service_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorChannelsSummaryReadModel {
    pub catalog_channel_count: usize,
    pub configured_channel_count: usize,
    pub configured_account_count: usize,
    pub enabled_account_count: usize,
    pub misconfigured_account_count: usize,
    pub runtime_backed_channel_count: usize,
    pub enabled_service_channel_count: usize,
    pub ready_service_channel_count: usize,
    pub surfaces: Vec<GatewayOperatorChannelSurfaceReadModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorRuntimeSummaryReadModel {
    pub enabled_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub visible_tool_count: usize,
    pub capability_snapshot_sha256: String,
    pub active_provider_profile_id: Option<String>,
    pub active_provider_label: Option<String>,
    pub tool_calling: GatewayToolCallingReadModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayToolCallingReadModel {
    pub availability: String,
    pub structured_tool_schema_enabled: bool,
    pub effective_tool_schema_mode: String,
    pub active_model: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorSummaryReadModel {
    pub owner: GatewayOwnerStatus,
    pub control_surface: GatewayOperatorControlSurfaceReadModel,
    pub channels: GatewayOperatorChannelsSummaryReadModel,
    pub runtime: GatewayOperatorRuntimeSummaryReadModel,
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
    let channel_surfaces = inventory
        .channel_surfaces
        .iter()
        .cloned()
        .map(build_channel_surface_read_model)
        .collect();
    let channel_access_policies = inventory.channel_access_policies.clone();

    GatewayChannelInventoryReadModel {
        config,
        schema,
        channels,
        catalog_only_channels,
        channel_catalog,
        channel_surfaces,
        channel_access_policies,
    }
}

fn build_channel_surface_read_model(
    surface: mvp::channel::ChannelSurface,
) -> GatewayChannelSurfaceReadModel {
    let plugin_bridge_account_summary = plugin_bridge_account_summary(&surface);

    GatewayChannelSurfaceReadModel {
        surface,
        plugin_bridge_account_summary,
    }
}

pub fn build_acp_session_list_read_model(
    config_path: &str,
    matched_count: usize,
    sessions: &[mvp::acp::AcpSessionMetadata],
) -> GatewayAcpSessionListReadModel {
    let config = config_path.to_owned();
    let returned_count = sessions.len();
    let sessions = sessions
        .iter()
        .map(build_acp_session_metadata_read_model)
        .collect();

    GatewayAcpSessionListReadModel {
        config,
        matched_count,
        returned_count,
        sessions,
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
    let tool_calling = build_tool_calling_read_model(&snapshot.tool_calling);
    let tools = GatewayRuntimeSnapshotToolsReadModel {
        visible_tool_count,
        visible_tool_names,
        capability_snapshot_sha256,
        capability_snapshot,
        tool_calling,
    };
    let runtime_plugins = crate::runtime_snapshot_runtime_plugins_json(&snapshot.runtime_plugins);
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
        runtime_plugins,
        external_skills,
    }
}

pub fn build_operator_summary_read_model(
    owner_status: &GatewayOwnerStatus,
    channel_inventory: &GatewayChannelInventoryReadModel,
    runtime_snapshot: &GatewayRuntimeSnapshotReadModel,
) -> GatewayOperatorSummaryReadModel {
    let owner = owner_status.clone();
    let control_surface = build_operator_control_surface_read_model(owner_status);
    let channels = build_operator_channels_summary_read_model(channel_inventory, runtime_snapshot);
    let runtime = build_operator_runtime_summary_read_model(runtime_snapshot);

    GatewayOperatorSummaryReadModel {
        owner,
        control_surface,
        channels,
        runtime,
    }
}

fn build_acp_binding_scope_read_model(
    binding: &mvp::acp::AcpSessionBindingScope,
) -> GatewayAcpBindingScopeReadModel {
    let route_session_id = binding.route_session_id.clone();
    let channel_id = binding.channel_id.clone();
    let account_id = binding.account_id.clone();
    let conversation_id = binding.conversation_id.clone();
    let participant_id = binding.participant_id.clone();
    let thread_id = binding.thread_id.clone();

    GatewayAcpBindingScopeReadModel {
        route_session_id,
        channel_id,
        account_id,
        conversation_id,
        participant_id,
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

fn build_acp_session_metadata_read_model(
    metadata: &mvp::acp::AcpSessionMetadata,
) -> GatewayAcpSessionMetadataReadModel {
    let session_key = metadata.session_key.clone();
    let conversation_id = metadata.conversation_id.clone();
    let binding = metadata
        .binding
        .as_ref()
        .map(build_acp_binding_scope_read_model);
    let activation_origin = metadata
        .activation_origin
        .map(mvp::acp::AcpRoutingOrigin::as_str);
    let provenance = build_acp_session_activation_provenance_read_model(metadata.activation_origin);
    let backend_id = metadata.backend_id.clone();
    let runtime_session_name = metadata.runtime_session_name.clone();
    let working_directory = metadata
        .working_directory
        .as_ref()
        .map(|path| path.display().to_string());
    let backend_session_id = metadata.backend_session_id.clone();
    let agent_session_id = metadata.agent_session_id.clone();
    let mode = metadata.mode.map(crate::acp_session_mode_label);
    let state = crate::acp_session_state_label(metadata.state);
    let last_activity_ms = metadata.last_activity_ms;
    let last_error = metadata.last_error.clone();

    GatewayAcpSessionMetadataReadModel {
        session_key,
        conversation_id,
        binding,
        activation_origin,
        provenance,
        backend_id,
        runtime_session_name,
        working_directory,
        backend_session_id,
        agent_session_id,
        mode,
        state,
        last_activity_ms,
        last_error,
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
    let participant_id = address.participant_id.clone();
    let thread_id = address.thread_id.clone();

    GatewayConversationAddressReadModel {
        session_id,
        channel_id,
        account_id,
        conversation_id,
        participant_id,
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
    let participant_id = target.participant_id.clone();
    let thread_id = target.thread_id.clone();
    let channel_path = target.channel_path.clone();

    GatewayAcpDispatchTargetReadModel {
        original_session_id,
        route_session_id,
        prefixed_agent_id,
        channel_id,
        account_id,
        conversation_id,
        participant_id,
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

fn build_operator_control_surface_read_model(
    owner_status: &GatewayOwnerStatus,
) -> GatewayOperatorControlSurfaceReadModel {
    let base_url = gateway_owner_base_url(owner_status);
    let loopback_only = gateway_owner_control_is_loopback(owner_status);

    GatewayOperatorControlSurfaceReadModel {
        base_url,
        loopback_only,
    }
}

fn build_operator_channels_summary_read_model(
    channel_inventory: &GatewayChannelInventoryReadModel,
    runtime_snapshot: &GatewayRuntimeSnapshotReadModel,
) -> GatewayOperatorChannelsSummaryReadModel {
    let catalog_channel_count = channel_inventory.channel_catalog.len();
    let configured_channel_count = channel_inventory
        .channel_surfaces
        .iter()
        .filter(|surface| !surface.surface.configured_accounts.is_empty())
        .count();
    let configured_account_count = channel_inventory.channels.len();
    let enabled_account_count = channel_inventory
        .channels
        .iter()
        .filter(|account| account.enabled)
        .count();
    let misconfigured_account_count = channel_inventory
        .channels
        .iter()
        .filter(|account| channel_account_is_misconfigured(account))
        .count();
    let runtime_backed_channel_count = channel_inventory
        .channel_catalog
        .iter()
        .filter(|channel| {
            channel.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
        })
        .count();
    let enabled_service_channel_ids = &runtime_snapshot.channels.enabled_service_channel_ids;
    let enabled_service_channel_count = enabled_service_channel_ids.len();
    let surfaces = build_operator_channel_surface_read_models(
        &channel_inventory.channel_surfaces,
        &channel_inventory.channel_access_policies,
        enabled_service_channel_ids,
    );
    let ready_service_channel_count = surfaces
        .iter()
        .filter(|surface| surface.service_ready)
        .count();

    GatewayOperatorChannelsSummaryReadModel {
        catalog_channel_count,
        configured_channel_count,
        configured_account_count,
        enabled_account_count,
        misconfigured_account_count,
        runtime_backed_channel_count,
        enabled_service_channel_count,
        ready_service_channel_count,
        surfaces,
    }
}

fn build_operator_channel_surface_read_models(
    channel_surfaces: &[GatewayChannelSurfaceReadModel],
    channel_access_policies: &[mvp::channel::ChannelConfiguredAccountAccessPolicy],
    enabled_service_channel_ids: &[String],
) -> Vec<GatewayOperatorChannelSurfaceReadModel> {
    let mut surfaces = Vec::with_capacity(channel_surfaces.len());

    for channel_surface in channel_surfaces {
        let surface = build_operator_channel_surface_read_model(
            channel_surface,
            channel_access_policies,
            enabled_service_channel_ids,
        );
        surfaces.push(surface);
    }

    surfaces
}

fn build_operator_channel_surface_read_model(
    channel_surface: &GatewayChannelSurfaceReadModel,
    channel_access_policies: &[mvp::channel::ChannelConfiguredAccountAccessPolicy],
    enabled_service_channel_ids: &[String],
) -> GatewayOperatorChannelSurfaceReadModel {
    let surface = &channel_surface.surface;
    let channel_id = surface.catalog.id.to_owned();
    let label = surface.catalog.label.to_owned();
    let implementation_status = surface.catalog.implementation_status.as_str().to_owned();
    let configured_account_count = surface.configured_accounts.len();
    let enabled_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| account.enabled)
        .count();
    let misconfigured_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| channel_account_is_misconfigured(account))
        .count();
    let ready_send_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| {
            channel_account_operation_is_ready(account, mvp::channel::CHANNEL_OPERATION_SEND_ID)
        })
        .count();
    let ready_serve_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| {
            channel_account_operation_is_ready(account, mvp::channel::CHANNEL_OPERATION_SERVE_ID)
        })
        .count();
    let conversation_gated_account_count = channel_access_policies
        .iter()
        .filter(|policy| policy.channel_id == surface.catalog.id)
        .filter(|policy| {
            policy.summary.conversation_mode != mvp::channel::ChannelAccessRestrictionMode::Open
        })
        .count();
    let sender_gated_account_count = channel_access_policies
        .iter()
        .filter(|policy| policy.channel_id == surface.catalog.id)
        .filter(|policy| {
            policy.summary.sender_mode != mvp::channel::ChannelAccessRestrictionMode::Open
        })
        .count();
    let mention_gated_account_count = channel_access_policies
        .iter()
        .filter(|policy| policy.channel_id == surface.catalog.id)
        .filter(|policy| policy.summary.mention_required)
        .count();
    let default_configured_account_id = surface.default_configured_account_id.clone();
    let plugin_bridge_account_summary = channel_surface.plugin_bridge_account_summary.clone();
    let service_enabled = enabled_service_channel_ids.contains(&channel_id);
    let service_ready = service_enabled && ready_serve_account_count > 0;

    GatewayOperatorChannelSurfaceReadModel {
        channel_id,
        label,
        implementation_status,
        configured_account_count,
        enabled_account_count,
        misconfigured_account_count,
        ready_send_account_count,
        ready_serve_account_count,
        conversation_gated_account_count,
        sender_gated_account_count,
        mention_gated_account_count,
        default_configured_account_id,
        plugin_bridge_account_summary,
        service_enabled,
        service_ready,
    }
}

fn build_operator_runtime_summary_read_model(
    runtime_snapshot: &GatewayRuntimeSnapshotReadModel,
) -> GatewayOperatorRuntimeSummaryReadModel {
    let enabled_channel_ids = runtime_snapshot.channels.enabled_channel_ids.clone();
    let enabled_service_channel_ids = runtime_snapshot
        .channels
        .enabled_service_channel_ids
        .clone();
    let visible_tool_count = runtime_snapshot.tools.visible_tool_count;
    let capability_snapshot_sha256 = runtime_snapshot.tools.capability_snapshot_sha256.clone();
    let active_provider_profile_id =
        json_string_field(&runtime_snapshot.provider, "active_profile_id");
    let active_provider_label = json_string_field(&runtime_snapshot.provider, "active_label");
    let tool_calling = runtime_snapshot.tools.tool_calling.clone();

    GatewayOperatorRuntimeSummaryReadModel {
        enabled_channel_ids,
        enabled_service_channel_ids,
        visible_tool_count,
        capability_snapshot_sha256,
        active_provider_profile_id,
        active_provider_label,
        tool_calling,
    }
}

fn build_tool_calling_read_model(
    state: &crate::RuntimeSnapshotToolCallingState,
) -> GatewayToolCallingReadModel {
    GatewayToolCallingReadModel {
        availability: state.availability.clone(),
        structured_tool_schema_enabled: state.structured_tool_schema_enabled,
        effective_tool_schema_mode: state.effective_tool_schema_mode.clone(),
        active_model: state.active_model.clone(),
        reason: state.reason.clone(),
    }
}

fn channel_account_is_misconfigured(account: &mvp::channel::ChannelStatusSnapshot) -> bool {
    account
        .operations
        .iter()
        .any(|operation| operation.health == mvp::channel::ChannelOperationHealth::Misconfigured)
}

fn channel_account_operation_is_ready(
    account: &mvp::channel::ChannelStatusSnapshot,
    operation_id: &str,
) -> bool {
    let operation = account.operation(operation_id);
    let Some(operation) = operation else {
        return false;
    };

    operation.health == mvp::channel::ChannelOperationHealth::Ready
}

fn gateway_owner_base_url(owner_status: &GatewayOwnerStatus) -> Option<String> {
    let bind_address = owner_status.bind_address.as_deref()?;
    let port = owner_status.port?;
    let ip_address = bind_address.parse::<IpAddr>().ok()?;
    let socket_address = SocketAddr::new(ip_address, port);
    let base_url = format!("http://{socket_address}");
    Some(base_url)
}

fn gateway_owner_control_is_loopback(owner_status: &GatewayOwnerStatus) -> bool {
    let bind_address = owner_status.bind_address.as_deref();
    let Some(bind_address) = bind_address else {
        return false;
    };

    let ip_address = bind_address.parse::<IpAddr>();
    let Ok(ip_address) = ip_address else {
        return false;
    };

    ip_address.is_loopback()
}

fn json_string_field(value: &Value, field: &str) -> Option<String> {
    let object = value.as_object()?;
    let value = object.get(field)?;
    let text = value.as_str()?;
    Some(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_channel_surface_read_model_keeps_plugin_backed_summary_context() {
        let config: mvp::config::LoongClawConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "default_account": "ops",
                "accounts": {
                    "ops": {
                        "enabled": true,
                        "bridge_url": "https://bridge.example.test/ops",
                        "bridge_access_token": "ops-token",
                        "allowed_contact_ids": ["wxid_ops"]
                    },
                    "backup": {
                        "enabled": true,
                        "bridge_access_token": "backup-token",
                        "allowed_contact_ids": ["wxid_backup"]
                    }
                }
            }
        }))
        .expect("deserialize weixin config");
        let inventory = mvp::channel::channel_inventory(&config);
        let surface = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "weixin")
            .expect("weixin surface");
        let read_model = build_channel_surface_read_model(surface.clone());
        let operator_surface = build_operator_channel_surface_read_model(
            &read_model,
            &inventory.channel_access_policies,
            &Vec::new(),
        );

        assert_eq!(operator_surface.channel_id, "weixin");
        assert_eq!(operator_surface.implementation_status, "plugin_backed");
        assert_eq!(operator_surface.conversation_gated_account_count, 0);
        assert_eq!(operator_surface.sender_gated_account_count, 0);
        assert_eq!(
            operator_surface.plugin_bridge_account_summary.as_deref(),
            Some(
                "configured_account=ops (default): ready; configured_account=backup: bridge_url is missing"
            )
        );
    }

    #[test]
    fn operator_channel_surface_read_model_keeps_non_plugin_backed_summary_empty() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "123456:test-token".to_owned(),
        ));
        config.telegram.allowed_chat_ids = vec![1];
        let inventory = mvp::channel::channel_inventory(&config);
        let surface = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "telegram")
            .expect("telegram surface");
        let read_model = build_channel_surface_read_model(surface.clone());
        let operator_surface = build_operator_channel_surface_read_model(
            &read_model,
            &inventory.channel_access_policies,
            &Vec::new(),
        );

        assert_eq!(operator_surface.channel_id, "telegram");
        assert_eq!(operator_surface.implementation_status, "runtime_backed");
        assert_eq!(operator_surface.conversation_gated_account_count, 1);
        assert_eq!(operator_surface.sender_gated_account_count, 0);
        assert_eq!(operator_surface.plugin_bridge_account_summary, None);
    }

    #[test]
    fn channel_inventory_read_model_includes_structured_channel_access_policies() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
            "cli_a1b2c3".to_owned(),
        ));
        config.feishu.app_secret =
            Some(loongclaw_contracts::SecretRef::Inline("secret".to_owned()));
        config.feishu.allowed_chat_ids = vec!["*".to_owned()];
        config.feishu.allowed_sender_ids = vec!["ou_admin".to_owned()];

        let inventory = mvp::channel::channel_inventory(&config);
        let read_model = build_channel_inventory_read_model("/tmp/loongclaw.toml", &inventory);
        let access_policy = read_model
            .channel_access_policies
            .iter()
            .find(|policy| policy.channel_id == "feishu")
            .expect("feishu access policy");

        assert_eq!(access_policy.conversation_config_key, "allowed_chat_ids");
        assert_eq!(access_policy.sender_config_key, "allowed_sender_ids");
        assert_eq!(
            access_policy.summary.conversation_mode,
            mvp::channel::ChannelAccessRestrictionMode::WildcardAllowlist
        );
        assert_eq!(
            access_policy.summary.allowed_conversations,
            vec!["*".to_owned()]
        );
        assert_eq!(
            access_policy.summary.allowed_senders,
            vec!["ou_admin".to_owned()]
        );
    }
}
