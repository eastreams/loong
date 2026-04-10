use std::{path::Path, sync::Arc};

use clap::Subcommand;

use crate::{
    CliResult, MultiChannelServeChannelAccount,
    supervisor::{
        LoadedSupervisorConfig, RuntimeOwnerMode, SupervisorRuntimeHooks, SupervisorSpec,
        collect_loaded_background_surfaces, run_supervisor_with_loaded_config_for_test,
    },
};

use crate::mvp::acp::AcpSessionManager;

use super::control::start_gateway_control_surface;
use super::state::{
    GatewayOwnerMode, GatewayOwnerStatus, GatewayOwnerTracker, GatewayStopRequestOutcome,
    default_gateway_runtime_state_dir, load_gateway_owner_status, request_gateway_stop,
    wait_for_gateway_stop_request,
};

#[derive(Subcommand, Debug)]
pub enum GatewayCommand {
    /// Claim the gateway owner slot and run the gateway runtime
    Run {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long = "channel-account", value_name = "CHANNEL=ACCOUNT")]
        channel_account: Vec<MultiChannelServeChannelAccount>,
    },
    /// Show the persisted gateway owner status
    Status {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Request cooperative shutdown for the active gateway owner
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayRuntimeEntryPoint {
    GatewayRun,
    MultiChannelServeCompatibility,
}

impl GatewayRuntimeEntryPoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::GatewayRun => "gateway_run",
            Self::MultiChannelServeCompatibility => "multi_channel_compat",
        }
    }
}

pub async fn run_gateway_cli(command: GatewayCommand) -> CliResult<()> {
    match command {
        GatewayCommand::Run {
            config,
            session,
            channel_account,
        } => run_gateway_run_cli(config.as_deref(), session.as_deref(), channel_account).await,
        GatewayCommand::Status { json } => run_gateway_status_cli(json),
        GatewayCommand::Stop => run_gateway_stop_cli(),
    }
}

pub async fn run_gateway_run_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
) -> CliResult<()> {
    let runtime_dir = default_gateway_runtime_state_dir();
    let supervisor = run_gateway_runtime_with_hooks_for_test(
        config_path,
        session,
        channel_accounts,
        runtime_dir.as_path(),
        GatewayRuntimeEntryPoint::GatewayRun,
        SupervisorRuntimeHooks::production(),
    )
    .await?;
    supervisor.final_exit_result()
}

pub async fn run_multi_channel_serve_gateway_compat_cli(
    config_path: Option<&str>,
    session: &str,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
) -> CliResult<()> {
    let runtime_dir = default_gateway_runtime_state_dir();
    let supervisor = run_gateway_runtime_with_hooks_for_test(
        config_path,
        Some(session),
        channel_accounts,
        runtime_dir.as_path(),
        GatewayRuntimeEntryPoint::MultiChannelServeCompatibility,
        SupervisorRuntimeHooks::production(),
    )
    .await?;
    supervisor.final_exit_result()
}

