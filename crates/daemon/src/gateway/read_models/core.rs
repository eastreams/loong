use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION;
use crate::RuntimeSnapshotCliState;
use crate::app;
use crate::operator_inventory_cli::{
    CHANNELS_CLI_JSON_LEGACY_VIEWS, CHANNELS_CLI_JSON_SCHEMA_VERSION,
};
use crate::plugin_bridge_account_summary::plugin_bridge_account_summary;

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
    pub channels: Vec<app::channel::ChannelStatusSnapshot>,
    pub catalog_only_channels: Vec<app::channel::ChannelCatalogEntry>,
    pub channel_catalog: Vec<GatewayChannelCatalogEntryReadModel>,
    pub channel_surfaces: Vec<GatewayChannelSurfaceReadModel>,
    pub channel_access_policies: Vec<app::channel::ChannelConfiguredAccountAccessPolicy>,
}

pub type ChannelsCliJsonPayload = GatewayChannelInventoryReadModel;

#[derive(Debug, Clone, Serialize)]
pub struct GatewayChannelCatalogEntryReadModel {
    #[serde(flatten)]
    pub catalog: app::channel::ChannelCatalogEntry,
    pub runtime_kind: String,
    pub operational_model: String,
    pub service_contract_model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayChannelSurfaceReadModel {
    #[serde(flatten)]
    pub surface: app::channel::ChannelSurface,
    pub runtime_kind: String,
    pub operational_model: String,
    pub service_contract_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_bridge_account_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayChannelRuntimeKindCountsReadModel {
    pub runtime_backed: usize,
    pub plugin_backed: usize,
    pub outbound_only: usize,
    pub catalog_only: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayChannelOperationalModelCountsReadModel {
    pub gateway_supervised: usize,
    pub standalone_runtime: usize,
    pub plugin_backed: usize,
    pub outbound_only: usize,
    pub catalog_only: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayChannelServiceContractModelCountsReadModel {
    pub managed_bridge_capable_service: usize,
    pub native_service_channel: usize,
    pub standalone_native_service: usize,
    pub external_plugin_bridge: usize,
    pub direct_send_only: usize,
    pub catalog_only: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayChannelInventorySummaryReadModel {
    pub total_surface_count: usize,
    pub runtime_backed_surface_count: usize,
    pub config_backed_surface_count: usize,
    pub plugin_backed_surface_count: usize,
    pub catalog_only_surface_count: usize,
    pub runtime_kind_counts: GatewayChannelRuntimeKindCountsReadModel,
    pub operational_model_counts: GatewayChannelOperationalModelCountsReadModel,
    pub service_contract_model_counts: GatewayChannelServiceContractModelCountsReadModel,
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
pub struct GatewayToolAccessReadModel {
    pub ordinary_network_access_enabled: bool,
    pub query_search_enabled: bool,
    pub query_search_default_provider: String,
    pub query_search_source: String,
    pub query_search_provider_label: String,
    pub query_search_credential_ready: bool,
    pub browser_page_access_enabled: bool,
    pub managed_browser_session_enabled: bool,
    pub managed_browser_session_ready: bool,
    pub consent_mode: String,
    pub approval_mode: String,
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
    pub access: GatewayToolAccessReadModel,
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
    pub skills: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayToolCallingReadModel {
    pub availability: String,
    pub structured_tool_schema_enabled: bool,
    pub effective_tool_schema_mode: String,
    pub active_model: String,
    pub reason: String,
}

pub fn build_channel_inventory_read_model(
    config_path: &str,
    inventory: &app::channel::ChannelInventory,
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
    let channel_catalog = inventory
        .channel_catalog
        .iter()
        .cloned()
        .map(build_channel_catalog_entry_read_model)
        .collect();
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

fn build_channel_catalog_entry_read_model(
    catalog: app::channel::ChannelCatalogEntry,
) -> GatewayChannelCatalogEntryReadModel {
    let classification = app::channel::channel_classification(catalog.id);

    GatewayChannelCatalogEntryReadModel {
        catalog,
        runtime_kind: classification.runtime_kind.as_str().to_owned(),
        operational_model: classification.operational_model.as_str().to_owned(),
        service_contract_model: classification.service_contract_model.as_str().to_owned(),
    }
}

pub(crate) fn build_channel_surface_read_model(
    surface: app::channel::ChannelSurface,
) -> GatewayChannelSurfaceReadModel {
    let plugin_bridge_account_summary = plugin_bridge_account_summary(&surface);
    let classification = app::channel::channel_classification(surface.catalog.id);

    GatewayChannelSurfaceReadModel {
        surface,
        runtime_kind: classification.runtime_kind.as_str().to_owned(),
        operational_model: classification.operational_model.as_str().to_owned(),
        service_contract_model: classification.service_contract_model.as_str().to_owned(),
        plugin_bridge_account_summary,
    }
}

fn build_channel_inventory_summary_read_model(
    channel_surfaces: &[app::channel::ChannelSurface],
) -> GatewayChannelInventorySummaryReadModel {
    let runtime_kind_counts = summarize_channel_runtime_kind_counts(channel_surfaces);
    let operational_model_counts = summarize_channel_operational_model_counts(channel_surfaces);
    let service_contract_model_counts =
        summarize_channel_service_contract_model_counts(channel_surfaces);
    let total_surface_count = channel_surfaces.len();
    let runtime_backed_surface_count = runtime_kind_counts.runtime_backed;
    let config_backed_surface_count = runtime_kind_counts.outbound_only;
    let plugin_backed_surface_count = runtime_kind_counts.plugin_backed;
    let catalog_only_surface_count = runtime_kind_counts.catalog_only;

    GatewayChannelInventorySummaryReadModel {
        total_surface_count,
        runtime_backed_surface_count,
        config_backed_surface_count,
        plugin_backed_surface_count,
        catalog_only_surface_count,
        runtime_kind_counts,
        operational_model_counts,
        service_contract_model_counts,
    }
}

fn summarize_channel_runtime_kind_counts(
    channel_surfaces: &[app::channel::ChannelSurface],
) -> GatewayChannelRuntimeKindCountsReadModel {
    GatewayChannelRuntimeKindCountsReadModel {
        runtime_backed: channel_surfaces
            .iter()
            .filter(|surface| channel_runtime_kind_text(surface.catalog.id) == "runtime_backed")
            .count(),
        plugin_backed: channel_surfaces
            .iter()
            .filter(|surface| channel_runtime_kind_text(surface.catalog.id) == "plugin_backed")
            .count(),
        outbound_only: channel_surfaces
            .iter()
            .filter(|surface| channel_runtime_kind_text(surface.catalog.id) == "outbound_only")
            .count(),
        catalog_only: channel_surfaces
            .iter()
            .filter(|surface| channel_runtime_kind_text(surface.catalog.id) == "catalog_only")
            .count(),
    }
}

fn summarize_channel_operational_model_counts(
    channel_surfaces: &[app::channel::ChannelSurface],
) -> GatewayChannelOperationalModelCountsReadModel {
    GatewayChannelOperationalModelCountsReadModel {
        gateway_supervised: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_operational_model_text(surface.catalog.id) == "gateway_supervised"
            })
            .count(),
        standalone_runtime: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_operational_model_text(surface.catalog.id) == "standalone_runtime"
            })
            .count(),
        plugin_backed: channel_surfaces
            .iter()
            .filter(|surface| channel_operational_model_text(surface.catalog.id) == "plugin_backed")
            .count(),
        outbound_only: channel_surfaces
            .iter()
            .filter(|surface| channel_operational_model_text(surface.catalog.id) == "outbound_only")
            .count(),
        catalog_only: channel_surfaces
            .iter()
            .filter(|surface| channel_operational_model_text(surface.catalog.id) == "catalog_only")
            .count(),
    }
}

fn summarize_channel_service_contract_model_counts(
    channel_surfaces: &[app::channel::ChannelSurface],
) -> GatewayChannelServiceContractModelCountsReadModel {
    GatewayChannelServiceContractModelCountsReadModel {
        managed_bridge_capable_service: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_service_contract_model_text(surface.catalog.id)
                    == "managed_bridge_capable_service"
            })
            .count(),
        native_service_channel: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_service_contract_model_text(surface.catalog.id) == "native_service_channel"
            })
            .count(),
        standalone_native_service: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_service_contract_model_text(surface.catalog.id)
                    == "standalone_native_service"
            })
            .count(),
        external_plugin_bridge: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_service_contract_model_text(surface.catalog.id) == "external_plugin_bridge"
            })
            .count(),
        direct_send_only: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_service_contract_model_text(surface.catalog.id) == "direct_send_only"
            })
            .count(),
        catalog_only: channel_surfaces
            .iter()
            .filter(|surface| {
                channel_service_contract_model_text(surface.catalog.id) == "catalog_only"
            })
            .count(),
    }
}

