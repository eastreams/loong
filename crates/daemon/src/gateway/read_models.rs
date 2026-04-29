use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};

use loong_protocol::{
    CONTROL_PLANE_PROTOCOL_VERSION, ControlPlaneChallengeResponse, ControlPlanePrincipal,
    ControlPlaneRole, ControlPlaneScope,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION;
use crate::RuntimeSnapshotCliState;
use crate::mvp;
use crate::operator_inventory_cli::{
    CHANNELS_CLI_JSON_LEGACY_VIEWS, CHANNELS_CLI_JSON_SCHEMA_VERSION,
};
use crate::plugin_bridge_account_summary::plugin_bridge_account_summary;

use super::event_bus::{GatewayEventRecord, GatewayEventReplayWindow};
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
    pub summary: GatewayChannelInventorySummaryReadModel,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayChannelInventorySummaryReadModel {
    pub total_surface_count: usize,
    pub runtime_backed_surface_count: usize,
    pub config_backed_surface_count: usize,
    pub plugin_backed_surface_count: usize,
    pub catalog_only_surface_count: usize,
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
    pub enabled_runtime_backed_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub enabled_plugin_backed_channel_ids: Vec<String>,
    pub enabled_outbound_only_channel_ids: Vec<String>,
    pub inventory: GatewayChannelInventoryReadModel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayWebAccessReadModel {
    pub ordinary_network_access_enabled: bool,
    pub query_search_enabled: bool,
    pub query_search_default_provider: String,
    pub query_search_credential_ready: bool,
    pub separation_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayRuntimeSnapshotToolsReadModel {
    pub visible_tool_count: usize,
    pub visible_tool_names: Vec<String>,
    pub visible_direct_tool_names: Vec<String>,
    pub hidden_tool_count: usize,
    pub hidden_tool_tags: Vec<String>,
    pub hidden_tool_surfaces: Vec<GatewayToolSurfaceReadModel>,
    pub capability_snapshot_sha256: String,
    pub capability_snapshot: String,
    pub tool_calling: GatewayToolCallingReadModel,
    pub web_access: GatewayWebAccessReadModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayToolSurfaceReadModel {
    pub surface_id: String,
    pub prompt_snippet: String,
    pub usage_guidance: String,
    pub tool_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_tool_names: Vec<String>,
    pub tool_ids: Vec<String>,
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
pub struct GatewayOperatorRuntimeIncidentReadModel {
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub kind: String,
    pub at_ms: u64,
    pub detail: Option<String>,
    pub owner_pids: Vec<u32>,
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
    pub runtime_attention_account_count: usize,
    pub runtime_attention_reasons: Vec<String>,
    pub runtime_attention_remediations: Vec<String>,
    pub retrying_runtime_account_count: usize,
    pub stale_runtime_account_count: usize,
    pub duplicate_runtime_account_count: usize,
    pub preferred_runtime_owner_pids: Vec<u32>,
    pub duplicate_runtime_cleanup_owner_pids: Vec<u32>,
    pub last_duplicate_runtime_auto_reclaim_at: Option<u64>,
    pub last_duplicate_runtime_auto_cleanup_owner_pids: Vec<u32>,
    pub recent_runtime_incidents: Vec<GatewayOperatorRuntimeIncidentReadModel>,
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
    pub config_backed_channel_count: usize,
    pub plugin_backed_channel_count: usize,
    pub catalog_only_channel_count: usize,
    pub enabled_runtime_backed_channel_count: usize,
    pub enabled_plugin_backed_channel_count: usize,
    pub enabled_outbound_only_channel_count: usize,
    pub enabled_service_channel_count: usize,
    pub ready_service_channel_count: usize,
    pub runtime_attention_surface_count: usize,
    pub retrying_runtime_surface_count: usize,
    pub stale_runtime_surface_count: usize,
    pub duplicate_runtime_surface_count: usize,
    pub runtime_attention_surface_ids: Vec<String>,
    pub retrying_runtime_surface_ids: Vec<String>,
    pub stale_runtime_surface_ids: Vec<String>,
    pub duplicate_runtime_surface_ids: Vec<String>,
    pub surfaces: Vec<GatewayOperatorChannelSurfaceReadModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorRuntimeSummaryReadModel {
    pub enabled_channel_ids: Vec<String>,
    pub enabled_runtime_backed_channel_ids: Vec<String>,
    pub enabled_service_channel_ids: Vec<String>,
    pub enabled_plugin_backed_channel_ids: Vec<String>,
    pub enabled_outbound_only_channel_ids: Vec<String>,
    pub visible_tool_count: usize,
    pub visible_direct_tool_names: Vec<String>,
    pub hidden_tool_surface_ids: Vec<String>,
    pub capability_snapshot_sha256: String,
    pub active_provider_profile_id: Option<String>,
    pub active_provider_label: Option<String>,
    pub compaction_hygiene: crate::RuntimeSnapshotCompactionHygieneState,
    pub tool_calling: GatewayToolCallingReadModel,
    pub web_access: GatewayWebAccessReadModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorPairingSummaryReadModel {
    pub pending_request_count: usize,
    pub approved_device_count: usize,
    pub last_activity_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayNodeInventorySummaryReadModel {
    pub paired_device_count: usize,
    pub managed_bridge_count: usize,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOperatorNodesSummaryReadModel {
    pub paired_device_count: usize,
    pub managed_bridge_count: usize,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPairedDeviceNodeReadModel {
    pub node_id: String,
    pub node_kind: String,
    pub trust_state: String,
    pub role: String,
    pub public_key: String,
    pub approved_scopes: Vec<String>,
    pub issued_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayManagedBridgeNodeReadModel {
    pub node_id: String,
    pub node_kind: String,
    pub trust_state: String,
    pub channel_id: String,
    pub implementation_status: String,
    pub configured_account_count: usize,
    pub enabled_account_count: usize,
    pub configured_plugin_id: Option<String>,
    pub selected_plugin_id: Option<String>,
    pub discovery_status: Option<String>,
    pub selection_status: Option<String>,
    pub compatible_plugins: usize,
    pub incomplete_plugins: usize,
    pub incompatible_plugins: usize,
    pub account_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayNodeInventoryReadModel {
    pub config: String,
    pub summary: GatewayNodeInventorySummaryReadModel,
    pub paired_devices: Vec<GatewayPairedDeviceNodeReadModel>,
    pub managed_bridges: Vec<GatewayManagedBridgeNodeReadModel>,
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
    pub pairing: GatewayOperatorPairingSummaryReadModel,
    pub nodes: GatewayOperatorNodesSummaryReadModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPairingStartReadModel {
    pub protocol: u32,
    pub challenge: ControlPlaneChallengeResponse,
    pub connect_path: String,
    pub pairing_requests_path: String,
    pub pairing_resolve_path: String,
    pub recommended_role: ControlPlaneRole,
    pub recommended_scopes: Vec<ControlPlaneScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPairingCompleteReadModel {
    pub status: String,
    pub device_id: String,
    pub client_id: String,
    pub role: ControlPlaneRole,
    pub requested_scopes: Vec<ControlPlaneScope>,
    pub lease: GatewayPairingSessionLeaseReadModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPairingSessionLeaseReadModel {
    pub connection_token: String,
    pub connection_token_expires_at_ms: u64,
    pub principal: ControlPlanePrincipal,
    pub last_acknowledged_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPairingReplayWindowReadModel {
    pub oldest_retained_seq: Option<u64>,
    pub latest_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPairingSessionReadModel {
    pub status: String,
    pub connection_token_expires_at_ms: u64,
    pub principal: ControlPlanePrincipal,
    pub last_acknowledged_seq: Option<u64>,
    pub resume_status: String,
    pub resume_from_after_seq: u64,
    pub earliest_resumable_after_seq: u64,
    pub replay_window: GatewayPairingReplayWindowReadModel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPairingEventsReadModel {
    pub after_seq: u64,
    pub effective_after_seq: u64,
    pub returned_count: usize,
    pub last_acknowledged_seq: Option<u64>,
    pub resume_status: String,
    pub next_after_seq: u64,
    pub earliest_resumable_after_seq: u64,
    pub replay_window: GatewayPairingReplayWindowReadModel,
    pub events: Vec<GatewayEventRecord>,
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
    let summary = build_channel_inventory_summary_read_model(&inventory.channel_surfaces);
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
        summary,
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

fn build_channel_inventory_summary_read_model(
    channel_surfaces: &[mvp::channel::ChannelSurface],
) -> GatewayChannelInventorySummaryReadModel {
    let total_surface_count = channel_surfaces.len();
    let runtime_backed_surface_count = channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
        })
        .count();
    let config_backed_surface_count = channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
        })
        .count();
    let plugin_backed_surface_count = channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
        })
        .count();
    let catalog_only_surface_count = channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::Stub
        })
        .count();

    GatewayChannelInventorySummaryReadModel {
        total_surface_count,
        runtime_backed_surface_count,
        config_backed_surface_count,
        plugin_backed_surface_count,
        catalog_only_surface_count,
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
    let context_engine = crate::runtime_snapshot_context_engine_json(
        &snapshot.context_engine,
        &snapshot.compaction_hygiene,
    );
    let memory_system = crate::runtime_snapshot_memory_system_json(&snapshot.memory_system);
    let acp = crate::runtime_snapshot_acp_json(&snapshot.acp);
    let inventory = build_channel_inventory_read_model(config.as_str(), &snapshot.channels);
    let enabled_channel_ids = snapshot.enabled_channel_ids.clone();
    let enabled_runtime_backed_channel_ids = snapshot.enabled_runtime_backed_channel_ids.clone();
    let enabled_service_channel_ids = snapshot.enabled_service_channel_ids.clone();
    let enabled_plugin_backed_channel_ids = snapshot.enabled_plugin_backed_channel_ids.clone();
    let enabled_outbound_only_channel_ids = snapshot.enabled_outbound_only_channel_ids.clone();
    let channels = GatewayRuntimeSnapshotChannelsReadModel {
        enabled_channel_ids,
        enabled_runtime_backed_channel_ids,
        enabled_service_channel_ids,
        enabled_plugin_backed_channel_ids,
        enabled_outbound_only_channel_ids,
        inventory,
    };
    let tool_runtime = crate::runtime_snapshot_tool_runtime_json(&snapshot.tool_runtime);
    let visible_tool_count = snapshot.visible_tool_names.len();
    let visible_tool_names = snapshot.visible_tool_names.clone();
    let visible_direct_tool_names = snapshot
        .discoverable_tool_summary
        .visible_direct_tools
        .clone();
    let hidden_tool_count = snapshot.discoverable_tool_summary.hidden_tool_count;
    let hidden_tool_tags = snapshot.discoverable_tool_summary.hidden_tags.clone();
    let hidden_tool_surfaces = snapshot
        .discoverable_tool_summary
        .hidden_surfaces
        .iter()
        .map(build_tool_surface_read_model)
        .collect::<Vec<_>>();
    let capability_snapshot_sha256 = snapshot.capability_snapshot_sha256.clone();
    let capability_snapshot = snapshot.capability_snapshot.clone();
    let tool_calling = build_tool_calling_read_model(&snapshot.tool_calling);
    let web_access = build_web_access_read_model(&snapshot.tool_runtime);
    let tools = GatewayRuntimeSnapshotToolsReadModel {
        visible_tool_count,
        visible_tool_names,
        visible_direct_tool_names,
        hidden_tool_count,
        hidden_tool_tags,
        hidden_tool_surfaces,
        capability_snapshot_sha256,
        capability_snapshot,
        tool_calling,
        web_access,
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
    pairing: GatewayOperatorPairingSummaryReadModel,
    nodes: GatewayOperatorNodesSummaryReadModel,
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
        pairing,
        nodes,
    }
}

pub fn build_node_inventory_read_model(
    config_path: &str,
    channel_inventory: &GatewayChannelInventoryReadModel,
    paired_devices: &[mvp::control_plane::ControlPlaneApprovedDeviceSummary],
) -> GatewayNodeInventoryReadModel {
    let config = config_path.to_owned();
    let paired_devices = build_paired_device_nodes_read_model(paired_devices);
    let managed_bridges = build_managed_bridge_nodes_read_model(channel_inventory);
    let summary = GatewayNodeInventorySummaryReadModel {
        paired_device_count: paired_devices.len(),
        managed_bridge_count: managed_bridges.len(),
        total_count: paired_devices.len() + managed_bridges.len(),
    };

    GatewayNodeInventoryReadModel {
        config,
        summary,
        paired_devices,
        managed_bridges,
    }
}

pub fn build_operator_nodes_summary_read_model(
    inventory: &GatewayNodeInventoryReadModel,
) -> GatewayOperatorNodesSummaryReadModel {
    GatewayOperatorNodesSummaryReadModel {
        paired_device_count: inventory.summary.paired_device_count,
        managed_bridge_count: inventory.summary.managed_bridge_count,
        total_count: inventory.summary.total_count,
    }
}

pub fn build_gateway_pairing_start_read_model(
    challenge: ControlPlaneChallengeResponse,
) -> GatewayPairingStartReadModel {
    GatewayPairingStartReadModel {
        protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        challenge,
        connect_path: "/v1/pairing/complete".to_owned(),
        pairing_requests_path: "/v1/pairing/requests".to_owned(),
        pairing_resolve_path: "/v1/pairing/resolve".to_owned(),
        recommended_role: ControlPlaneRole::Operator,
        recommended_scopes: vec![
            ControlPlaneScope::OperatorRead,
            ControlPlaneScope::OperatorPairing,
        ],
    }
}

pub fn build_gateway_pairing_complete_read_model(
    device_id: &str,
    client_id: &str,
    role: ControlPlaneRole,
    requested_scopes: Vec<ControlPlaneScope>,
    lease: GatewayPairingSessionLeaseReadModel,
) -> GatewayPairingCompleteReadModel {
    GatewayPairingCompleteReadModel {
        status: "authorized".to_owned(),
        device_id: device_id.to_owned(),
        client_id: client_id.to_owned(),
        role,
        requested_scopes,
        lease,
    }
}

pub fn build_gateway_pairing_session_read_model(
    lease: GatewayPairingSessionLeaseReadModel,
    replay_window: GatewayEventReplayWindow,
) -> GatewayPairingSessionReadModel {
    let earliest_resumable_after_seq = replay_window
        .oldest_retained_seq
        .map(|seq| seq.saturating_sub(1))
        .unwrap_or(0);
    let (resume_status, resume_from_after_seq) =
        gateway_pairing_resume_contract(lease.last_acknowledged_seq, replay_window);

    GatewayPairingSessionReadModel {
        status: "active".to_owned(),
        connection_token_expires_at_ms: lease.connection_token_expires_at_ms,
        principal: lease.principal,
        last_acknowledged_seq: lease.last_acknowledged_seq,
        resume_status: resume_status.to_owned(),
        resume_from_after_seq,
        earliest_resumable_after_seq,
        replay_window: build_gateway_pairing_replay_window_read_model(replay_window),
    }
}

pub fn build_gateway_pairing_events_read_model(
    after_seq: u64,
    last_acknowledged_seq: Option<u64>,
    replay_window: GatewayEventReplayWindow,
    events: Vec<GatewayEventRecord>,
) -> GatewayPairingEventsReadModel {
    let returned_count = events.len();
    let next_after_seq = events.last().map(|event| event.seq).unwrap_or(after_seq);
    let earliest_resumable_after_seq = replay_window
        .oldest_retained_seq
        .map(|seq| seq.saturating_sub(1))
        .unwrap_or(0);
    let resume_status = if after_seq == 0 { "fresh" } else { "resumed" };

    GatewayPairingEventsReadModel {
        after_seq,
        effective_after_seq: after_seq,
        returned_count,
        last_acknowledged_seq,
        resume_status: resume_status.to_owned(),
        next_after_seq,
        earliest_resumable_after_seq,
        replay_window: build_gateway_pairing_replay_window_read_model(replay_window),
        events,
    }
}

pub fn build_gateway_pairing_replay_window_read_model(
    replay_window: GatewayEventReplayWindow,
) -> GatewayPairingReplayWindowReadModel {
    GatewayPairingReplayWindowReadModel {
        oldest_retained_seq: replay_window.oldest_retained_seq,
        latest_seq: replay_window.latest_seq,
    }
}

fn gateway_pairing_resume_contract(
    last_acknowledged_seq: Option<u64>,
    replay_window: GatewayEventReplayWindow,
) -> (&'static str, u64) {
    let earliest_resumable_after_seq = replay_window
        .oldest_retained_seq
        .map(|seq| seq.saturating_sub(1))
        .unwrap_or(0);
    let Some(last_acknowledged_seq) = last_acknowledged_seq else {
        return ("fresh", 0);
    };

    if replay_window.oldest_retained_seq.is_some()
        && last_acknowledged_seq < earliest_resumable_after_seq
    {
        return ("stale", earliest_resumable_after_seq);
    }

    ("resumed", last_acknowledged_seq)
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
    let config_backed_channel_count = channel_inventory
        .channel_catalog
        .iter()
        .filter(|channel| {
            channel.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
        })
        .count();
    let plugin_backed_channel_count = channel_inventory
        .channel_catalog
        .iter()
        .filter(|channel| {
            channel.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
        })
        .count();
    let catalog_only_channel_count = channel_inventory
        .channel_catalog
        .iter()
        .filter(|channel| {
            channel.implementation_status == mvp::channel::ChannelCatalogImplementationStatus::Stub
        })
        .count();
    let enabled_runtime_backed_channel_ids =
        &runtime_snapshot.channels.enabled_runtime_backed_channel_ids;
    let enabled_plugin_backed_channel_ids =
        &runtime_snapshot.channels.enabled_plugin_backed_channel_ids;
    let enabled_outbound_only_channel_ids =
        &runtime_snapshot.channels.enabled_outbound_only_channel_ids;
    let enabled_service_channel_ids = &runtime_snapshot.channels.enabled_service_channel_ids;
    let enabled_runtime_backed_channel_count = enabled_runtime_backed_channel_ids.len();
    let enabled_plugin_backed_channel_count = enabled_plugin_backed_channel_ids.len();
    let enabled_outbound_only_channel_count = enabled_outbound_only_channel_ids.len();
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
    let runtime_attention_surface_count = surfaces
        .iter()
        .filter(|surface| surface.runtime_attention_account_count > 0)
        .count();
    let retrying_runtime_surface_count = surfaces
        .iter()
        .filter(|surface| surface.retrying_runtime_account_count > 0)
        .count();
    let stale_runtime_surface_count = surfaces
        .iter()
        .filter(|surface| surface.stale_runtime_account_count > 0)
        .count();
    let duplicate_runtime_surface_count = surfaces
        .iter()
        .filter(|surface| surface.duplicate_runtime_account_count > 0)
        .count();
    let runtime_attention_surface_ids = surfaces
        .iter()
        .filter(|surface| surface.runtime_attention_account_count > 0)
        .map(|surface| surface.channel_id.clone())
        .collect::<Vec<_>>();
    let retrying_runtime_surface_ids = surfaces
        .iter()
        .filter(|surface| surface.retrying_runtime_account_count > 0)
        .map(|surface| surface.channel_id.clone())
        .collect::<Vec<_>>();
    let stale_runtime_surface_ids = surfaces
        .iter()
        .filter(|surface| surface.stale_runtime_account_count > 0)
        .map(|surface| surface.channel_id.clone())
        .collect::<Vec<_>>();
    let duplicate_runtime_surface_ids = surfaces
        .iter()
        .filter(|surface| surface.duplicate_runtime_account_count > 0)
        .map(|surface| surface.channel_id.clone())
        .collect::<Vec<_>>();

    GatewayOperatorChannelsSummaryReadModel {
        catalog_channel_count,
        configured_channel_count,
        configured_account_count,
        enabled_account_count,
        misconfigured_account_count,
        runtime_backed_channel_count,
        config_backed_channel_count,
        plugin_backed_channel_count,
        catalog_only_channel_count,
        enabled_runtime_backed_channel_count,
        enabled_plugin_backed_channel_count,
        enabled_outbound_only_channel_count,
        enabled_service_channel_count,
        ready_service_channel_count,
        runtime_attention_surface_count,
        retrying_runtime_surface_count,
        stale_runtime_surface_count,
        duplicate_runtime_surface_count,
        runtime_attention_surface_ids,
        retrying_runtime_surface_ids,
        stale_runtime_surface_ids,
        duplicate_runtime_surface_ids,
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
    let runtime_attention_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| channel_account_has_runtime_attention(account))
        .count();
    let runtime_attention_reasons = collect_channel_surface_runtime_attention_reasons(surface);
    let runtime_attention_remediations = runtime_attention_reasons
        .iter()
        .map(|reason| runtime_attention_reason_remediation(reason.as_str()).to_owned())
        .collect::<Vec<_>>();
    let retrying_runtime_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| channel_account_has_retrying_runtime(account))
        .count();
    let stale_runtime_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| channel_account_has_stale_runtime(account))
        .count();
    let duplicate_runtime_account_count = surface
        .configured_accounts
        .iter()
        .filter(|account| channel_account_has_duplicate_runtime(account))
        .count();
    let preferred_runtime_owner_pids =
        collect_channel_surface_preferred_runtime_owner_pids(surface);
    let duplicate_runtime_cleanup_owner_pids =
        collect_channel_surface_duplicate_runtime_cleanup_owner_pids(surface);
    let last_duplicate_runtime_auto_reclaim_at =
        collect_channel_surface_last_duplicate_runtime_auto_reclaim_at(surface);
    let last_duplicate_runtime_auto_cleanup_owner_pids =
        collect_channel_surface_last_duplicate_runtime_auto_cleanup_owner_pids(surface);
    let recent_runtime_incidents = collect_channel_surface_recent_runtime_incidents(surface);
    let service_enabled = enabled_service_channel_ids.contains(&channel_id);
    let service_ready =
        service_enabled && ready_serve_account_count > 0 && runtime_attention_account_count == 0;

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
        runtime_attention_account_count,
        runtime_attention_reasons,
        runtime_attention_remediations,
        retrying_runtime_account_count,
        stale_runtime_account_count,
        duplicate_runtime_account_count,
        preferred_runtime_owner_pids,
        duplicate_runtime_cleanup_owner_pids,
        last_duplicate_runtime_auto_reclaim_at,
        last_duplicate_runtime_auto_cleanup_owner_pids,
        recent_runtime_incidents,
        service_enabled,
        service_ready,
    }
}

fn build_operator_runtime_summary_read_model(
    runtime_snapshot: &GatewayRuntimeSnapshotReadModel,
) -> GatewayOperatorRuntimeSummaryReadModel {
    let enabled_channel_ids = runtime_snapshot.channels.enabled_channel_ids.clone();
    let enabled_runtime_backed_channel_ids = runtime_snapshot
        .channels
        .enabled_runtime_backed_channel_ids
        .clone();
    let enabled_service_channel_ids = runtime_snapshot
        .channels
        .enabled_service_channel_ids
        .clone();
    let enabled_plugin_backed_channel_ids = runtime_snapshot
        .channels
        .enabled_plugin_backed_channel_ids
        .clone();
    let enabled_outbound_only_channel_ids = runtime_snapshot
        .channels
        .enabled_outbound_only_channel_ids
        .clone();
    let visible_tool_count = runtime_snapshot.tools.visible_tool_count;
    let visible_direct_tool_names = runtime_snapshot.tools.visible_direct_tool_names.clone();
    let hidden_tool_surface_ids = runtime_snapshot
        .tools
        .hidden_tool_surfaces
        .iter()
        .map(|surface| surface.surface_id.clone())
        .collect::<Vec<_>>();
    let capability_snapshot_sha256 = runtime_snapshot.tools.capability_snapshot_sha256.clone();
    let active_provider_profile_id =
        json_string_field(&runtime_snapshot.provider, "active_profile_id");
    let active_provider_label = json_string_field(&runtime_snapshot.provider, "active_label");
    let compaction_hygiene = runtime_snapshot.context_engine.get("compaction_hygiene");
    let compaction_hygiene =
        crate::RuntimeSnapshotCompactionHygieneState::decode_or_unknown(compaction_hygiene);
    let tool_calling = runtime_snapshot.tools.tool_calling.clone();
    let web_access = runtime_snapshot.tools.web_access.clone();

    GatewayOperatorRuntimeSummaryReadModel {
        enabled_channel_ids,
        enabled_runtime_backed_channel_ids,
        enabled_service_channel_ids,
        enabled_plugin_backed_channel_ids,
        enabled_outbound_only_channel_ids,
        visible_tool_count,
        visible_direct_tool_names,
        hidden_tool_surface_ids,
        capability_snapshot_sha256,
        active_provider_profile_id,
        active_provider_label,
        compaction_hygiene,
        tool_calling,
        web_access,
    }
}

fn build_paired_device_nodes_read_model(
    paired_devices: &[mvp::control_plane::ControlPlaneApprovedDeviceSummary],
) -> Vec<GatewayPairedDeviceNodeReadModel> {
    paired_devices
        .iter()
        .map(|device| GatewayPairedDeviceNodeReadModel {
            node_id: device.device_id.clone(),
            node_kind: paired_device_node_kind(device.role.as_str()).to_owned(),
            trust_state: "paired".to_owned(),
            role: device.role.clone(),
            public_key: device.public_key.clone(),
            approved_scopes: device.approved_scopes.iter().cloned().collect(),
            issued_at_ms: device.issued_at_ms,
        })
        .collect()
}

fn build_managed_bridge_nodes_read_model(
    channel_inventory: &GatewayChannelInventoryReadModel,
) -> Vec<GatewayManagedBridgeNodeReadModel> {
    let mut nodes = channel_inventory
        .channel_surfaces
        .iter()
        .filter_map(|surface| {
            let discovery = surface.surface.plugin_bridge_discovery.as_ref()?;
            let enabled_account_count = surface
                .surface
                .configured_accounts
                .iter()
                .filter(|snapshot| snapshot.enabled)
                .count();
            let has_operator_relevant_surface = enabled_account_count > 0
                || discovery.selected_plugin_id.is_some()
                || discovery.configured_plugin_id.is_some()
                || discovery.compatible_plugins > 0;
            if !has_operator_relevant_surface {
                return None;
            }
            Some(GatewayManagedBridgeNodeReadModel {
                node_id: format!("managed_bridge:{}", surface.surface.catalog.id),
                node_kind: "managed_bridge".to_owned(),
                trust_state: managed_bridge_trust_state(discovery).to_owned(),
                channel_id: surface.surface.catalog.id.to_owned(),
                implementation_status: surface
                    .surface
                    .catalog
                    .implementation_status
                    .as_str()
                    .to_owned(),
                configured_account_count: surface.surface.configured_accounts.len(),
                enabled_account_count,
                configured_plugin_id: discovery.configured_plugin_id.clone(),
                selected_plugin_id: discovery.selected_plugin_id.clone(),
                discovery_status: Some(discovery.status.as_str().to_owned()),
                selection_status: discovery
                    .selection_status
                    .map(|status| status.as_str().to_owned()),
                compatible_plugins: discovery.compatible_plugins,
                incomplete_plugins: discovery.incomplete_plugins,
                incompatible_plugins: discovery.incompatible_plugins,
                account_summary: surface.plugin_bridge_account_summary.clone(),
            })
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
    nodes
}

fn paired_device_node_kind(role: &str) -> &'static str {
    match role {
        "operator" => "operator_ui",
        _ => "node_client",
    }
}

fn managed_bridge_trust_state(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> &'static str {
    use mvp::channel::ChannelPluginBridgeDiscoveryStatus as DiscoveryStatus;

    match discovery.status {
        DiscoveryStatus::NotConfigured => "not_configured",
        DiscoveryStatus::ScanFailed => "scan_failed",
        DiscoveryStatus::NoMatches => "unresolved",
        DiscoveryStatus::MatchesFound => {
            if discovery
                .selection_status
                .is_some_and(|status| status.selects_ready_plugin())
            {
                "ready"
            } else {
                "review_required"
            }
        }
    }
}

fn build_web_access_read_model(
    runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> GatewayWebAccessReadModel {
    let summary = crate::runtime_web_access_summary(runtime);

    GatewayWebAccessReadModel {
        ordinary_network_access_enabled: summary.ordinary_network_access_enabled,
        query_search_enabled: summary.query_search_enabled,
        query_search_default_provider: summary.query_search_default_provider,
        query_search_credential_ready: summary.query_search_credential_ready,
        separation_note: summary.separation_note.to_owned(),
    }
}

fn build_tool_surface_read_model(
    surface: &mvp::tools::ToolSurfaceState,
) -> GatewayToolSurfaceReadModel {
    GatewayToolSurfaceReadModel {
        surface_id: surface.surface_id.clone(),
        prompt_snippet: surface.prompt_snippet.clone(),
        usage_guidance: surface.usage_guidance.clone(),
        tool_count: surface.tool_count(),
        visible_tool_names: visible_tool_names_for_surface(surface),
        tool_ids: surface.tool_ids.clone(),
    }
}

fn visible_tool_names_for_surface(surface: &mvp::tools::ToolSurfaceState) -> Vec<String> {
    let mut visible_tool_names = Vec::new();

    for tool_id in &surface.tool_ids {
        let visible_tool_name = mvp::tools::user_visible_tool_name(tool_id.as_str());
        if !visible_tool_names.contains(&visible_tool_name) {
            visible_tool_names.push(visible_tool_name);
        }
    }

    visible_tool_names
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

fn channel_account_serve_runtime(
    account: &mvp::channel::ChannelStatusSnapshot,
) -> Option<&mvp::channel::ChannelOperationRuntime> {
    account
        .operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID)
        .and_then(|operation| operation.runtime.as_ref())
}

fn channel_account_has_runtime_attention(account: &mvp::channel::ChannelStatusSnapshot) -> bool {
    channel_account_has_retrying_runtime(account)
        || channel_account_has_stale_runtime(account)
        || channel_account_has_duplicate_runtime(account)
}

fn collect_channel_surface_runtime_attention_reasons(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if surface
        .configured_accounts
        .iter()
        .any(channel_account_has_retrying_runtime)
    {
        reasons.push("retrying".to_owned());
    }
    if surface
        .configured_accounts
        .iter()
        .any(channel_account_has_stale_runtime)
    {
        reasons.push("stale".to_owned());
    }
    if surface
        .configured_accounts
        .iter()
        .any(channel_account_has_duplicate_runtime)
    {
        reasons.push("duplicate_runtime_instances".to_owned());
    }

    reasons
}

fn runtime_attention_reason_remediation(reason: &str) -> &'static str {
    match reason {
        "retrying" => "inspect_bridge_connectivity",
        "stale" => "restart_stale_runtime",
        "duplicate_runtime_instances" => "stop_duplicate_runtime_instances",
        _ => "inspect_runtime_attention",
    }
}

fn channel_account_has_retrying_runtime(account: &mvp::channel::ChannelStatusSnapshot) -> bool {
    channel_account_serve_runtime(account)
        .map(|runtime| runtime.running && runtime.consecutive_failures > 0)
        .unwrap_or(false)
}

fn channel_account_has_stale_runtime(account: &mvp::channel::ChannelStatusSnapshot) -> bool {
    channel_account_serve_runtime(account)
        .map(|runtime| runtime.stale)
        .unwrap_or(false)
}

fn channel_account_has_duplicate_runtime(account: &mvp::channel::ChannelStatusSnapshot) -> bool {
    channel_account_serve_runtime(account)
        .map(|runtime| runtime.running_instances > 1)
        .unwrap_or(false)
}

fn collect_channel_surface_preferred_runtime_owner_pids(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<u32> {
    let mut owner_pids = BTreeSet::new();

    for account in &surface.configured_accounts {
        let Some(runtime) = channel_account_serve_runtime(account) else {
            continue;
        };
        if runtime.duplicate_owner_pids.is_empty() {
            continue;
        }
        let Some(pid) = runtime.pid else {
            continue;
        };
        owner_pids.insert(pid);
    }

    owner_pids.into_iter().collect()
}

fn collect_channel_surface_duplicate_runtime_cleanup_owner_pids(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<u32> {
    let mut owner_pids = BTreeSet::new();

    for account in &surface.configured_accounts {
        let Some(runtime) = channel_account_serve_runtime(account) else {
            continue;
        };
        if runtime.duplicate_owner_pids.is_empty() {
            continue;
        }
        let preferred_pid = runtime.pid;
        for owner_pid in &runtime.duplicate_owner_pids {
            if Some(*owner_pid) == preferred_pid {
                continue;
            }
            owner_pids.insert(*owner_pid);
        }
    }

    owner_pids.into_iter().collect()
}

fn collect_channel_surface_last_duplicate_runtime_auto_reclaim_at(
    surface: &mvp::channel::ChannelSurface,
) -> Option<u64> {
    surface
        .configured_accounts
        .iter()
        .filter_map(channel_account_serve_runtime)
        .filter_map(|runtime| runtime.last_duplicate_reclaim_at)
        .max()
}

fn collect_channel_surface_last_duplicate_runtime_auto_cleanup_owner_pids(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<u32> {
    let latest_reclaim_at = collect_channel_surface_last_duplicate_runtime_auto_reclaim_at(surface);
    let Some(latest_reclaim_at) = latest_reclaim_at else {
        return Vec::new();
    };

    let mut owner_pids = BTreeSet::new();
    for runtime in surface
        .configured_accounts
        .iter()
        .filter_map(channel_account_serve_runtime)
        .filter(|runtime| runtime.last_duplicate_reclaim_at == Some(latest_reclaim_at))
    {
        for owner_pid in &runtime.last_duplicate_reclaim_cleanup_owner_pids {
            owner_pids.insert(*owner_pid);
        }
    }

    owner_pids.into_iter().collect()
}

fn collect_channel_surface_recent_runtime_incidents(
    surface: &mvp::channel::ChannelSurface,
) -> Vec<GatewayOperatorRuntimeIncidentReadModel> {
    let mut incidents = surface
        .configured_accounts
        .iter()
        .filter_map(|account| {
            let runtime = channel_account_serve_runtime(account)?;
            Some(
                runtime
                    .recent_incidents
                    .iter()
                    .map(|incident| GatewayOperatorRuntimeIncidentReadModel {
                        account_id: runtime.account_id.clone(),
                        account_label: runtime.account_label.clone(),
                        kind: match incident.kind {
                            mvp::channel::ChannelOperationRuntimeIncidentKind::Failure => {
                                "failure".to_owned()
                            }
                            mvp::channel::ChannelOperationRuntimeIncidentKind::Recovery => {
                                "recovery".to_owned()
                            }
                            mvp::channel::ChannelOperationRuntimeIncidentKind::DuplicateReclaim => {
                                "duplicate_reclaim".to_owned()
                            }
                        },
                        at_ms: incident.at_ms,
                        detail: incident.detail.clone(),
                        owner_pids: incident.owner_pids.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect::<Vec<_>>();

    incidents.sort_by_key(|incident| std::cmp::Reverse(incident.at_ms));
    incidents.truncate(5);
    incidents
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
        let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
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
        assert_eq!(operator_surface.runtime_attention_account_count, 0);
        assert!(operator_surface.runtime_attention_reasons.is_empty());
        assert!(operator_surface.runtime_attention_remediations.is_empty());
        assert_eq!(operator_surface.retrying_runtime_account_count, 0);
        assert_eq!(
            operator_surface.plugin_bridge_account_summary.as_deref(),
            Some(
                "configured_account=ops (default): ready; configured_account=backup: bridge_url is missing"
            )
        );
    }

    #[test]
    fn operator_channel_surface_read_model_keeps_non_plugin_backed_summary_empty() {
        let mut config = mvp::config::LoongConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(loong_contracts::SecretRef::Inline(
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
        assert_eq!(operator_surface.runtime_attention_account_count, 0);
        assert!(operator_surface.runtime_attention_reasons.is_empty());
        assert!(operator_surface.runtime_attention_remediations.is_empty());
        assert_eq!(operator_surface.plugin_bridge_account_summary, None);
    }

    #[test]
    fn operator_channel_surface_read_model_counts_retrying_runtime_attention() {
        let config = mvp::config::LoongConfig::default();
        let mut inventory = mvp::channel::channel_inventory(&config);
        let surface = inventory
            .channel_surfaces
            .iter_mut()
            .find(|surface| surface.catalog.id == "weixin")
            .expect("weixin surface");
        let account = surface
            .configured_accounts
            .iter_mut()
            .find(|account| account.configured_account_id == "default")
            .expect("default weixin account");
        let serve = account
            .operations
            .iter_mut()
            .find(|operation| operation.id == "serve")
            .expect("weixin serve operation");
        serve.runtime = Some(mvp::channel::ChannelOperationRuntime {
            running: true,
            stale: false,
            busy: false,
            active_runs: 0,
            consecutive_failures: 2,
            last_run_activity_at: Some(1_700_000_000_000),
            last_heartbeat_at: Some(1_700_000_005_000),
            last_failure_at: Some(1_700_000_006_000),
            last_recovery_at: None,
            last_error: Some("temporary bridge timeout".to_owned()),
            last_duplicate_reclaim_at: None,
            pid: Some(5151),
            account_id: Some("default".to_owned()),
            account_label: Some("default".to_owned()),
            instance_count: 1,
            running_instances: 1,
            stale_instances: 0,
            duplicate_owner_pids: Vec::new(),
            last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
            recent_incidents: Vec::new(),
        });

        let read_model = build_channel_surface_read_model(surface.clone());
        let operator_surface = build_operator_channel_surface_read_model(
            &read_model,
            &inventory.channel_access_policies,
            &["weixin".to_owned()],
        );

        assert_eq!(operator_surface.runtime_attention_account_count, 1);
        assert_eq!(
            operator_surface.runtime_attention_reasons,
            vec!["retrying".to_owned()]
        );
        assert_eq!(
            operator_surface.runtime_attention_remediations,
            vec!["inspect_bridge_connectivity".to_owned()]
        );
        assert_eq!(operator_surface.retrying_runtime_account_count, 1);
        assert_eq!(operator_surface.stale_runtime_account_count, 0);
        assert_eq!(operator_surface.duplicate_runtime_account_count, 0);
        assert!(operator_surface.preferred_runtime_owner_pids.is_empty());
        assert!(
            operator_surface
                .duplicate_runtime_cleanup_owner_pids
                .is_empty()
        );
        assert!(
            operator_surface
                .last_duplicate_runtime_auto_reclaim_at
                .is_none()
        );
        assert!(
            operator_surface
                .last_duplicate_runtime_auto_cleanup_owner_pids
                .is_empty()
        );
        assert!(operator_surface.service_enabled);
        assert!(!operator_surface.service_ready);
    }

    #[test]
    fn operator_channel_surface_read_model_collects_duplicate_runtime_owner_pids() {
        let config = mvp::config::LoongConfig::default();
        let mut inventory = mvp::channel::channel_inventory(&config);
        let surface = inventory
            .channel_surfaces
            .iter_mut()
            .find(|surface| surface.catalog.id == "weixin")
            .expect("weixin surface");
        let account = surface
            .configured_accounts
            .iter_mut()
            .find(|account| account.configured_account_id == "default")
            .expect("default weixin account");
        let serve = account
            .operations
            .iter_mut()
            .find(|operation| operation.id == "serve")
            .expect("weixin serve operation");
        serve.runtime = Some(mvp::channel::ChannelOperationRuntime {
            running: true,
            stale: false,
            busy: false,
            active_runs: 0,
            consecutive_failures: 0,
            last_run_activity_at: Some(1_700_000_000_000),
            last_heartbeat_at: Some(1_700_000_005_000),
            last_failure_at: None,
            last_recovery_at: None,
            last_error: None,
            last_duplicate_reclaim_at: Some(1_700_000_007_000),
            pid: Some(6262),
            account_id: Some("default".to_owned()),
            account_label: Some("default".to_owned()),
            instance_count: 2,
            running_instances: 2,
            stale_instances: 0,
            duplicate_owner_pids: vec![5151, 6262],
            last_duplicate_reclaim_cleanup_owner_pids: vec![5151],
            recent_incidents: vec![mvp::channel::ChannelOperationRuntimeIncident {
                at_ms: 1_700_000_007_000,
                kind: mvp::channel::ChannelOperationRuntimeIncidentKind::DuplicateReclaim,
                detail: Some(
                    "requested cooperative shutdown for duplicate runtime owners".to_owned(),
                ),
                owner_pids: vec![5151],
            }],
        });

        let read_model = build_channel_surface_read_model(surface.clone());
        let operator_surface = build_operator_channel_surface_read_model(
            &read_model,
            &inventory.channel_access_policies,
            &["weixin".to_owned()],
        );

        assert_eq!(operator_surface.runtime_attention_account_count, 1);
        assert_eq!(
            operator_surface.runtime_attention_reasons,
            vec!["duplicate_runtime_instances".to_owned()]
        );
        assert_eq!(
            operator_surface.runtime_attention_remediations,
            vec!["stop_duplicate_runtime_instances".to_owned()]
        );
        assert_eq!(operator_surface.duplicate_runtime_account_count, 1);
        assert_eq!(operator_surface.preferred_runtime_owner_pids, vec![6262]);
        assert_eq!(
            operator_surface.duplicate_runtime_cleanup_owner_pids,
            vec![5151]
        );
        assert_eq!(
            operator_surface.last_duplicate_runtime_auto_reclaim_at,
            Some(1_700_000_007_000)
        );
        assert_eq!(
            operator_surface.last_duplicate_runtime_auto_cleanup_owner_pids,
            vec![5151]
        );
        assert_eq!(operator_surface.recent_runtime_incidents.len(), 1);
        assert_eq!(
            operator_surface.recent_runtime_incidents[0].kind,
            "duplicate_reclaim"
        );
        assert_eq!(
            operator_surface.recent_runtime_incidents[0].owner_pids,
            vec![5151]
        );
        assert!(!operator_surface.service_ready);
    }

    #[test]
    fn operator_channels_summary_read_model_collects_runtime_attention_surface_ids() {
        let config = mvp::config::LoongConfig::default();
        let mut inventory = mvp::channel::channel_inventory(&config);
        let surface = inventory
            .channel_surfaces
            .iter_mut()
            .find(|surface| surface.catalog.id == "weixin")
            .expect("weixin surface");
        let account = surface
            .configured_accounts
            .iter_mut()
            .find(|account| account.configured_account_id == "default")
            .expect("default weixin account");
        let serve = account
            .operations
            .iter_mut()
            .find(|operation| operation.id == "serve")
            .expect("weixin serve operation");
        serve.runtime = Some(mvp::channel::ChannelOperationRuntime {
            running: true,
            stale: false,
            busy: false,
            active_runs: 0,
            consecutive_failures: 2,
            last_run_activity_at: Some(1_700_000_000_000),
            last_heartbeat_at: Some(1_700_000_005_000),
            last_failure_at: Some(1_700_000_006_000),
            last_recovery_at: None,
            last_error: Some("temporary bridge timeout".to_owned()),
            last_duplicate_reclaim_at: None,
            pid: Some(5151),
            account_id: Some("default".to_owned()),
            account_label: Some("default".to_owned()),
            instance_count: 1,
            running_instances: 1,
            stale_instances: 0,
            duplicate_owner_pids: Vec::new(),
            last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
            recent_incidents: Vec::new(),
        });

        let channel_inventory = build_channel_inventory_read_model("/tmp/loong.toml", &inventory);
        let runtime_snapshot = GatewayRuntimeSnapshotReadModel {
            config: "/tmp/loong.toml".to_owned(),
            schema: GatewayRuntimeSnapshotSchema {
                version: 1,
                surface: "runtime_snapshot",
                purpose: "test",
            },
            provider: serde_json::json!({}),
            context_engine: serde_json::json!({}),
            memory_system: serde_json::json!({}),
            acp: serde_json::json!({}),
            channels: GatewayRuntimeSnapshotChannelsReadModel {
                enabled_channel_ids: vec!["weixin".to_owned()],
                enabled_runtime_backed_channel_ids: Vec::new(),
                enabled_service_channel_ids: vec!["weixin".to_owned()],
                enabled_plugin_backed_channel_ids: vec!["weixin".to_owned()],
                enabled_outbound_only_channel_ids: Vec::new(),
                inventory: channel_inventory.clone(),
            },
            tool_runtime: serde_json::json!({}),
            tools: GatewayRuntimeSnapshotToolsReadModel {
                visible_tool_count: 0,
                visible_tool_names: Vec::new(),
                visible_direct_tool_names: Vec::new(),
                hidden_tool_count: 0,
                hidden_tool_tags: Vec::new(),
                hidden_tool_surfaces: Vec::new(),
                capability_snapshot_sha256: "abc123".to_owned(),
                capability_snapshot: "{}".to_owned(),
                tool_calling: GatewayToolCallingReadModel {
                    availability: "ready".to_owned(),
                    structured_tool_schema_enabled: true,
                    effective_tool_schema_mode: "enabled".to_owned(),
                    active_model: "gpt-4.1-mini".to_owned(),
                    reason: "test".to_owned(),
                },
                web_access: GatewayWebAccessReadModel {
                    ordinary_network_access_enabled: false,
                    query_search_enabled: false,
                    query_search_default_provider: "duckduckgo".to_owned(),
                    query_search_credential_ready: false,
                    separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
                },
            },
            runtime_plugins: serde_json::json!({}),
            external_skills: serde_json::json!({}),
        };

        let summary =
            build_operator_channels_summary_read_model(&channel_inventory, &runtime_snapshot);

        assert_eq!(
            summary.runtime_attention_surface_ids,
            vec!["weixin".to_owned()]
        );
        assert_eq!(
            summary.retrying_runtime_surface_ids,
            vec!["weixin".to_owned()]
        );
        assert!(summary.stale_runtime_surface_ids.is_empty());
        assert!(summary.duplicate_runtime_surface_ids.is_empty());
    }

    #[test]
    fn channel_inventory_read_model_includes_structured_channel_access_policies() {
        let mut config = mvp::config::LoongConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned()));
        config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("secret".to_owned()));
        config.feishu.allowed_chat_ids = vec!["*".to_owned()];
        config.feishu.allowed_sender_ids = vec!["ou_admin".to_owned()];

        let inventory = mvp::channel::channel_inventory(&config);
        let read_model = build_channel_inventory_read_model("/tmp/loong.toml", &inventory);
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

    #[test]
    fn tool_surface_read_model_preserves_guidance_and_counts() {
        let surface = mvp::tools::ToolSurfaceState {
            surface_id: "read".to_owned(),
            prompt_snippet: "inspect files".to_owned(),
            usage_guidance: "prefer direct read before shell".to_owned(),
            tool_ids: vec!["file.read".to_owned(), "file.write".to_owned()],
        };

        let read_model = build_tool_surface_read_model(&surface);

        assert_eq!(read_model.surface_id, "read");
        assert_eq!(read_model.tool_count, 2);
        assert_eq!(read_model.visible_tool_names, vec!["read", "write"]);
        assert_eq!(read_model.tool_ids, vec!["file.read", "file.write"]);
        assert_eq!(read_model.usage_guidance, "prefer direct read before shell");
    }

    #[test]
    fn operator_runtime_summary_includes_hidden_surface_ids() {
        let runtime_snapshot = GatewayRuntimeSnapshotReadModel {
            config: "/tmp/loongclaw.toml".to_owned(),
            schema: GatewayRuntimeSnapshotSchema {
                version: 1,
                surface: "runtime_snapshot",
                purpose: "runtime_introspection",
            },
            provider: serde_json::json!({
                "active_profile_id": "demo",
                "active_label": "Demo"
            }),
            context_engine: serde_json::json!({}),
            memory_system: serde_json::json!({}),
            acp: serde_json::json!({}),
            channels: GatewayRuntimeSnapshotChannelsReadModel {
                enabled_channel_ids: vec![],
                enabled_runtime_backed_channel_ids: vec![],
                enabled_service_channel_ids: vec![],
                enabled_plugin_backed_channel_ids: vec![],
                enabled_outbound_only_channel_ids: vec![],
                inventory: GatewayChannelInventoryReadModel {
                    config: "/tmp/loongclaw.toml".to_owned(),
                    schema: GatewayChannelInventorySchema {
                        version: 1,
                        primary_channel_view: "channel_surfaces",
                        catalog_view: "channel_catalog",
                        legacy_channel_views: &[],
                    },
                    summary: GatewayChannelInventorySummaryReadModel {
                        total_surface_count: 0,
                        runtime_backed_surface_count: 0,
                        config_backed_surface_count: 0,
                        plugin_backed_surface_count: 0,
                        catalog_only_surface_count: 0,
                    },
                    channels: vec![],
                    catalog_only_channels: vec![],
                    channel_catalog: vec![],
                    channel_surfaces: vec![],
                    channel_access_policies: vec![],
                },
            },
            tool_runtime: serde_json::json!({}),
            tools: GatewayRuntimeSnapshotToolsReadModel {
                visible_tool_count: 3,
                visible_tool_names: vec!["tool.search".to_owned()],
                visible_direct_tool_names: vec!["read".to_owned(), "write".to_owned()],
                hidden_tool_count: 2,
                hidden_tool_tags: vec!["session".to_owned(), "web".to_owned()],
                hidden_tool_surfaces: vec![
                    GatewayToolSurfaceReadModel {
                        surface_id: "agent".to_owned(),
                        prompt_snippet: "inspect agent runtime state".to_owned(),
                        usage_guidance: "use for approvals, sessions, routing, or delegation"
                            .to_owned(),
                        tool_count: 2,
                        visible_tool_names: vec![
                            "session_events".to_owned(),
                            "session_status".to_owned(),
                        ],
                        tool_ids: vec!["session_events".to_owned(), "session_status".to_owned()],
                    },
                    GatewayToolSurfaceReadModel {
                        surface_id: "web".to_owned(),
                        prompt_snippet: "hidden http operations".to_owned(),
                        usage_guidance: "use direct web first".to_owned(),
                        tool_count: 1,
                        visible_tool_names: vec!["http.request".to_owned()],
                        tool_ids: vec!["http.request".to_owned()],
                    },
                ],
                capability_snapshot_sha256: "abc123".to_owned(),
                capability_snapshot: String::new(),
                tool_calling: GatewayToolCallingReadModel {
                    availability: "ready".to_owned(),
                    structured_tool_schema_enabled: true,
                    effective_tool_schema_mode: "enabled_with_downgrade".to_owned(),
                    active_model: "gpt-4.1-mini".to_owned(),
                    reason: "runtime ready".to_owned(),
                },
                web_access: GatewayWebAccessReadModel {
                    ordinary_network_access_enabled: true,
                    query_search_enabled: false,
                    query_search_default_provider: "duckduckgo".to_owned(),
                    query_search_credential_ready: true,
                    separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
                },
            },
            runtime_plugins: serde_json::json!({}),
            external_skills: serde_json::json!({}),
        };

        let summary = build_operator_runtime_summary_read_model(&runtime_snapshot);

        assert_eq!(summary.visible_direct_tool_names, vec!["read", "write"]);
        assert_eq!(summary.hidden_tool_surface_ids, vec!["agent", "web"]);
        assert!(summary.web_access.ordinary_network_access_enabled);
        assert!(!summary.web_access.query_search_enabled);
    }
}