async fn run_gateway_runtime_with_hooks_for_test(
    config_path: Option<&str>,
    session: Option<&str>,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
    runtime_dir: &Path,
    entry_point: GatewayRuntimeEntryPoint,
    hooks: SupervisorRuntimeHooks,
) -> CliResult<crate::supervisor::SupervisorState> {
    let loaded_config = (hooks.load_config)(config_path)?;
    (hooks.initialize_runtime_environment)(&loaded_config);
    let spec =
        build_gateway_supervisor_spec(&loaded_config, session, &channel_accounts, entry_point)?;
    let owner_mode = gateway_owner_mode(entry_point, session);
    let configured_surface_count = spec.surfaces.len();
    let resolved_config_path = loaded_config.resolved_path.display().to_string();
    let runtime_dir_display = runtime_dir.display().to_string();
    let attached_cli_session = session.unwrap_or("-");

    tracing::info!(
        target: "loongclaw.gateway",
        entry_point = entry_point.as_str(),
        owner_mode = owner_mode.as_str(),
        config_path = %resolved_config_path,
        runtime_dir = %runtime_dir_display,
        attached_cli_session = %attached_cli_session,
        configured_surface_count,
        "starting gateway runtime"
    );

    let tracker = Arc::new(GatewayOwnerTracker::acquire(
        runtime_dir,
        owner_mode,
        loaded_config.resolved_path.as_path(),
        session,
        spec.surfaces.len(),
    )?);
    let owner_token = tracker.owner_token().to_owned();
    let acp_manager = acquire_gateway_acp_session_manager(
        tracker.as_ref(),
        &loaded_config.config,
        build_gateway_acp_session_manager,
    )?;
    let control_surface_result =
        start_gateway_control_surface(runtime_dir, &loaded_config, Some(acp_manager)).await;
    let control_surface = match control_surface_result {
        Ok(control_surface) => control_surface,
        Err(error) => {
            tracker.finalize_with_error(error.as_str())?;
            return Err(error);
        }
    };
    let binding_result = tracker.set_control_surface_binding(control_surface.binding());
    if let Err(error) = binding_result {
        let shutdown_result = control_surface.shutdown().await;
        let final_error = merge_gateway_runtime_errors(error, shutdown_result.err());
        tracker.finalize_with_error(final_error.as_str())?;
        return Err(final_error);
    }

    let control_binding = control_surface.binding();
    let bind_address = control_binding.bind_address.as_str();
    let port = control_binding.port;
    let token_path = control_binding.token_path.display().to_string();

    tracing::info!(
        target: "loongclaw.gateway",
        entry_point = entry_point.as_str(),
        owner_mode = owner_mode.as_str(),
        configured_surface_count,
        bind_address = %bind_address,
        port,
        token_path = %token_path,
        "gateway control surface is ready"
    );

    let mut runtime_hooks = hooks.clone();
    let original_wait_for_shutdown = hooks.wait_for_shutdown.clone();
    let runtime_dir_for_shutdown = runtime_dir.to_path_buf();
    let control_surface_for_shutdown = control_surface.clone();
    runtime_hooks.wait_for_shutdown = Arc::new(move || {
        let original_wait_for_shutdown = original_wait_for_shutdown.clone();
        let runtime_dir = runtime_dir_for_shutdown.clone();
        let control_surface = control_surface_for_shutdown.clone();
        let owner_token = owner_token.clone();
        Box::pin(async move {
            tokio::select! {
                result = (original_wait_for_shutdown)() => result,
                result = wait_for_gateway_stop_request(runtime_dir.as_path(), owner_token.as_str()) => {
                    result?;
                    Ok("gateway stop requested".to_owned())
                }
                result = control_surface.wait_for_unexpected_exit() => result,
            }
        })
    });

    let tracker_for_observer = tracker.clone();
    runtime_hooks.observe_state =
        Arc::new(move |supervisor| tracker_for_observer.sync_from_supervisor(supervisor));

    let supervisor_result =
        run_supervisor_with_loaded_config_for_test(loaded_config, spec, runtime_hooks).await;
    match supervisor_result {
        Ok(supervisor) => {
            let shutdown_result = control_surface.shutdown().await;
            if let Err(error) = shutdown_result {
                tracker.finalize_with_error(error.as_str())?;
                return Err(error);
            }
            tracker.finalize_from_supervisor(&supervisor)?;
            Ok(supervisor)
        }
        Err(error) => {
            let shutdown_result = control_surface.shutdown().await;
            let final_error = merge_gateway_runtime_errors(error, shutdown_result.err());
            tracker.finalize_with_error(final_error.as_str())?;
            Err(final_error)
        }
    }
}

#[doc(hidden)]
pub async fn run_gateway_run_with_hooks_for_test(
    config_path: Option<&str>,
    session: Option<&str>,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
    runtime_dir: &Path,
    hooks: SupervisorRuntimeHooks,
) -> CliResult<crate::supervisor::SupervisorState> {
    run_gateway_runtime_with_hooks_for_test(
        config_path,
        session,
        channel_accounts,
        runtime_dir,
        GatewayRuntimeEntryPoint::GatewayRun,
        hooks,
    )
    .await
}

