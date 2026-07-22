use super::*;
use std::sync::atomic::AtomicU64;

fn build_control_plane_routes() -> Router<ControlPlaneHttpState> {
    Router::new()
        .route("/readyz", get(readyz))
        .route("/healthz", get(healthz))
        .route("/control/challenge", get(control_challenge))
        .route("/control/ping", get(control_ping))
        .route("/control/connect", post(control_connect))
        .route("/control/subscribe", get(control_subscribe))
        .route("/control/snapshot", get(control_snapshot))
        .route("/control/events", get(control_events))
        .route("/session/list", get(session_list))
        .route("/session/read", get(session_read))
        .route("/task/list", get(task_list))
        .route("/task/read", get(task_read))
        .route("/turn/submit", post(turn_submit))
        .route("/turn/result", get(turn_result))
        .route("/turn/stream", get(turn_stream))
        .route("/approval/list", get(approval_list))
        .route("/pairing/list", get(pairing_list))
        .route("/pairing/resolve", post(pairing_resolve))
        .route("/acp/session/list", get(acp_session_list))
        .route("/acp/session/read", get(acp_session_read))
}

fn build_control_plane_router_with_state(state: ControlPlaneHttpState) -> Router {
    build_control_plane_routes().with_state(state)
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_http_state(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    turn_runtime: Option<Arc<ControlPlaneTurnRuntime>>,
    pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    exposure_policy: ControlPlaneExposurePolicy,
) -> Result<ControlPlaneHttpState, String> {
    let kernel_authority = Arc::new(ControlPlaneKernelAuthority::new()?);
    Ok(ControlPlaneHttpState {
        manager,
        connection_counter: Arc::new(AtomicU64::new(0)),
        connection_registry: Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new()),
        challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
        pairing_registry,
        kernel_authority,
        exposure_policy: Arc::new(exposure_policy),
        repository_view,
        acp_view,
        turn_runtime,
    })
}

#[cfg(not(feature = "memory-sqlite"))]
fn build_control_plane_http_state(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    exposure_policy: ControlPlaneExposurePolicy,
) -> Result<ControlPlaneHttpState, String> {
    let kernel_authority = Arc::new(ControlPlaneKernelAuthority::new()?);
    Ok(ControlPlaneHttpState {
        manager,
        connection_counter: Arc::new(AtomicU64::new(0)),
        connection_registry: Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new()),
        challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
        pairing_registry,
        kernel_authority,
        exposure_policy: Arc::new(exposure_policy),
        turn_runtime: None,
    })
}

fn load_control_plane_config(
    config_path: Option<&str>,
) -> CliResult<Option<(std::path::PathBuf, mvp::config::LoongConfig)>> {
    match config_path {
        Some(config_path) => {
            let (resolved_path, config) = mvp::config::load(Some(config_path))?;
            Ok(Some((resolved_path, config)))
        }
        None => Ok(None),
    }
}