fn channel_runtime_kind_text(channel_id: &str) -> &'static str {
    app::channel::channel_classification(channel_id)
        .runtime_kind
        .as_str()
}

fn channel_operational_model_text(channel_id: &str) -> &'static str {
    app::channel::channel_classification(channel_id)
        .operational_model
        .as_str()
}

fn channel_service_contract_model_text(channel_id: &str) -> &'static str {
    app::channel::channel_classification(channel_id)
        .service_contract_model
        .as_str()
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
    let channels = build_runtime_snapshot_channels_read_model(config.as_str(), snapshot);
    let tool_runtime =
        crate::runtime_snapshot_tool_runtime_json(&snapshot.tool_runtime, &snapshot.tool_access);
    let tools = build_runtime_snapshot_tools_read_model(snapshot);
    let runtime_plugins = crate::runtime_snapshot_runtime_plugins_json(&snapshot.runtime_plugins);
    let skills = crate::runtime_snapshot_skills_json(&snapshot.skills);

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
        skills,
    }
}

pub(crate) fn build_runtime_snapshot_channels_read_model(
    config_path: &str,
    snapshot: &RuntimeSnapshotCliState,
) -> GatewayRuntimeSnapshotChannelsReadModel {
    let inventory = build_channel_inventory_read_model(config_path, &snapshot.channels);
    GatewayRuntimeSnapshotChannelsReadModel {
        enabled_channel_ids: snapshot.enabled_channel_ids.clone(),
        enabled_runtime_backed_channel_ids: snapshot.enabled_runtime_backed_channel_ids.clone(),
        enabled_service_channel_ids: snapshot.enabled_service_channel_ids.clone(),
        enabled_plugin_backed_channel_ids: snapshot.enabled_plugin_backed_channel_ids.clone(),
        enabled_outbound_only_channel_ids: snapshot.enabled_outbound_only_channel_ids.clone(),
        inventory,
    }
}

