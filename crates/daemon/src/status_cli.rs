use loong_contracts::WorkRuntimeHealthSnapshot;
use loong_spec::CliResult;
use serde::Serialize;
use std::path::Path;

use crate::gateway::client::GatewayLocalClient;
use crate::gateway::read_models::{
    GatewayAcpObservabilityReadModel, GatewayOperatorChannelsSummaryReadModel,
    GatewayOperatorSummaryReadModel, build_acp_observability_read_model,
    build_node_inventory_read_model, build_operator_nodes_summary_read_model,
    build_operator_summary_read_model, build_runtime_snapshot_read_model,
};
use crate::gateway::service::default_gateway_owner_status;
use crate::gateway::state::{default_gateway_runtime_state_dir, load_gateway_owner_status};
use crate::mvp;
use crate::runtime_snapshot_compaction_presentation::build_compaction_hygiene_status_values;
use crate::supervisor::LoadedSupervisorConfig;

const STATUS_CLI_JSON_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize)]
pub struct StatusCliJsonSchema {
    pub version: u32,
    pub surface: &'static str,
    pub purpose: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusCliAcpReadModel {
    pub enabled: bool,
    pub availability: String,
    pub error: Option<String>,
    pub persisted_session_count: Option<usize>,
    pub observability: Option<GatewayAcpObservabilityReadModel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusCliWorkUnitReadModel {
    pub availability: String,
    pub error: Option<String>,
    pub health: Option<WorkRuntimeHealthSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusCliAction {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusCliReadModel {
    pub config: String,
    pub schema: StatusCliJsonSchema,
    pub active_provider: String,
    pub active_model: String,
    pub memory_profile: String,
    pub gateway: GatewayOperatorSummaryReadModel,
    pub acp: StatusCliAcpReadModel,
    pub work_units: StatusCliWorkUnitReadModel,
    pub next_actions: Vec<StatusCliAction>,
    pub recipes: Vec<String>,
}

pub async fn run_status_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let status = collect_status_cli_read_model(config_path).await?;

    if as_json {
        let pretty_result = serde_json::to_string_pretty(&status);
        let pretty =
            pretty_result.map_err(|error| format!("serialize status output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = render_status_cli_text(&status);
    println!("{rendered}");
    Ok(())
}

pub async fn collect_status_cli_read_model(
    config_path: Option<&str>,
) -> CliResult<StatusCliReadModel> {
    let load_result = mvp::config::load(config_path);
    let (resolved_path, config) = load_result?;
    let resolved_path_ref = resolved_path.as_path();
    mvp::runtime_env::initialize_runtime_environment(&config, Some(resolved_path_ref));

    let loaded_config = LoadedSupervisorConfig {
        resolved_path: resolved_path.clone(),
        config: config.clone(),
    };
    let snapshot_result =
        crate::collect_runtime_snapshot_cli_state_from_loaded_config(&loaded_config);
    let snapshot = snapshot_result?;
    let config_path_display = resolved_path.display().to_string();
    let config_path_text = config_path_display.as_str();
    let channel_inventory =
        crate::build_channels_cli_json_payload(config_path_text, &snapshot.channels);
    let runtime_snapshot = build_runtime_snapshot_read_model(&snapshot);
    let runtime_dir = default_gateway_runtime_state_dir();
    let gateway = build_status_cli_local_gateway_summary(
        runtime_dir.as_path(),
        config_path_text,
        &channel_inventory,
        &runtime_snapshot,
    );
    let gateway =
        collect_status_cli_gateway_summary(config_path_text, runtime_dir.as_path(), gateway).await;
    let acp = collect_status_cli_acp_read_model(config_path_text, &config).await;
    let work_units = collect_status_cli_work_unit_read_model(&config);
    let mut next_actions = collect_status_runtime_attention_actions(config_path_text, &gateway);
    next_actions.extend(
        crate::next_actions::collect_setup_next_actions(&config, config_path_text)
            .into_iter()
            .map(|action| StatusCliAction {
                label: action.label,
                command: action.command,
            }),
    );
    let recipes = build_status_cli_recipes(config_path_text);
    let schema = StatusCliJsonSchema {
        version: STATUS_CLI_JSON_SCHEMA_VERSION,
        surface: "status",
        purpose: "operator_runtime_summary",
    };

    Ok(StatusCliReadModel {
        config: config_path_display,
        schema,
        active_provider: crate::provider_presentation::active_provider_detail_label(&config),
        active_model: config.provider.model.clone(),
        memory_profile: config.memory.profile.as_str().to_owned(),
        gateway,
        acp,
        work_units,
        next_actions,
        recipes,
    })
}

fn build_status_cli_local_gateway_summary(
    runtime_dir: &Path,
    config_path: &str,
    channel_inventory: &crate::gateway::read_models::GatewayChannelInventoryReadModel,
    runtime_snapshot: &crate::gateway::read_models::GatewayRuntimeSnapshotReadModel,
) -> GatewayOperatorSummaryReadModel {
    let owner_status_option = load_gateway_owner_status(runtime_dir);
    let owner_status =
        select_gateway_owner_status_for_config(runtime_dir, config_path, owner_status_option);
    let node_inventory = build_node_inventory_read_model(config_path, channel_inventory, &[]);
    let node_summary = build_operator_nodes_summary_read_model(&node_inventory);

    build_operator_summary_read_model(
        &owner_status,
        channel_inventory,
        runtime_snapshot,
        crate::gateway::read_models::GatewayOperatorPairingSummaryReadModel {
            pending_request_count: 0,
            approved_device_count: 0,
            last_activity_ms: None,
        },
        node_summary,
    )
}

async fn collect_status_cli_gateway_summary(
    config_path: &str,
    runtime_dir: &Path,
    local_gateway: GatewayOperatorSummaryReadModel,
) -> GatewayOperatorSummaryReadModel {
    let client = match GatewayLocalClient::discover(runtime_dir) {
        Ok(client) => client,
        Err(_) => return local_gateway,
    };

    if !gateway_owner_status_matches_config(client.discovery().owner_status(), config_path) {
        return local_gateway;
    }

    let live_gateway = match client.operator_summary().await {
        Ok(gateway) => gateway,
        Err(_) => return local_gateway,
    };

    if !gateway_owner_status_matches_config(&live_gateway.owner, config_path) {
        return local_gateway;
    }

    live_gateway
}

fn gateway_owner_status_matches_config(
    owner_status: &crate::gateway::state::GatewayOwnerStatus,
    config_path: &str,
) -> bool {
    let owner_config_path = Path::new(owner_status.config_path.as_str());
    let requested_config_path = Path::new(config_path);
    owner_config_path == requested_config_path
}

fn select_gateway_owner_status_for_config(
    runtime_dir: &Path,
    config_path: &str,
    owner_status: Option<crate::gateway::state::GatewayOwnerStatus>,
) -> crate::gateway::state::GatewayOwnerStatus {
    let Some(owner_status) = owner_status else {
        return default_gateway_owner_status(runtime_dir);
    };

    let owner_config_path = Path::new(owner_status.config_path.as_str());
    let requested_config_path = Path::new(config_path);
    let matches_requested_config = owner_config_path == requested_config_path;

    if matches_requested_config {
        return owner_status;
    }

    default_gateway_owner_status(runtime_dir)
}

async fn collect_status_cli_acp_read_model(
    config_path: &str,
    config: &mvp::config::LoongConfig,
) -> StatusCliAcpReadModel {
    let enabled = config.acp.enabled;
    let persisted_session_count = load_persisted_acp_session_count(config);

    if !enabled {
        return StatusCliAcpReadModel {
            enabled,
            availability: "disabled".to_owned(),
            error: None,
            persisted_session_count,
            observability: None,
        };
    }

    let manager_result = mvp::acp::shared_acp_session_manager(config);
    let manager = match manager_result {
        Ok(manager) => manager,
        Err(error) => {
            return build_unavailable_acp_read_model(enabled, error, persisted_session_count);
        }
    };

    let snapshot_result = manager.observability_snapshot(config).await;
    let snapshot = match snapshot_result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return build_unavailable_acp_read_model(enabled, error, persisted_session_count);
        }
    };

    let observability = build_acp_observability_read_model(config_path, &snapshot);

    StatusCliAcpReadModel {
        enabled,
        availability: "available".to_owned(),
        error: None,
        persisted_session_count,
        observability: Some(observability),
    }
}

fn build_unavailable_acp_read_model(
    enabled: bool,
    error: String,
    persisted_session_count: Option<usize>,
) -> StatusCliAcpReadModel {
    StatusCliAcpReadModel {
        enabled,
        availability: "unavailable".to_owned(),
        error: Some(error),
        persisted_session_count,
        observability: None,
    }
}

fn collect_status_cli_work_unit_read_model(
    config: &mvp::config::LoongConfig,
) -> StatusCliWorkUnitReadModel {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = config;
        StatusCliWorkUnitReadModel {
            availability: "unavailable".to_owned(),
            error: Some("work unit runtime requires feature `memory-sqlite`".to_owned()),
            health: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let session_store_config = mvp::session::store::SessionStoreConfig::from(&memory_config);
        let repository_result =
            mvp::work::repository::WorkUnitRepository::new(&session_store_config);
        let repository = match repository_result {
            Ok(repository) => repository,
            Err(error) => {
                return StatusCliWorkUnitReadModel {
                    availability: "unavailable".to_owned(),
                    error: Some(error),
                    health: None,
                };
            }
        };

        let health_result = repository.load_runtime_health(None);
        let health = match health_result {
            Ok(health) => health,
            Err(error) => {
                return StatusCliWorkUnitReadModel {
                    availability: "unavailable".to_owned(),
                    error: Some(error),
                    health: None,
                };
            }
        };

        StatusCliWorkUnitReadModel {
            availability: "available".to_owned(),
            error: None,
            health: Some(health),
        }
    }
}

fn load_persisted_acp_session_count(config: &mvp::config::LoongConfig) -> Option<usize> {
    #[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
    {
        let _ = config;
        None
    }

    #[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
    {
        let sqlite_path = config.memory.resolved_sqlite_path();
        let store = mvp::acp::AcpSqliteSessionStore::new(Some(sqlite_path));
        let sessions_result = mvp::acp::AcpSessionStore::list(&store);
        let sessions = match sessions_result {
            Ok(sessions) => sessions,
            Err(_) => {
                return None;
            }
        };
        Some(sessions.len())
    }
}

fn build_status_cli_recipes(config_path: &str) -> Vec<String> {
    let command_name = crate::active_cli_command_name();
    let config_arg = crate::cli_handoff::shell_quote_argument(config_path);
    let gateway_recipe = format!("{command_name} gateway status");
    let channels_recipe = format!("{command_name} channels --config {config_arg} --json");
    let acp_observability_recipe =
        format!("{command_name} runtime acp observability --config {config_arg} --json");
    let acp_sessions_recipe =
        format!("{command_name} runtime acp sessions --config {config_arg} --json");
    let work_units_recipe =
        format!("{command_name} runtime work-unit health --config {config_arg} --json");

    vec![
        gateway_recipe,
        channels_recipe,
        acp_observability_recipe,
        acp_sessions_recipe,
        work_units_recipe,
    ]
}

fn render_status_cli_text(status: &StatusCliReadModel) -> String {
    let gateway = &status.gateway;
    let owner = &gateway.owner;
    let control_surface = &gateway.control_surface;
    let channels = &gateway.channels;
    let runtime = &gateway.runtime;
    let base_url_option = control_surface.base_url.as_deref();
    let base_url = base_url_option.unwrap_or("-");
    let owner_pid = render_optional_u32(owner.pid);
    let owner_session_option = owner.attached_cli_session.as_deref();
    let owner_session = owner_session_option.unwrap_or("-");
    let owner_error_option = owner.last_error.as_deref();
    let owner_error = owner_error_option.unwrap_or("-");
    let owner_shutdown_reason_option = owner.shutdown_reason.as_deref();
    let owner_shutdown_reason = owner_shutdown_reason_option.unwrap_or("-");
    let active_provider_profile_id_option = runtime.active_provider_profile_id.as_deref();
    let active_provider_profile_id = active_provider_profile_id_option.unwrap_or("-");
    let active_provider_label_option = runtime.active_provider_label.as_deref();
    let active_provider_label = active_provider_label_option.unwrap_or("-");
    let capability_snapshot_sha256 = runtime.capability_snapshot_sha256.as_str();
    let compaction_hygiene = &runtime.compaction_hygiene;
    let compaction_presentation = build_compaction_hygiene_status_values(compaction_hygiene);
    let visible_direct_tools = if runtime.visible_direct_tool_names.is_empty() {
        "-".to_owned()
    } else {
        runtime.visible_direct_tool_names.join(",")
    };
    let hidden_tool_surfaces = if runtime.hidden_tool_surface_ids.is_empty() {
        "-".to_owned()
    } else {
        runtime.hidden_tool_surface_ids.join(",")
    };
    let tool_calling = &runtime.tool_calling;
    let web_access = &runtime.web_access;
    let ordinary_network_detail = render_web_ordinary_network_detail(web_access);
    let query_search_detail = render_web_query_search_detail(web_access);
    let web_boundary_note = web_access.separation_note.clone();
    let mut sections = Vec::new();

    if let Some(primary_action) = status.next_actions.first() {
        sections.push(loong_app::tui_surface::TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: false,
            items: vec![loong_app::tui_surface::TuiActionSpec {
                label: primary_action.label.clone(),
                command: primary_action.command.clone(),
            }],
        });
    }
    if status.next_actions.len() > 1 {
        sections.push(loong_app::tui_surface::TuiSectionSpec::ActionGroup {
            title: Some("also useful".to_owned()),
            inline_title_when_wide: false,
            items: status
                .next_actions
                .iter()
                .skip(1)
                .map(|action| loong_app::tui_surface::TuiActionSpec {
                    label: action.label.clone(),
                    command: action.command.clone(),
                })
                .collect(),
        });
    }

    sections.push(loong_app::tui_surface::TuiSectionSpec::Checklist {
        title: Some("runtime posture".to_owned()),
        items: vec![
            loong_app::tui_surface::TuiChecklistItemSpec {
                status: if tool_calling.availability == "ready" {
                    loong_app::tui_surface::TuiChecklistStatus::Pass
                } else {
                    loong_app::tui_surface::TuiChecklistStatus::Warn
                },
                label: "tool calling".to_owned(),
                detail: format!(
                    "{} · structured schema={} · mode={}",
                    tool_calling.availability,
                    tool_calling.structured_tool_schema_enabled,
                    tool_calling.effective_tool_schema_mode
                ),
            },
            loong_app::tui_surface::TuiChecklistItemSpec {
                status: if status.acp.availability == "available"
                    || status.acp.availability == "disabled"
                {
                    loong_app::tui_surface::TuiChecklistStatus::Pass
                } else {
                    loong_app::tui_surface::TuiChecklistStatus::Warn
                },
                label: "ACP".to_owned(),
                detail: format!(
                    "enabled={} · availability={}",
                    status.acp.enabled, status.acp.availability
                ),
            },
            loong_app::tui_surface::TuiChecklistItemSpec {
                status: if status.work_units.availability == "available" {
                    loong_app::tui_surface::TuiChecklistStatus::Pass
                } else {
                    loong_app::tui_surface::TuiChecklistStatus::Warn
                },
                label: "work units".to_owned(),
                detail: format!("availability={}", status.work_units.availability),
            },
            loong_app::tui_surface::TuiChecklistItemSpec {
                status: if web_access.ordinary_network_access_enabled {
                    loong_app::tui_surface::TuiChecklistStatus::Pass
                } else {
                    loong_app::tui_surface::TuiChecklistStatus::Warn
                },
                label: "ordinary network".to_owned(),
                detail: ordinary_network_detail.clone(),
            },
            loong_app::tui_surface::TuiChecklistItemSpec {
                status: query_search_checklist_status(web_access),
                label: "query search".to_owned(),
                detail: query_search_detail.clone(),
            },
        ],
    });

    sections.push(loong_app::tui_surface::TuiSectionSpec::KeyValues {
        title: Some("saved runtime".to_owned()),
        items: vec![
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "config".to_owned(),
                value: status.config.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "provider".to_owned(),
                value: status.active_provider.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "model".to_owned(),
                value: status.active_model.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "memory profile".to_owned(),
                value: status.memory_profile.clone(),
            },
        ],
    });
    sections.push(loong_app::tui_surface::TuiSectionSpec::KeyValues {
        title: Some("gateway summary".to_owned()),
        items: vec![
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "phase".to_owned(),
                value: owner.phase.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "mode".to_owned(),
                value: owner.mode.as_str().to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "pid".to_owned(),
                value: owner_pid,
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "attached session".to_owned(),
                value: owner_session.to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "control base url".to_owned(),
                value: base_url.to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "paired devices".to_owned(),
                value: gateway.nodes.paired_device_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "managed bridges".to_owned(),
                value: gateway.nodes.managed_bridge_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "known nodes".to_owned(),
                value: gateway.nodes.total_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "visible tools".to_owned(),
                value: runtime.visible_tool_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "direct tools".to_owned(),
                value: visible_direct_tools,
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "hidden surfaces".to_owned(),
                value: hidden_tool_surfaces,
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "ordinary network".to_owned(),
                value: ordinary_network_detail,
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "query search".to_owned(),
                value: query_search_detail,
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "web boundary".to_owned(),
                value: web_boundary_note,
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "compaction hygiene".to_owned(),
                value: compaction_presentation.hygiene.clone(),
            },
        ],
    });
    sections.push(loong_app::tui_surface::TuiSectionSpec::KeyValues {
        title: Some("channel and recovery detail".to_owned()),
        items: vec![
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "owner config".to_owned(),
                value: owner.config_path.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "loopback only".to_owned(),
                value: control_surface.loopback_only.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "configured surfaces".to_owned(),
                value: owner.configured_surface_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "running surfaces".to_owned(),
                value: owner.running_surface_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "channel catalog".to_owned(),
                value: channels.catalog_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "configured accounts".to_owned(),
                value: channels.configured_account_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "configured channels".to_owned(),
                value: channels.configured_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "enabled accounts".to_owned(),
                value: channels.enabled_account_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "misconfigured accounts".to_owned(),
                value: channels.misconfigured_account_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "runtime-backed channels".to_owned(),
                value: channels.runtime_backed_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "config-backed channels".to_owned(),
                value: channels.config_backed_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "plugin-backed channels".to_owned(),
                value: channels.plugin_backed_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "catalog-only channels".to_owned(),
                value: channels.catalog_only_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "enabled runtime-backed".to_owned(),
                value: channels.enabled_runtime_backed_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "enabled service channels".to_owned(),
                value: channels.enabled_service_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "enabled plugin-backed".to_owned(),
                value: channels.enabled_plugin_backed_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "enabled outbound-only".to_owned(),
                value: channels.enabled_outbound_only_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "ready service channels".to_owned(),
                value: channels.ready_service_channel_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "runtime attention surfaces".to_owned(),
                value: channels.runtime_attention_surface_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "retrying runtime surfaces".to_owned(),
                value: channels.retrying_runtime_surface_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "stale runtime surfaces".to_owned(),
                value: channels.stale_runtime_surface_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "duplicate runtime surfaces".to_owned(),
                value: channels.duplicate_runtime_surface_count.to_string(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "runtime attention ids".to_owned(),
                value: render_status_channel_ids(&channels.runtime_attention_surface_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "retrying runtime ids".to_owned(),
                value: render_status_channel_ids(&channels.retrying_runtime_surface_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "stale runtime ids".to_owned(),
                value: render_status_channel_ids(&channels.stale_runtime_surface_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "duplicate runtime ids".to_owned(),
                value: render_status_channel_ids(&channels.duplicate_runtime_surface_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "enabled channels".to_owned(),
                value: render_status_channel_ids(&runtime.enabled_channel_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "runtime-backed enabled ids".to_owned(),
                value: render_status_channel_ids(&runtime.enabled_runtime_backed_channel_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "service enabled ids".to_owned(),
                value: render_status_channel_ids(&runtime.enabled_service_channel_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "plugin-backed enabled ids".to_owned(),
                value: render_status_channel_ids(&runtime.enabled_plugin_backed_channel_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "outbound-only enabled ids".to_owned(),
                value: render_status_channel_ids(&runtime.enabled_outbound_only_channel_ids),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "shutdown reason".to_owned(),
                value: owner_shutdown_reason.to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "last error".to_owned(),
                value: owner_error.to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "provider profile".to_owned(),
                value: active_provider_profile_id.to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "provider label".to_owned(),
                value: active_provider_label.to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "capability snapshot".to_owned(),
                value: capability_snapshot_sha256.to_owned(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "compaction samples".to_owned(),
                value: compaction_presentation.samples.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "compaction prunes".to_owned(),
                value: compaction_presentation.prunes.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "compaction pressure".to_owned(),
                value: compaction_presentation.pressure.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "compaction trend".to_owned(),
                value: compaction_presentation.trend.clone(),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "compaction repairability".to_owned(),
                value: compaction_presentation.repairability,
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "ACP".to_owned(),
                value: render_status_cli_acp_text(&status.acp),
            },
            loong_app::tui_surface::TuiKeyValueSpec::Plain {
                key: "work units".to_owned(),
                value: render_status_cli_work_units_text(&status.work_units),
            },
        ],
    });
    let runtime_attention_items = collect_status_runtime_attention_items(channels);
    if !runtime_attention_items.is_empty() {
        sections.push(loong_app::tui_surface::TuiSectionSpec::Checklist {
            title: Some("channel runtime attention".to_owned()),
            items: runtime_attention_items,
        });
    }

    if !status.recipes.is_empty() {
        sections.push(loong_app::tui_surface::TuiSectionSpec::ActionGroup {
            title: Some("deep dives".to_owned()),
            inline_title_when_wide: false,
            items: status
                .recipes
                .iter()
                .map(|recipe| loong_app::tui_surface::TuiActionSpec {
                    label: "recipe".to_owned(),
                    command: recipe.clone(),
                })
                .collect(),
        });
    }

    let screen = loong_app::tui_surface::TuiScreenSpec {
        header_style: loong_app::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some("operator runtime summary".to_owned()),
        title: Some("status".to_owned()),
        progress_line: None,
        intro_lines: vec![
            "Use this summary to decide the next operator action before drilling into raw runtime detail.".to_owned(),
        ],
        sections,
        choices: Vec::new(),
        footer_lines: vec![
            "Use loong status --json for machine-readable automation.".to_owned(),
        ],
    };

    loong_app::tui_surface::render_tui_screen_spec_ratatui(
        &screen,
        loong_app::presentation::detect_render_width(),
        false,
    )
    .join("\n")
}

fn collect_status_runtime_attention_actions(
    config_path: &str,
    gateway: &GatewayOperatorSummaryReadModel,
) -> Vec<StatusCliAction> {
    let attention_surfaces = gateway
        .channels
        .surfaces
        .iter()
        .filter(|surface| surface.implementation_status == "plugin_backed")
        .filter(|surface| surface.runtime_attention_account_count > 0)
        .collect::<Vec<_>>();

    if attention_surfaces.is_empty() {
        return Vec::new();
    }

    let command = crate::cli_handoff::format_subcommand_with_config("doctor", config_path);
    let label = if attention_surfaces.len() == 1 {
        let surface = attention_surfaces
            .first()
            .expect("one runtime attention surface should exist");
        status_runtime_attention_action_label(surface)
    } else {
        format!(
            "inspect managed bridge runtimes: {}",
            attention_surfaces
                .iter()
                .map(|surface| format!(
                    "{}({})",
                    surface.channel_id,
                    if surface.runtime_attention_reasons.is_empty() {
                        "attention".to_owned()
                    } else {
                        surface.runtime_attention_reasons.join(",")
                    }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    vec![StatusCliAction { label, command }]
}

fn status_runtime_attention_action_label(
    surface: &crate::gateway::read_models::GatewayOperatorChannelSurfaceReadModel,
) -> String {
    match surface.runtime_attention_reasons.as_slice() {
        [reason] if reason == "retrying" => {
            format!(
                "inspect {} managed bridge runtime (retrying)",
                surface.channel_id
            )
        }
        [reason] if reason == "stale" => {
            format!(
                "recover stale {} managed bridge runtime",
                surface.channel_id
            )
        }
        [reason] if reason == "duplicate_runtime_instances" => format!(
            "clean up duplicate {} managed bridge runtimes{}",
            surface.channel_id,
            render_status_runtime_keep_pid_suffix(surface)
        ),
        _ => {
            let reasons = if surface.runtime_attention_reasons.is_empty() {
                String::new()
            } else {
                format!(" ({})", surface.runtime_attention_reasons.join(","))
            };
            format!(
                "inspect {} managed bridge runtime{}",
                surface.channel_id, reasons
            )
        }
    }
}

fn collect_status_runtime_attention_items(
    channels: &GatewayOperatorChannelsSummaryReadModel,
) -> Vec<loong_app::tui_surface::TuiChecklistItemSpec> {
    channels
        .surfaces
        .iter()
        .filter(|surface| surface.runtime_attention_account_count > 0)
        .map(|surface| loong_app::tui_surface::TuiChecklistItemSpec {
            status: loong_app::tui_surface::TuiChecklistStatus::Warn,
            label: surface.label.clone(),
            detail: format!(
                "channel_id={} reasons={} remediations={} retrying={} stale={} duplicate_instances={} affected_accounts={} keep_pids={} cleanup_pids={} last_auto_reclaim_at={} auto_cleanup_pids={} incidents={}",
                surface.channel_id,
                if surface.runtime_attention_reasons.is_empty() {
                    "-".to_owned()
                } else {
                    surface.runtime_attention_reasons.join(",")
                },
                if surface.runtime_attention_remediations.is_empty() {
                    "-".to_owned()
                } else {
                    surface.runtime_attention_remediations.join(",")
                },
                surface.retrying_runtime_account_count,
                surface.stale_runtime_account_count,
                surface.duplicate_runtime_account_count,
                surface.runtime_attention_account_count,
                render_status_runtime_owner_pids(&surface.preferred_runtime_owner_pids),
                render_status_runtime_owner_pids(&surface.duplicate_runtime_cleanup_owner_pids),
                render_status_optional_timestamp(surface.last_duplicate_runtime_auto_reclaim_at),
                render_status_runtime_owner_pids(
                    &surface.last_duplicate_runtime_auto_cleanup_owner_pids,
                ),
                render_status_runtime_incident_summary(&surface.recent_runtime_incidents),
            ),
        })
        .collect()
}

fn render_status_runtime_keep_pid_suffix(
    surface: &crate::gateway::read_models::GatewayOperatorChannelSurfaceReadModel,
) -> String {
    if surface.preferred_runtime_owner_pids.len() == 1 {
        let pid = surface
            .preferred_runtime_owner_pids
            .first()
            .copied()
            .unwrap_or_default();
        return format!(" (keep pid {pid})");
    }

    String::new()
}

fn render_status_runtime_owner_pids(owner_pids: &[u32]) -> String {
    if owner_pids.is_empty() {
        return "-".to_owned();
    }

    owner_pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn render_status_optional_timestamp(timestamp_ms: Option<u64>) -> String {
    timestamp_ms
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn render_status_runtime_incident_summary(
    incidents: &[crate::gateway::read_models::GatewayOperatorRuntimeIncidentReadModel],
) -> String {
    let Some(incident) = incidents.first() else {
        return "-".to_owned();
    };

    format!("{}@{}", incident.kind, incident.at_ms)
}

fn render_status_channel_ids(channel_ids: &[String]) -> String {
    if channel_ids.is_empty() {
        return "-".to_owned();
    }

    channel_ids.join(",")
}

fn render_status_cli_acp_text(acp: &StatusCliAcpReadModel) -> String {
    let persisted_session_count = render_optional_usize(acp.persisted_session_count);
    let availability = acp.availability.as_str();

    if let Some(observability) = &acp.observability {
        let snapshot = &observability.snapshot;
        let error_values = snapshot.errors_by_code.values();
        let error_values = error_values.copied();
        let error_total = error_values.sum::<usize>();
        let line = format!(
            "acp enabled={} availability={} persisted_sessions={} runtime_active_sessions={} bound_sessions={} unbound_sessions={} actor_queue_depth={} turn_queue_depth={} turn_failures={} error_total={}",
            acp.enabled,
            availability,
            persisted_session_count,
            snapshot.runtime_cache.active_sessions,
            snapshot.sessions.bound,
            snapshot.sessions.unbound,
            snapshot.actors.queue_depth,
            snapshot.turns.queue_depth,
            snapshot.turns.failed,
            error_total,
        );
        return line;
    }

    let error_option = acp.error.as_deref();
    let error = error_option.unwrap_or("-");
    format!(
        "acp enabled={} availability={} persisted_sessions={} error={}",
        acp.enabled, availability, persisted_session_count, error,
    )
}

fn render_status_cli_work_units_text(work_units: &StatusCliWorkUnitReadModel) -> String {
    let availability = work_units.availability.as_str();

    if let Some(health) = &work_units.health {
        let line = format!(
            "work_units availability={} total_count={} ready_count={} leased_count={} running_count={} blocked_count={} retry_pending_count={} terminal_count={} archived_count={} expired_lease_count={}",
            availability,
            health.total_count,
            health.ready_count,
            health.leased_count,
            health.running_count,
            health.blocked_count,
            health.retry_pending_count,
            health.terminal_count,
            health.archived_count,
            health.expired_lease_count,
        );
        return line;
    }

    let error_option = work_units.error.as_deref();
    let error = error_option.unwrap_or("-");
    format!("work_units availability={} error={}", availability, error)
}

fn render_optional_u32(value: Option<u32>) -> String {
    let value = value.map(|value| value.to_string());
    value.unwrap_or_else(|| "-".to_owned())
}

fn render_optional_usize(value: Option<usize>) -> String {
    let value = value.map(|value| value.to_string());
    value.unwrap_or_else(|| "-".to_owned())
}

fn render_web_ordinary_network_detail(
    web_access: &crate::gateway::read_models::GatewayWebAccessReadModel,
) -> String {
    format!("enabled={}", web_access.ordinary_network_access_enabled)
}

fn render_web_query_search_detail(
    web_access: &crate::gateway::read_models::GatewayWebAccessReadModel,
) -> String {
    format!(
        "enabled={} · provider={} · credential_ready={}",
        web_access.query_search_enabled,
        web_access.query_search_default_provider,
        web_access.query_search_credential_ready,
    )
}

fn query_search_checklist_status(
    web_access: &crate::gateway::read_models::GatewayWebAccessReadModel,
) -> loong_app::tui_surface::TuiChecklistStatus {
    if !web_access.query_search_enabled || web_access.query_search_credential_ready {
        loong_app::tui_surface::TuiChecklistStatus::Pass
    } else {
        loong_app::tui_surface::TuiChecklistStatus::Warn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::read_models::{
        GatewayOperatorChannelsSummaryReadModel, GatewayOperatorControlSurfaceReadModel,
        GatewayOperatorRuntimeSummaryReadModel,
    };
    use crate::gateway::state::{GatewayOwnerMode, GatewayOwnerStatus};

    fn sample_compaction_hygiene_state() -> crate::RuntimeSnapshotCompactionHygieneState {
        crate::RuntimeSnapshotCompactionHygieneState {
            strategy: "turn_floor_only".to_owned(),
            diagnostics_surface: "turn_checkpoint".to_owned(),
            evidence_status: "ok".to_owned(),
            trend_scope: "primary_lineage".to_owned(),
            primary_lineage:
                crate::runtime_snapshot_compaction_hygiene::RuntimeSnapshotCompactionLineageState {
                    root_session_id: Some("root-session".to_owned()),
                    sampled_session_count: 2,
                    compaction_sample_count: 2,
                    latest_compaction_status: Some(
                        crate::mvp::conversation::TurnCheckpointProgressStatus::FailedOpen,
                    ),
                    compaction_failure_streak: 1,
                    checkpoint_event_count: 3,
                    checkpoint_failure_streak: 2,
                    checkpoint_repair_action: Some(
                        crate::mvp::conversation::TurnCheckpointRecoveryAction::RunCompaction,
                    ),
                    checkpoint_repair_manual_reason: None,
                },
            overall_window:
                crate::runtime_snapshot_compaction_hygiene::RuntimeSnapshotCompactionHygieneWindow {
                    sampled_session_count: 4,
                    sessions_with_diagnostics: 2,
                    sampled_session_read_errors: 0,
                    failed_open_session_count: 1,
                    total_demoted_recent_turns: 3,
                    total_low_signal_turns: 4,
                    total_tool_result_prunes: 2,
                    total_tool_outcome_prunes: 1,
                },
            recent_window:
                crate::runtime_snapshot_compaction_hygiene::RuntimeSnapshotCompactionHygieneWindow {
                    sampled_session_count: 2,
                    sessions_with_diagnostics: 1,
                    sampled_session_read_errors: 0,
                    failed_open_session_count: 1,
                    total_demoted_recent_turns: 2,
                    total_low_signal_turns: 2,
                    total_tool_result_prunes: 1,
                    total_tool_outcome_prunes: 0,
                },
            baseline_window:
                crate::runtime_snapshot_compaction_hygiene::RuntimeSnapshotCompactionHygieneWindow {
                    sampled_session_count: 2,
                    sessions_with_diagnostics: 1,
                    sampled_session_read_errors: 0,
                    failed_open_session_count: 0,
                    total_demoted_recent_turns: 1,
                    total_low_signal_turns: 2,
                    total_tool_result_prunes: 1,
                    total_tool_outcome_prunes: 1,
                },
            error: None,
        }
    }

    #[test]
    fn build_status_cli_recipes_use_grouped_runtime_commands() {
        let recipes = build_status_cli_recipes("/tmp/config.toml");

        assert_eq!(
            recipes,
            vec![
                "loong gateway status".to_owned(),
                "loong channels --config '/tmp/config.toml' --json".to_owned(),
                "loong runtime acp observability --config '/tmp/config.toml' --json".to_owned(),
                "loong runtime acp sessions --config '/tmp/config.toml' --json".to_owned(),
                "loong runtime work-unit health --config '/tmp/config.toml' --json".to_owned(),
            ]
        );
    }

    #[test]
    fn query_search_checklist_status_treats_disabled_mode_as_non_degraded() {
        let disabled = crate::gateway::read_models::GatewayWebAccessReadModel {
            ordinary_network_access_enabled: true,
            query_search_enabled: false,
            query_search_default_provider: "duckduckgo".to_owned(),
            query_search_credential_ready: false,
            separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
        };
        assert_eq!(
            query_search_checklist_status(&disabled),
            loong_app::tui_surface::TuiChecklistStatus::Pass
        );

        let enabled_missing_credential = crate::gateway::read_models::GatewayWebAccessReadModel {
            ordinary_network_access_enabled: true,
            query_search_enabled: true,
            query_search_default_provider: "brave".to_owned(),
            query_search_credential_ready: false,
            separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
        };
        assert_eq!(
            query_search_checklist_status(&enabled_missing_credential),
            loong_app::tui_surface::TuiChecklistStatus::Warn
        );
    }

    #[test]
    fn render_status_cli_text_surfaces_drill_down_recipes() {
        let gateway = GatewayOperatorSummaryReadModel {
            owner: GatewayOwnerStatus {
                runtime_dir: "/tmp/runtime".to_owned(),
                phase: "running".to_owned(),
                running: true,
                stale: false,
                pid: Some(42),
                mode: GatewayOwnerMode::GatewayHeadless,
                version: "0.0.0-test".to_owned(),
                config_path: "/tmp/config.toml".to_owned(),
                attached_cli_session: None,
                started_at_ms: 1,
                last_heartbeat_at: 2,
                stopped_at_ms: None,
                shutdown_reason: None,
                last_error: None,
                configured_surface_count: 1,
                running_surface_count: 1,
                bind_address: Some("127.0.0.1".to_owned()),
                port: Some(7777),
                port_source: None,
                token_path: Some("/tmp/token".to_owned()),
            },
            control_surface: GatewayOperatorControlSurfaceReadModel {
                base_url: Some("http://127.0.0.1:7777".to_owned()),
                loopback_only: true,
            },
            channels: GatewayOperatorChannelsSummaryReadModel {
                catalog_channel_count: 1,
                configured_channel_count: 1,
                configured_account_count: 1,
                enabled_account_count: 1,
                misconfigured_account_count: 0,
                runtime_backed_channel_count: 1,
                config_backed_channel_count: 0,
                plugin_backed_channel_count: 0,
                catalog_only_channel_count: 0,
                enabled_runtime_backed_channel_count: 1,
                enabled_plugin_backed_channel_count: 0,
                enabled_outbound_only_channel_count: 0,
                enabled_service_channel_count: 1,
                ready_service_channel_count: 1,
                runtime_attention_surface_count: 0,
                retrying_runtime_surface_count: 0,
                stale_runtime_surface_count: 0,
                duplicate_runtime_surface_count: 0,
                runtime_attention_surface_ids: Vec::new(),
                retrying_runtime_surface_ids: Vec::new(),
                stale_runtime_surface_ids: Vec::new(),
                duplicate_runtime_surface_ids: Vec::new(),
                surfaces: Vec::new(),
            },
            runtime: GatewayOperatorRuntimeSummaryReadModel {
                enabled_channel_ids: vec!["telegram".to_owned()],
                enabled_runtime_backed_channel_ids: vec!["telegram".to_owned()],
                enabled_service_channel_ids: vec!["telegram".to_owned()],
                enabled_plugin_backed_channel_ids: Vec::new(),
                enabled_outbound_only_channel_ids: Vec::new(),
                visible_tool_count: 4,
                visible_direct_tool_names: vec!["read".to_owned(), "exec".to_owned()],
                hidden_tool_surface_ids: vec!["agent".to_owned(), "web".to_owned()],
                capability_snapshot_sha256: "abc123".to_owned(),
                active_provider_profile_id: Some("demo".to_owned()),
                active_provider_label: Some("Demo".to_owned()),
                compaction_hygiene: sample_compaction_hygiene_state(),
                tool_calling: crate::gateway::read_models::GatewayToolCallingReadModel {
                    availability: "ready".to_owned(),
                    structured_tool_schema_enabled: true,
                    effective_tool_schema_mode: "enabled_with_downgrade".to_owned(),
                    active_model: "gpt-4.1-mini".to_owned(),
                    reason:
                        "provider turns include structured tool definitions for the active model"
                            .to_owned(),
                },
                web_access: crate::gateway::read_models::GatewayWebAccessReadModel {
                    ordinary_network_access_enabled: true,
                    query_search_enabled: false,
                    query_search_default_provider: "duckduckgo".to_owned(),
                    query_search_credential_ready: true,
                    separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
                },
            },
            pairing: crate::gateway::read_models::GatewayOperatorPairingSummaryReadModel {
                pending_request_count: 0,
                approved_device_count: 0,
                last_activity_ms: None,
            },
            nodes: crate::gateway::read_models::GatewayOperatorNodesSummaryReadModel {
                paired_device_count: 2,
                managed_bridge_count: 1,
                total_count: 3,
            },
        };
        let status = StatusCliReadModel {
            config: "/tmp/config.toml".to_owned(),
            schema: StatusCliJsonSchema {
                version: STATUS_CLI_JSON_SCHEMA_VERSION,
                surface: "status",
                purpose: "operator_runtime_summary",
            },
            active_provider: "Demo [demo]".to_owned(),
            active_model: "gpt-4.1-mini".to_owned(),
            memory_profile: "window_only".to_owned(),
            gateway,
            acp: StatusCliAcpReadModel {
                enabled: false,
                availability: "disabled".to_owned(),
                error: None,
                persisted_session_count: Some(0),
                observability: None,
            },
            work_units: StatusCliWorkUnitReadModel {
                availability: "available".to_owned(),
                error: None,
                health: Some(WorkRuntimeHealthSnapshot {
                    total_count: 0,
                    ready_count: 0,
                    leased_count: 0,
                    running_count: 0,
                    blocked_count: 0,
                    retry_pending_count: 0,
                    terminal_count: 0,
                    archived_count: 0,
                    expired_lease_count: 0,
                }),
            },
            next_actions: vec![StatusCliAction {
                label: "first answer".to_owned(),
                command: "loong ask --config '/tmp/config.toml' --message 'hello'".to_owned(),
            }],
            recipes: vec!["loong gateway status".to_owned()],
        };

        let rendered = render_status_cli_text(&status);

        assert!(rendered.contains("start here"));
        assert!(
            rendered.contains(
                "- first answer: loong ask --config '/tmp/config.toml' --message 'hello'"
            )
        );
        assert!(rendered.contains("runtime posture"));
        assert!(rendered.contains("[OK] tool calling"));
        assert!(rendered.contains("[OK] ordinary network"));
        assert!(rendered.contains("[OK] query search"));
        assert!(rendered.contains("configured channels"));
        assert!(rendered.contains("enabled channels"));
        assert!(rendered.contains("service enabled ids"));
        assert!(rendered.contains("runtime attention surfaces"));
        assert!(rendered.contains("runtime attention ids"));
        assert!(rendered.contains("saved runtime"));
        assert!(rendered.contains("gateway summary"));
        assert!(rendered.contains("paired devices: 2"));
        assert!(rendered.contains("managed bridges: 1"));
        assert!(rendered.contains("known nodes: 3"));
        assert!(rendered.contains("visible tools: 4"));
        assert!(rendered.contains("direct tools: read,exec"));
        assert!(rendered.contains("hidden surfaces: agent,web"));
        assert!(rendered.contains("ordinary network"));
        assert!(rendered.contains("enabled=true"));
        assert!(rendered.contains("query search"));
        assert!(rendered.contains("provider=duckduckgo"));
        assert!(rendered.contains("credential_ready=true"));
        assert!(rendered.contains("web boundary"));
        assert!(rendered.contains("ordinary network access stays separately governed"));
        assert!(rendered.contains("compaction hygiene"));
        assert!(rendered.contains("turn_floor_only"));
        assert!(rendered.contains("posture=degraded"));
        assert!(rendered.contains("surface=turn_checkpoint"));
        assert!(rendered.contains("coverage=2/4 (50.0%)"));
        assert!(rendered.contains("channel and recovery detail"));
        assert!(rendered.contains("enabled channels: telegram"));
        assert!(rendered.contains("service enabled ids: telegram"));
        assert!(rendered.contains("compaction samples"));
        assert!(rendered.contains("compaction prunes"));
        assert!(rendered.contains("compaction pressure"));
        assert!(rendered.contains("compaction trend"));
        assert!(rendered.contains("updated_at_desc"));
        assert!(rendered.contains("scope=primary_lineage"));
        assert!(rendered.contains("root=root-session"));
        assert!(rendered.contains("latest=failed_open"));
        assert!(rendered.contains("failure_streak=1"));
        assert!(rendered.contains("continuity=broken"));
        assert!(rendered.contains("compaction repairability"));
        assert!(rendered.contains("retryable"));
        assert!(rendered.contains("action=run_compaction"));
        assert!(rendered.contains("recovery_posture=retry_exhausted"));
        assert!(rendered.contains("reliability=worsening"));
        assert!(rendered.contains("rate=1/2 (50.0%)"));
        assert!(rendered.contains("demoted_recent=1.500/session"));
        assert!(rendered.contains("capability snapshot: abc123"));
        assert!(rendered.contains("ACP: acp enabled=false availability=disabled"));
        assert!(rendered.contains("deep dives"));
        assert!(rendered.contains("- recipe: loong gateway status"));
    }

    #[test]
    fn collect_status_runtime_attention_actions_prefers_managed_bridge_runtime_diagnostics() {
        let gateway = GatewayOperatorSummaryReadModel {
            owner: GatewayOwnerStatus {
                runtime_dir: "/tmp/runtime".to_owned(),
                phase: "running".to_owned(),
                running: true,
                stale: false,
                pid: Some(42),
                mode: GatewayOwnerMode::GatewayHeadless,
                version: "0.0.0-test".to_owned(),
                config_path: "/tmp/config.toml".to_owned(),
                attached_cli_session: None,
                started_at_ms: 1,
                last_heartbeat_at: 2,
                stopped_at_ms: None,
                shutdown_reason: None,
                last_error: None,
                configured_surface_count: 1,
                running_surface_count: 1,
                bind_address: Some("127.0.0.1".to_owned()),
                port: Some(7777),
                port_source: None,
                token_path: Some("/tmp/token".to_owned()),
            },
            control_surface: GatewayOperatorControlSurfaceReadModel {
                base_url: Some("http://127.0.0.1:7777".to_owned()),
                loopback_only: true,
            },
            channels: GatewayOperatorChannelsSummaryReadModel {
                catalog_channel_count: 3,
                configured_channel_count: 1,
                configured_account_count: 1,
                enabled_account_count: 1,
                misconfigured_account_count: 0,
                runtime_backed_channel_count: 0,
                config_backed_channel_count: 0,
                plugin_backed_channel_count: 3,
                catalog_only_channel_count: 0,
                enabled_runtime_backed_channel_count: 0,
                enabled_plugin_backed_channel_count: 1,
                enabled_outbound_only_channel_count: 0,
                enabled_service_channel_count: 1,
                ready_service_channel_count: 1,
                runtime_attention_surface_count: 1,
                retrying_runtime_surface_count: 1,
                stale_runtime_surface_count: 0,
                duplicate_runtime_surface_count: 0,
                runtime_attention_surface_ids: vec!["weixin".to_owned()],
                retrying_runtime_surface_ids: vec!["weixin".to_owned()],
                stale_runtime_surface_ids: Vec::new(),
                duplicate_runtime_surface_ids: Vec::new(),
                surfaces: vec![
                    crate::gateway::read_models::GatewayOperatorChannelSurfaceReadModel {
                        channel_id: "weixin".to_owned(),
                        label: "Weixin".to_owned(),
                        implementation_status: "plugin_backed".to_owned(),
                        configured_account_count: 1,
                        enabled_account_count: 1,
                        misconfigured_account_count: 0,
                        ready_send_account_count: 1,
                        ready_serve_account_count: 1,
                        conversation_gated_account_count: 0,
                        sender_gated_account_count: 0,
                        mention_gated_account_count: 0,
                        default_configured_account_id: Some("default".to_owned()),
                        plugin_bridge_account_summary: None,
                        runtime_attention_account_count: 1,
                        runtime_attention_reasons: vec!["retrying".to_owned()],
                        runtime_attention_remediations: vec![
                            "inspect_bridge_connectivity".to_owned(),
                        ],
                        retrying_runtime_account_count: 1,
                        stale_runtime_account_count: 0,
                        duplicate_runtime_account_count: 0,
                        preferred_runtime_owner_pids: Vec::new(),
                        duplicate_runtime_cleanup_owner_pids: Vec::new(),
                        last_duplicate_runtime_auto_reclaim_at: None,
                        last_duplicate_runtime_auto_cleanup_owner_pids: Vec::new(),
                        recent_runtime_incidents: Vec::new(),
                        service_enabled: true,
                        service_ready: false,
                    },
                ],
            },
            runtime: GatewayOperatorRuntimeSummaryReadModel {
                enabled_channel_ids: vec!["weixin".to_owned()],
                enabled_runtime_backed_channel_ids: Vec::new(),
                enabled_service_channel_ids: vec!["weixin".to_owned()],
                enabled_plugin_backed_channel_ids: vec!["weixin".to_owned()],
                enabled_outbound_only_channel_ids: Vec::new(),
                visible_tool_count: 4,
                visible_direct_tool_names: vec!["read".to_owned(), "exec".to_owned()],
                hidden_tool_surface_ids: vec!["agent".to_owned(), "web".to_owned()],
                capability_snapshot_sha256: "abc123".to_owned(),
                active_provider_profile_id: Some("demo".to_owned()),
                active_provider_label: Some("Demo".to_owned()),
                compaction_hygiene: sample_compaction_hygiene_state(),
                tool_calling: crate::gateway::read_models::GatewayToolCallingReadModel {
                    availability: "ready".to_owned(),
                    structured_tool_schema_enabled: true,
                    effective_tool_schema_mode: "enabled_with_downgrade".to_owned(),
                    active_model: "gpt-4.1-mini".to_owned(),
                    reason:
                        "provider turns include structured tool definitions for the active model"
                            .to_owned(),
                },
                web_access: crate::gateway::read_models::GatewayWebAccessReadModel {
                    ordinary_network_access_enabled: true,
                    query_search_enabled: false,
                    query_search_default_provider: "duckduckgo".to_owned(),
                    query_search_credential_ready: true,
                    separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
                },
            },
            pairing: crate::gateway::read_models::GatewayOperatorPairingSummaryReadModel {
                pending_request_count: 0,
                approved_device_count: 0,
                last_activity_ms: None,
            },
            nodes: crate::gateway::read_models::GatewayOperatorNodesSummaryReadModel {
                paired_device_count: 0,
                managed_bridge_count: 0,
                total_count: 0,
            },
        };

        let actions = collect_status_runtime_attention_actions("/tmp/config.toml", &gateway);
        let action = actions.first().expect("runtime attention action");

        assert_eq!(
            action.label,
            "inspect weixin managed bridge runtime (retrying)"
        );
        assert_eq!(action.command, "loong doctor --config '/tmp/config.toml'");
    }

    #[test]
    fn collect_status_runtime_attention_actions_include_duplicate_runtime_winner() {
        let gateway = GatewayOperatorSummaryReadModel {
            owner: GatewayOwnerStatus {
                runtime_dir: "/tmp/runtime".to_owned(),
                phase: "running".to_owned(),
                running: true,
                stale: false,
                pid: Some(42),
                mode: GatewayOwnerMode::GatewayHeadless,
                version: "0.0.0-test".to_owned(),
                config_path: "/tmp/config.toml".to_owned(),
                attached_cli_session: None,
                started_at_ms: 1,
                last_heartbeat_at: 2,
                stopped_at_ms: None,
                shutdown_reason: None,
                last_error: None,
                configured_surface_count: 1,
                running_surface_count: 1,
                bind_address: Some("127.0.0.1".to_owned()),
                port: Some(7777),
                port_source: None,
                token_path: Some("/tmp/token".to_owned()),
            },
            control_surface: GatewayOperatorControlSurfaceReadModel {
                base_url: Some("http://127.0.0.1:7777".to_owned()),
                loopback_only: true,
            },
            channels: GatewayOperatorChannelsSummaryReadModel {
                catalog_channel_count: 3,
                configured_channel_count: 1,
                configured_account_count: 1,
                enabled_account_count: 1,
                misconfigured_account_count: 0,
                runtime_backed_channel_count: 0,
                config_backed_channel_count: 0,
                plugin_backed_channel_count: 3,
                catalog_only_channel_count: 0,
                enabled_runtime_backed_channel_count: 0,
                enabled_plugin_backed_channel_count: 1,
                enabled_outbound_only_channel_count: 0,
                enabled_service_channel_count: 1,
                ready_service_channel_count: 0,
                runtime_attention_surface_count: 1,
                retrying_runtime_surface_count: 0,
                stale_runtime_surface_count: 0,
                duplicate_runtime_surface_count: 1,
                runtime_attention_surface_ids: vec!["weixin".to_owned()],
                retrying_runtime_surface_ids: Vec::new(),
                stale_runtime_surface_ids: Vec::new(),
                duplicate_runtime_surface_ids: vec!["weixin".to_owned()],
                surfaces: vec![
                    crate::gateway::read_models::GatewayOperatorChannelSurfaceReadModel {
                        channel_id: "weixin".to_owned(),
                        label: "Weixin".to_owned(),
                        implementation_status: "plugin_backed".to_owned(),
                        configured_account_count: 1,
                        enabled_account_count: 1,
                        misconfigured_account_count: 0,
                        ready_send_account_count: 1,
                        ready_serve_account_count: 1,
                        conversation_gated_account_count: 0,
                        sender_gated_account_count: 0,
                        mention_gated_account_count: 0,
                        default_configured_account_id: Some("default".to_owned()),
                        plugin_bridge_account_summary: None,
                        runtime_attention_account_count: 1,
                        runtime_attention_reasons: vec!["duplicate_runtime_instances".to_owned()],
                        runtime_attention_remediations: vec![
                            "stop_duplicate_runtime_instances".to_owned(),
                        ],
                        retrying_runtime_account_count: 0,
                        stale_runtime_account_count: 0,
                        duplicate_runtime_account_count: 1,
                        preferred_runtime_owner_pids: vec![6262],
                        duplicate_runtime_cleanup_owner_pids: vec![5151],
                        last_duplicate_runtime_auto_reclaim_at: Some(1_700_000_007_000),
                        last_duplicate_runtime_auto_cleanup_owner_pids: vec![5151],
                        recent_runtime_incidents: vec![
                            crate::gateway::read_models::GatewayOperatorRuntimeIncidentReadModel {
                                account_id: Some("default".to_owned()),
                                account_label: Some("default".to_owned()),
                                kind: "duplicate_reclaim".to_owned(),
                                at_ms: 1_700_000_007_000,
                                detail: Some(
                                    "requested cooperative shutdown for duplicate runtime owners"
                                        .to_owned(),
                                ),
                                owner_pids: vec![5151],
                            },
                        ],
                        service_enabled: true,
                        service_ready: false,
                    },
                ],
            },
            runtime: GatewayOperatorRuntimeSummaryReadModel {
                enabled_channel_ids: vec!["weixin".to_owned()],
                enabled_runtime_backed_channel_ids: Vec::new(),
                enabled_service_channel_ids: vec!["weixin".to_owned()],
                enabled_plugin_backed_channel_ids: vec!["weixin".to_owned()],
                enabled_outbound_only_channel_ids: Vec::new(),
                visible_tool_count: 4,
                visible_direct_tool_names: vec!["read".to_owned(), "exec".to_owned()],
                hidden_tool_surface_ids: vec!["agent".to_owned(), "web".to_owned()],
                capability_snapshot_sha256: "abc123".to_owned(),
                active_provider_profile_id: Some("demo".to_owned()),
                active_provider_label: Some("Demo".to_owned()),
                compaction_hygiene: sample_compaction_hygiene_state(),
                tool_calling: crate::gateway::read_models::GatewayToolCallingReadModel {
                    availability: "ready".to_owned(),
                    structured_tool_schema_enabled: true,
                    effective_tool_schema_mode: "enabled_with_downgrade".to_owned(),
                    active_model: "gpt-4.1-mini".to_owned(),
                    reason:
                        "provider turns include structured tool definitions for the active model"
                            .to_owned(),
                },
                web_access: crate::gateway::read_models::GatewayWebAccessReadModel {
                    ordinary_network_access_enabled: true,
                    query_search_enabled: false,
                    query_search_default_provider: "duckduckgo".to_owned(),
                    query_search_credential_ready: true,
                    separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
                },
            },
            pairing: crate::gateway::read_models::GatewayOperatorPairingSummaryReadModel {
                pending_request_count: 0,
                approved_device_count: 0,
                last_activity_ms: None,
            },
            nodes: crate::gateway::read_models::GatewayOperatorNodesSummaryReadModel {
                paired_device_count: 0,
                managed_bridge_count: 0,
                total_count: 0,
            },
        };

        let actions = collect_status_runtime_attention_actions("/tmp/config.toml", &gateway);
        let action = actions.first().expect("runtime attention action");

        assert_eq!(
            action.label,
            "clean up duplicate weixin managed bridge runtimes (keep pid 6262)"
        );
        assert_eq!(action.command, "loong doctor --config '/tmp/config.toml'");
    }

    #[test]
    fn render_status_cli_text_lists_channel_runtime_attention_section() {
        let gateway = GatewayOperatorSummaryReadModel {
            owner: GatewayOwnerStatus {
                runtime_dir: "/tmp/runtime".to_owned(),
                phase: "running".to_owned(),
                running: true,
                stale: false,
                pid: Some(42),
                mode: GatewayOwnerMode::GatewayHeadless,
                version: "0.0.0-test".to_owned(),
                config_path: "/tmp/config.toml".to_owned(),
                attached_cli_session: None,
                started_at_ms: 1,
                last_heartbeat_at: 2,
                stopped_at_ms: None,
                shutdown_reason: None,
                last_error: None,
                configured_surface_count: 1,
                running_surface_count: 1,
                bind_address: Some("127.0.0.1".to_owned()),
                port: Some(7777),
                port_source: None,
                token_path: Some("/tmp/token".to_owned()),
            },
            control_surface: GatewayOperatorControlSurfaceReadModel {
                base_url: Some("http://127.0.0.1:7777".to_owned()),
                loopback_only: true,
            },
            channels: GatewayOperatorChannelsSummaryReadModel {
                catalog_channel_count: 3,
                configured_channel_count: 1,
                configured_account_count: 1,
                enabled_account_count: 1,
                misconfigured_account_count: 0,
                runtime_backed_channel_count: 0,
                config_backed_channel_count: 0,
                plugin_backed_channel_count: 3,
                catalog_only_channel_count: 0,
                enabled_runtime_backed_channel_count: 0,
                enabled_plugin_backed_channel_count: 1,
                enabled_outbound_only_channel_count: 0,
                enabled_service_channel_count: 1,
                ready_service_channel_count: 0,
                runtime_attention_surface_count: 1,
                retrying_runtime_surface_count: 1,
                stale_runtime_surface_count: 0,
                duplicate_runtime_surface_count: 0,
                runtime_attention_surface_ids: vec!["weixin".to_owned()],
                retrying_runtime_surface_ids: vec!["weixin".to_owned()],
                stale_runtime_surface_ids: Vec::new(),
                duplicate_runtime_surface_ids: Vec::new(),
                surfaces: vec![
                    crate::gateway::read_models::GatewayOperatorChannelSurfaceReadModel {
                        channel_id: "weixin".to_owned(),
                        label: "Weixin".to_owned(),
                        implementation_status: "plugin_backed".to_owned(),
                        configured_account_count: 1,
                        enabled_account_count: 1,
                        misconfigured_account_count: 0,
                        ready_send_account_count: 1,
                        ready_serve_account_count: 1,
                        conversation_gated_account_count: 0,
                        sender_gated_account_count: 0,
                        mention_gated_account_count: 0,
                        default_configured_account_id: Some("default".to_owned()),
                        plugin_bridge_account_summary: None,
                        runtime_attention_account_count: 1,
                        runtime_attention_reasons: vec!["retrying".to_owned()],
                        runtime_attention_remediations: vec![
                            "inspect_bridge_connectivity".to_owned(),
                        ],
                        retrying_runtime_account_count: 1,
                        stale_runtime_account_count: 0,
                        duplicate_runtime_account_count: 0,
                        preferred_runtime_owner_pids: Vec::new(),
                        duplicate_runtime_cleanup_owner_pids: Vec::new(),
                        last_duplicate_runtime_auto_reclaim_at: None,
                        last_duplicate_runtime_auto_cleanup_owner_pids: Vec::new(),
                        recent_runtime_incidents: Vec::new(),
                        service_enabled: true,
                        service_ready: false,
                    },
                ],
            },
            runtime: GatewayOperatorRuntimeSummaryReadModel {
                enabled_channel_ids: vec!["weixin".to_owned()],
                enabled_runtime_backed_channel_ids: Vec::new(),
                enabled_service_channel_ids: vec!["weixin".to_owned()],
                enabled_plugin_backed_channel_ids: vec!["weixin".to_owned()],
                enabled_outbound_only_channel_ids: Vec::new(),
                visible_tool_count: 4,
                visible_direct_tool_names: vec!["read".to_owned(), "exec".to_owned()],
                hidden_tool_surface_ids: vec!["agent".to_owned(), "web".to_owned()],
                capability_snapshot_sha256: "abc123".to_owned(),
                active_provider_profile_id: Some("demo".to_owned()),
                active_provider_label: Some("Demo".to_owned()),
                compaction_hygiene: sample_compaction_hygiene_state(),
                tool_calling: crate::gateway::read_models::GatewayToolCallingReadModel {
                    availability: "ready".to_owned(),
                    structured_tool_schema_enabled: true,
                    effective_tool_schema_mode: "enabled_with_downgrade".to_owned(),
                    active_model: "gpt-4.1-mini".to_owned(),
                    reason:
                        "provider turns include structured tool definitions for the active model"
                            .to_owned(),
                },
                web_access: crate::gateway::read_models::GatewayWebAccessReadModel {
                    ordinary_network_access_enabled: true,
                    query_search_enabled: false,
                    query_search_default_provider: "duckduckgo".to_owned(),
                    query_search_credential_ready: true,
                    separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
                },
            },
            pairing: crate::gateway::read_models::GatewayOperatorPairingSummaryReadModel {
                pending_request_count: 0,
                approved_device_count: 0,
                last_activity_ms: None,
            },
            nodes: crate::gateway::read_models::GatewayOperatorNodesSummaryReadModel {
                paired_device_count: 0,
                managed_bridge_count: 0,
                total_count: 0,
            },
        };
        let status = StatusCliReadModel {
            config: "/tmp/config.toml".to_owned(),
            schema: StatusCliJsonSchema {
                version: STATUS_CLI_JSON_SCHEMA_VERSION,
                surface: "status",
                purpose: "operator_runtime_summary",
            },
            active_provider: "Demo [demo]".to_owned(),
            active_model: "gpt-4.1-mini".to_owned(),
            memory_profile: "window_only".to_owned(),
            gateway,
            acp: StatusCliAcpReadModel {
                enabled: false,
                availability: "disabled".to_owned(),
                error: None,
                persisted_session_count: Some(0),
                observability: None,
            },
            work_units: StatusCliWorkUnitReadModel {
                availability: "available".to_owned(),
                error: None,
                health: Some(WorkRuntimeHealthSnapshot {
                    total_count: 0,
                    ready_count: 0,
                    leased_count: 0,
                    running_count: 0,
                    blocked_count: 0,
                    retry_pending_count: 0,
                    terminal_count: 0,
                    archived_count: 0,
                    expired_lease_count: 0,
                }),
            },
            next_actions: vec![StatusCliAction {
                label: "inspect weixin managed bridge runtime (retrying)".to_owned(),
                command: "loong doctor --config '/tmp/config.toml'".to_owned(),
            }],
            recipes: vec!["loong gateway status".to_owned()],
        };

        let rendered = render_status_cli_text(&status);

        assert!(rendered.contains("channel runtime attention"));
        assert!(rendered.contains("[WARN] Weixin"));
        assert!(rendered.contains("channel_id=weixin"));
        assert!(rendered.contains("reasons=retrying"));
        assert!(rendered.contains("remediations=inspect_bridge_connectivity"));
        assert!(rendered.contains("retrying=1"));
        assert!(rendered.contains("duplicate_instances=0"));
        assert!(rendered.contains("affected_accounts=1"));
        assert!(rendered.contains("runtime attention ids"));
        assert!(rendered.contains("weixin"));
        assert!(rendered.contains("ready service channels"));
    }

    #[test]
    fn select_gateway_owner_status_for_config_ignores_mismatched_gateway_owner() {
        let runtime_dir = Path::new("/tmp/runtime");
        let owner_status = GatewayOwnerStatus {
            runtime_dir: runtime_dir.display().to_string(),
            phase: "running".to_owned(),
            running: true,
            stale: false,
            pid: Some(42),
            mode: GatewayOwnerMode::GatewayHeadless,
            version: "0.0.0-test".to_owned(),
            config_path: "/tmp/other-config.toml".to_owned(),
            attached_cli_session: None,
            started_at_ms: 1,
            last_heartbeat_at: 2,
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            configured_surface_count: 1,
            running_surface_count: 1,
            bind_address: None,
            port: None,
            port_source: None,
            token_path: None,
        };

        let selected = select_gateway_owner_status_for_config(
            runtime_dir,
            "/tmp/requested-config.toml",
            Some(owner_status),
        );

        assert_eq!(selected.phase, "stopped");
        assert!(!selected.running);
        assert_eq!(selected.config_path, "-");
    }

    #[test]
    fn select_gateway_owner_status_for_config_keeps_matching_gateway_owner() {
        let runtime_dir = Path::new("/tmp/runtime");
        let owner_status = GatewayOwnerStatus {
            runtime_dir: runtime_dir.display().to_string(),
            phase: "running".to_owned(),
            running: true,
            stale: false,
            pid: Some(42),
            mode: GatewayOwnerMode::GatewayHeadless,
            version: "0.0.0-test".to_owned(),
            config_path: "/tmp/requested-config.toml".to_owned(),
            attached_cli_session: None,
            started_at_ms: 1,
            last_heartbeat_at: 2,
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            configured_surface_count: 1,
            running_surface_count: 1,
            bind_address: None,
            port: None,
            port_source: None,
            token_path: None,
        };

        let selected = select_gateway_owner_status_for_config(
            runtime_dir,
            "/tmp/requested-config.toml",
            Some(owner_status.clone()),
        );

        assert_eq!(selected, owner_status);
    }
}