fn build_control_plane_turn_runtime(
    loaded_config: Option<&(std::path::PathBuf, mvp::config::LoongConfig)>,
) -> CliResult<Option<Arc<ControlPlaneTurnRuntime>>> {
    match loaded_config {
        Some((resolved_path, config)) => Ok(Some(Arc::new(ControlPlaneTurnRuntime::new(
            resolved_path.clone(),
            config.clone(),
        )?))),
        None => Ok(None),
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_repository_views(
    loaded_config: Option<&(std::path::PathBuf, mvp::config::LoongConfig)>,
    turn_runtime: Option<&ControlPlaneTurnRuntime>,
    current_session_id: Option<&str>,
) -> Result<
    (
        Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
        Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    ),
    String,
> {
    match loaded_config {
        Some((resolved_path, config)) => {
            let turn_runtime = turn_runtime.ok_or_else(|| {
                "configured control-plane repository view requires the shared Runtime".to_owned()
            })?;
            let session_id = current_session_id.unwrap_or("default");
            println!(
                "loong control plane session view rooted at `{session_id}` from {}",
                resolved_path.display()
            );
            Ok((
                Some(Arc::new(
                    mvp::control_plane::ControlPlaneRepositoryView::new(
                        config,
                        Arc::clone(&turn_runtime.runtime),
                        session_id,
                    ),
                )),
                Some(Arc::new(mvp::control_plane::ControlPlaneAcpView::new(
                    config.clone(),
                    session_id,
                ))),
            ))
        }
        None => Ok((None, None)),
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_pairing_registry(
    loaded_config: Option<&(std::path::PathBuf, mvp::config::LoongConfig)>,
) -> CliResult<Arc<mvp::control_plane::ControlPlanePairingRegistry>> {
    match loaded_config {
        Some((_, config)) => Ok(Arc::new(
            mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(
                mvp::session::store::SessionStoreConfig::from_memory_config_without_env_overrides(
                    &config.memory,
                ),
            )?,
        )),
        None => Ok(Arc::new(
            mvp::control_plane::ControlPlanePairingRegistry::new(),
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_control_plane_router_with_runtime(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    turn_runtime: Option<Arc<ControlPlaneTurnRuntime>>,
    pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    exposure_policy: ControlPlaneExposurePolicy,
) -> Result<Router, String> {
    let state = build_control_plane_http_state(
        manager,
        repository_view,
        acp_view,
        turn_runtime,
        pairing_registry,
        exposure_policy,
    )?;
    Ok(build_control_plane_router_with_state(state))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_control_plane_router_with_views(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
) -> Result<Router, String> {
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    let exposure_policy = default_loopback_exposure_policy();
    build_control_plane_router_with_runtime(
        manager,
        repository_view,
        acp_view,
        None,
        pairing_registry,
        exposure_policy,
    )
}

#[cfg(not(feature = "memory-sqlite"))]
fn build_control_plane_router_without_repository(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    exposure_policy: ControlPlaneExposurePolicy,
) -> Result<Router, String> {
    let state = build_control_plane_http_state(
        manager,
        Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new()),
        exposure_policy,
    )?;
    Ok(build_control_plane_router_with_state(state))
}

pub fn build_control_plane_router(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
) -> Result<Router, String> {
    #[cfg(feature = "memory-sqlite")]
    {
        build_control_plane_router_with_views(manager, None, None)
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let exposure_policy = default_loopback_exposure_policy();
        build_control_plane_router_without_repository(manager, exposure_policy)
    }
}

pub async fn run_control_plane_serve_cli(
    config_path: Option<&str>,
    current_session_id: Option<&str>,
    bind_override: Option<&str>,
    port: u16,
) -> CliResult<()> {
    if current_session_id.is_some() && config_path.is_none() {
        return Err("runtime control-plane serve --session requires --config".to_owned());
    }
    let bind_addr = resolve_control_plane_bind_addr(bind_override, port)?;
    let loaded_config = load_control_plane_config(config_path)?;
    let exposure_policy =
        build_control_plane_exposure_policy(bind_addr, loaded_config.as_ref().map(|(_, c)| c))?;
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let turn_runtime = build_control_plane_turn_runtime(loaded_config.as_ref())?;
    #[cfg(feature = "memory-sqlite")]
    let (repository_view, acp_view) = build_control_plane_repository_views(
        loaded_config.as_ref(),
        turn_runtime.as_deref(),
        current_session_id,
    )?;
    #[cfg(feature = "memory-sqlite")]
    let pairing_registry = build_control_plane_pairing_registry(loaded_config.as_ref())?;
    #[cfg(not(feature = "memory-sqlite"))]
    let _ = (config_path, current_session_id);

    #[cfg(feature = "memory-sqlite")]
    let router = build_control_plane_router_with_runtime(
        manager,
        repository_view,
        acp_view,
        turn_runtime,
        pairing_registry,
        exposure_policy,
    )?;
    #[cfg(not(feature = "memory-sqlite"))]
    let router = build_control_plane_router_without_repository(manager, exposure_policy)?;
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|error| format!("bind control-plane listener failed: {error}"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("read control-plane local address failed: {error}"))?;

    println!("loong control plane listening on http://{local_addr}");
    axum::serve(listener, router)
        .await
        .map_err(|error| format!("control-plane listener failed: {error}"))
}