pub(crate) fn build_runtime_snapshot_tools_read_model(
    snapshot: &RuntimeSnapshotCliState,
) -> GatewayRuntimeSnapshotToolsReadModel {
    GatewayRuntimeSnapshotToolsReadModel {
        visible_tool_count: snapshot.visible_tool_names.len(),
        visible_tool_names: snapshot.visible_tool_names.clone(),
        visible_direct_tool_names: snapshot
            .discoverable_tool_summary
            .visible_direct_tools
            .clone(),
        hidden_tool_count: snapshot.discoverable_tool_summary.hidden_tool_count,
        hidden_tool_tags: snapshot.discoverable_tool_summary.hidden_tags.clone(),
        hidden_tool_surfaces: snapshot
            .discoverable_tool_summary
            .hidden_surfaces
            .iter()
            .map(build_tool_surface_read_model)
            .collect(),
        capability_snapshot_sha256: snapshot.capability_snapshot_sha256.clone(),
        capability_snapshot: snapshot.capability_snapshot.clone(),
        tool_calling: build_tool_calling_read_model(&snapshot.tool_calling),
        access: build_tool_access_read_model(&snapshot.tool_access),
    }
}

pub(crate) fn build_tool_access_read_model(
    summary: &crate::RuntimeToolAccessSummary,
) -> GatewayToolAccessReadModel {
    GatewayToolAccessReadModel {
        ordinary_network_access_enabled: summary.ordinary_network_access_enabled,
        query_search_enabled: summary.query_search_enabled,
        query_search_default_provider: summary.query_search_default_provider.clone(),
        query_search_source: summary.query_search_source.to_owned(),
        query_search_provider_label: summary.query_search_provider_label.clone(),
        query_search_credential_ready: summary.query_search_credential_ready,
        browser_page_access_enabled: summary.browser_page_access_enabled,
        managed_browser_session_enabled: summary.managed_browser_session_enabled,
        managed_browser_session_ready: summary.managed_browser_session_ready,
        consent_mode: summary.consent_mode.to_owned(),
        approval_mode: summary.approval_mode.to_owned(),
        separation_note: summary.separation_note.to_owned(),
    }
}

pub(crate) fn build_tool_surface_read_model(
    surface: &app::tools::ToolSurfaceState,
) -> GatewayToolSurfaceReadModel {
    let visible_tool_names = visible_tool_names_for_surface(surface);
    GatewayToolSurfaceReadModel {
        surface_id: surface.surface_id.clone(),
        prompt_snippet: surface.prompt_snippet.clone(),
        usage_guidance: surface.usage_guidance.clone(),
        tool_count: surface.tool_count(),
        visible_tool_names: visible_tool_names.clone(),
        tool_ids: visible_tool_names,
    }
}

fn visible_tool_names_for_surface(surface: &app::tools::ToolSurfaceState) -> Vec<String> {
    let mut visible_tool_names = Vec::new();

    for tool_id in &surface.tool_ids {
        let visible_tool_name = app::tools::legacy_display_tool_name(tool_id.as_str());
        if !visible_tool_names.contains(&visible_tool_name) {
            visible_tool_names.push(visible_tool_name);
        }
    }

    visible_tool_names
}

pub(crate) fn build_tool_calling_read_model(
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

pub(crate) fn channel_account_is_misconfigured(
    account: &app::channel::ChannelStatusSnapshot,
) -> bool {
    account
        .operations
        .iter()
        .any(|operation| operation.health == app::channel::ChannelOperationHealth::Misconfigured)
}

pub(crate) fn gateway_owner_base_url(
    owner_status: &crate::gateway::state::GatewayOwnerStatus,
) -> Option<String> {
    let bind_address = owner_status.bind_address.as_deref()?;
    let port = owner_status.port?;
    let ip_address = bind_address.parse::<IpAddr>().ok()?;
    let socket_address = SocketAddr::new(ip_address, port);
    let base_url = format!("http://{socket_address}");
    Some(base_url)
}

pub(crate) fn gateway_owner_control_is_loopback(
    owner_status: &crate::gateway::state::GatewayOwnerStatus,
) -> bool {
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
