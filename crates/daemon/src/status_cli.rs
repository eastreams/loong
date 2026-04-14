use loongclaw_contracts::WorkRuntimeHealthSnapshot;
use loongclaw_spec::CliResult;
use serde::Serialize;
use std::path::Path;

use crate::gateway::read_models::{
    GatewayAcpObservabilityReadModel, GatewayOperatorSummaryReadModel,
    build_acp_observability_read_model, build_operator_summary_read_model,
    build_runtime_snapshot_read_model,
};
use crate::gateway::service::default_gateway_owner_status;
use crate::gateway::state::{default_gateway_runtime_state_dir, load_gateway_owner_status};
use crate::mvp;
use crate::supervisor::LoadedSupervisorConfig;

const STATUS_CLI_JSON_SCHEMA_VERSION: u32 = 1;

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
pub struct StatusCliReadModel {
    pub config: String,
    pub schema: StatusCliJsonSchema,
    pub gateway: GatewayOperatorSummaryReadModel,
    pub acp: StatusCliAcpReadModel,
    pub work_units: StatusCliWorkUnitReadModel,
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
    let owner_status_option = load_gateway_owner_status(runtime_dir.as_path());
    let owner_status = select_gateway_owner_status_for_config(
        runtime_dir.as_path(),
        config_path_text,
        owner_status_option,
    );
    let gateway =
        build_operator_summary_read_model(&owner_status, &channel_inventory, &runtime_snapshot);
    let acp = collect_status_cli_acp_read_model(config_path_text, &config).await;
    let work_units = collect_status_cli_work_unit_read_model(&config);
    let recipes = build_status_cli_recipes(config_path_text);
    let schema = StatusCliJsonSchema {
        version: STATUS_CLI_JSON_SCHEMA_VERSION,
        surface: "status",
        purpose: "operator_runtime_summary",
    };

    Ok(StatusCliReadModel {
        config: config_path_display,
        schema,
        gateway,
        acp,
        work_units,
        recipes,
    })
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
    config: &mvp::config::LoongClawConfig,
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
    config: &mvp::config::LoongClawConfig,
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
        let repository_result = mvp::work::repository::WorkUnitRepository::new(&memory_config);
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

fn load_persisted_acp_session_count(config: &mvp::config::LoongClawConfig) -> Option<usize> {
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
        format!("{command_name} acp-observability --config {config_arg} --json");
    let acp_sessions_recipe =
        format!("{command_name} list-acp-sessions --config {config_arg} --json");
    let work_units_recipe = format!("{command_name} work-unit health --config {config_arg} --json");

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
    let tool_calling = &runtime.tool_calling;

    let mut lines = Vec::new();
    lines.push(format!("config={}", status.config));
    lines.push(format!(
        "gateway phase={} running={} stale={} mode={} pid={} session={} control_base_url={} owner_config={} loopback_only={} surfaces_configured={} surfaces_running={}",
        owner.phase,
        owner.running,
        owner.stale,
        owner.mode.as_str(),
        owner_pid,
        owner_session,
        base_url,
        owner.config_path,
        control_surface.loopback_only,
        owner.configured_surface_count,
        owner.running_surface_count,
    ));
    lines.push(format!(
        "gateway_shutdown_reason={} gateway_last_error={}",
        owner_shutdown_reason, owner_error,
    ));
    lines.push(format!(
        "channels catalog={} configured={} enabled_accounts={} misconfigured_accounts={} runtime_backed={} enabled_service_channels={} ready_service_channels={}",
        channels.catalog_channel_count,
        channels.configured_account_count,
        channels.enabled_account_count,
        channels.misconfigured_account_count,
        channels.runtime_backed_channel_count,
        channels.enabled_service_channel_count,
        channels.ready_service_channel_count,
    ));
    lines.push(format!(
        "runtime provider_profile={} provider_label={} visible_tool_count={} capability_snapshot_sha256={}",
        active_provider_profile_id,
        active_provider_label,
        runtime.visible_tool_count,
        capability_snapshot_sha256,
    ));
    lines.push(format!(
        "tool_calling availability={} structured_tool_schema_enabled={} mode={} active_model={} reason={}",
        tool_calling.availability,
        tool_calling.structured_tool_schema_enabled,
        tool_calling.effective_tool_schema_mode,
        tool_calling.active_model,
        tool_calling.reason,
    ));
    lines.push(render_status_cli_acp_text(&status.acp));
    lines.push(render_status_cli_work_units_text(&status.work_units));

    if !status.recipes.is_empty() {
        lines.push("recipes:".to_owned());
        for recipe in &status.recipes {
            lines.push(format!("- {recipe}"));
        }
    }

    lines.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::read_models::{
        GatewayOperatorChannelsSummaryReadModel, GatewayOperatorControlSurfaceReadModel,
        GatewayOperatorRuntimeSummaryReadModel,
    };
    use crate::gateway::state::{GatewayOwnerMode, GatewayOwnerStatus, GatewayPortSource};

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
                port_source: Some(GatewayPortSource::Default),
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
                enabled_service_channel_count: 1,
                ready_service_channel_count: 1,
                surfaces: Vec::new(),
            },
            runtime: GatewayOperatorRuntimeSummaryReadModel {
                enabled_channel_ids: vec!["telegram".to_owned()],
                enabled_service_channel_ids: vec!["telegram".to_owned()],
                visible_tool_count: 4,
                capability_snapshot_sha256: "abc123".to_owned(),
                active_provider_profile_id: Some("demo".to_owned()),
                active_provider_label: Some("Demo".to_owned()),
                tool_calling: crate::gateway::read_models::GatewayToolCallingReadModel {
                    availability: "ready".to_owned(),
                    structured_tool_schema_enabled: true,
                    effective_tool_schema_mode: "enabled_with_downgrade".to_owned(),
                    active_model: "gpt-4.1-mini".to_owned(),
                    reason:
                        "provider turns include structured tool definitions for the active model"
                            .to_owned(),
                },
            },
        };
        let status = StatusCliReadModel {
            config: "/tmp/config.toml".to_owned(),
            schema: StatusCliJsonSchema {
                version: STATUS_CLI_JSON_SCHEMA_VERSION,
                surface: "status",
                purpose: "operator_runtime_summary",
            },
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
            recipes: vec!["loong gateway status".to_owned()],
        };

        let rendered = render_status_cli_text(&status);

        assert!(rendered.contains("gateway phase=running"));
        assert!(rendered.contains("tool_calling availability=ready"));
        assert!(rendered.contains("acp enabled=false availability=disabled"));
        assert!(rendered.contains("work_units availability=available total_count=0"));
        assert!(rendered.contains("recipes:\n- loong gateway status"));
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