#[doc(hidden)]
pub async fn run_multi_channel_serve_gateway_compat_with_hooks_for_test(
    config_path: Option<&str>,
    session: &str,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
    runtime_dir: &Path,
    hooks: SupervisorRuntimeHooks,
) -> CliResult<crate::supervisor::SupervisorState> {
    run_gateway_runtime_with_hooks_for_test(
        config_path,
        Some(session),
        channel_accounts,
        runtime_dir,
        GatewayRuntimeEntryPoint::MultiChannelServeCompatibility,
        hooks,
    )
    .await
}

pub fn run_gateway_status_cli(as_json: bool) -> CliResult<()> {
    let runtime_dir = default_gateway_runtime_state_dir();
    let status = load_gateway_owner_status(runtime_dir.as_path())
        .unwrap_or_else(|| default_gateway_owner_status(runtime_dir.as_path()));

    if as_json {
        let pretty = serde_json::to_string_pretty(&status)
            .map_err(|error| format!("serialize gateway status failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = render_gateway_status_text(&status);
    println!("{rendered}");
    Ok(())
}

pub fn run_gateway_stop_cli() -> CliResult<()> {
    let runtime_dir = default_gateway_runtime_state_dir();
    let outcome = request_gateway_stop(runtime_dir.as_path())?;
    match outcome {
        GatewayStopRequestOutcome::Requested => {
            println!("gateway stop requested");
        }
        GatewayStopRequestOutcome::AlreadyRequested => {
            println!("gateway stop already requested");
        }
        GatewayStopRequestOutcome::AlreadyStopped => {
            println!("gateway is not running");
        }
    }
    Ok(())
}

fn build_gateway_supervisor_spec(
    loaded_config: &LoadedSupervisorConfig,
    session: Option<&str>,
    channel_accounts: &[MultiChannelServeChannelAccount],
    entry_point: GatewayRuntimeEntryPoint,
) -> Result<SupervisorSpec, String> {
    match entry_point {
        GatewayRuntimeEntryPoint::GatewayRun => {
            let surfaces =
                collect_loaded_background_surfaces(&loaded_config.config, channel_accounts)?;
            let mode = gateway_runtime_owner_mode(session)?;
            SupervisorSpec::new(mode, surfaces)
        }
        GatewayRuntimeEntryPoint::MultiChannelServeCompatibility => {
            let session = session.ok_or_else(|| {
                "multi-channel gateway compatibility path requires a CLI session".to_owned()
            })?;
            SupervisorSpec::from_loaded_multi_channel_serve(
                session,
                &loaded_config.config,
                channel_accounts,
            )
        }
    }
}

fn gateway_runtime_owner_mode(session: Option<&str>) -> Result<RuntimeOwnerMode, String> {
    match normalize_optional_text(session) {
        Some(session) => Ok(RuntimeOwnerMode::GatewayAttachedCli {
            cli_session: session,
        }),
        None => Ok(RuntimeOwnerMode::GatewayHeadless),
    }
}

fn gateway_owner_mode(
    entry_point: GatewayRuntimeEntryPoint,
    session: Option<&str>,
) -> GatewayOwnerMode {
    match entry_point {
        GatewayRuntimeEntryPoint::GatewayRun => match normalize_optional_text(session) {
            Some(_) => GatewayOwnerMode::GatewayAttachedCli,
            None => GatewayOwnerMode::GatewayHeadless,
        },
        GatewayRuntimeEntryPoint::MultiChannelServeCompatibility => {
            GatewayOwnerMode::MultiChannelServe
        }
    }
}

pub(crate) fn default_gateway_owner_status(runtime_dir: &Path) -> GatewayOwnerStatus {
    GatewayOwnerStatus {
        runtime_dir: runtime_dir.display().to_string(),
        phase: "stopped".to_owned(),
        running: false,
        stale: false,
        pid: None,
        mode: GatewayOwnerMode::GatewayHeadless,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        config_path: "-".to_owned(),
        attached_cli_session: None,
        started_at_ms: 0,
        last_heartbeat_at: 0,
        stopped_at_ms: None,
        shutdown_reason: None,
        last_error: None,
        configured_surface_count: 0,
        running_surface_count: 0,
        bind_address: None,
        port: None,
        token_path: None,
    }
}

fn build_gateway_acp_session_manager(
    config: &crate::mvp::config::LoongClawConfig,
) -> CliResult<Arc<AcpSessionManager>> {
    let manager = crate::mvp::acp::shared_acp_session_manager(config)?;
    Ok(manager)
}

fn acquire_gateway_acp_session_manager(
    tracker: &GatewayOwnerTracker,
    config: &crate::mvp::config::LoongClawConfig,
    builder: impl FnOnce(&crate::mvp::config::LoongClawConfig) -> CliResult<Arc<AcpSessionManager>>,
) -> CliResult<Arc<AcpSessionManager>> {
    let manager_result = builder(config);
    let manager = match manager_result {
        Ok(manager) => manager,
        Err(error) => {
            tracker.finalize_with_error(error.as_str())?;
            return Err(error);
        }
    };
    Ok(manager)
}

fn render_gateway_status_text(status: &GatewayOwnerStatus) -> String {
    let pid = status
        .pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let session = status.attached_cli_session.as_deref().unwrap_or("-");
    let stopped_at = status
        .stopped_at_ms
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let shutdown_reason = status.shutdown_reason.as_deref().unwrap_or("-");
    let last_error = status.last_error.as_deref().unwrap_or("-");
    let bind_address = status.bind_address.as_deref().unwrap_or("-");
    let port = status
        .port
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let token_path = status.token_path.as_deref().unwrap_or("-");

    format!(
        "runtime_dir={}\nphase={} running={} stale={} pid={} mode={} config={} session={} version={}\nstarted_at_ms={} last_heartbeat_at_ms={} stopped_at_ms={}\nsurfaces configured={} running={}\nshutdown_reason={}\nlast_error={}\nbind_address={} port={} token_path={}",
        status.runtime_dir,
        status.phase,
        status.running,
        status.stale,
        pid,
        status.mode.as_str(),
        status.config_path,
        session,
        status.version,
        status.started_at_ms,
        status.last_heartbeat_at,
        stopped_at,
        status.configured_surface_count,
        status.running_surface_count,
        shutdown_reason,
        last_error,
        bind_address,
        port,
        token_path,
    )
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn merge_gateway_runtime_errors(primary_error: String, secondary_error: Option<String>) -> String {
    let Some(secondary_error) = secondary_error else {
        return primary_error;
    };

    format!("{primary_error}; {secondary_error}")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn unique_runtime_dir(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let process_id = std::process::id();
        let runtime_dir = std::env::temp_dir().join(format!(
            "loongclaw-gateway-service-{label}-{process_id}-{timestamp}"
        ));
        fs::create_dir_all(runtime_dir.as_path()).expect("create runtime dir");
        runtime_dir
    }

    #[tokio::test(flavor = "current_thread")]
    async fn acp_manager_init_failure_marks_gateway_owner_failed() {
        let runtime_dir = unique_runtime_dir("acp-manager-init-failure");
        let config_path = runtime_dir.join("gateway-config.toml");
        let tracker = GatewayOwnerTracker::acquire(
            runtime_dir.as_path(),
            GatewayOwnerMode::GatewayHeadless,
            config_path.as_path(),
            None,
            1,
        )
        .expect("acquire gateway owner tracker");
        let config = crate::mvp::config::LoongClawConfig::default();
        let expected_error = "simulated ACP manager init failure".to_owned();
        let builder = |_config: &crate::mvp::config::LoongClawConfig| Err(expected_error.clone());

        let manager_result = acquire_gateway_acp_session_manager(&tracker, &config, builder);
        let error = match manager_result {
            Ok(_) => panic!("ACP manager init should fail"),
            Err(error) => error,
        };

        assert_eq!(error, expected_error);

        let status = load_gateway_owner_status(runtime_dir.as_path())
            .expect("gateway owner status should be persisted");

        assert_eq!(status.phase, "failed");
        assert!(!status.running);
        assert_eq!(
            status.shutdown_reason.as_deref(),
            Some(expected_error.as_str())
        );
        assert_eq!(status.last_error.as_deref(), Some(expected_error.as_str()));
        assert_eq!(
            request_gateway_stop(runtime_dir.as_path()).expect("query gateway stop outcome"),
            GatewayStopRequestOutcome::AlreadyStopped
        );

        let _ = fs::remove_dir_all(runtime_dir.as_path());
    }
}
