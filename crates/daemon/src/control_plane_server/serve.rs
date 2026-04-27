use super::*;

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_control_plane_router_with_runtime(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    turn_runtime: Option<Arc<ControlPlaneTurnRuntime>>,
    pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    exposure_policy: ControlPlaneExposurePolicy,
) -> Result<Router, String> {
    let kernel_authority = Arc::new(ControlPlaneKernelAuthority::new()?);
    let state = ControlPlaneHttpState {
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
    };

    let router = Router::new()
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
        .with_state(state);
    Ok(router)
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
    let kernel_authority = Arc::new(ControlPlaneKernelAuthority::new()?);
    let state = ControlPlaneHttpState {
        manager,
        connection_counter: Arc::new(AtomicU64::new(0)),
        connection_registry: Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new()),
        challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
        pairing_registry: Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new()),
        kernel_authority,
        exposure_policy: Arc::new(exposure_policy),
        turn_runtime: None,
    };

    let router = Router::new()
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
        .with_state(state);
    Ok(router)
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
    let loaded_config = match config_path {
        Some(config_path) => {
            let (resolved_path, config) = mvp::config::load(Some(config_path))?;
            Some((resolved_path, config))
        }
        None => None,
    };
    let exposure_policy =
        build_control_plane_exposure_policy(bind_addr, loaded_config.as_ref().map(|(_, c)| c))?;
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let turn_runtime = match loaded_config.as_ref() {
        Some((resolved_path, config)) => Some(Arc::new(ControlPlaneTurnRuntime::new(
            resolved_path.clone(),
            config.clone(),
        )?)),
        None => None,
    };
    #[cfg(feature = "memory-sqlite")]
    let (repository_view, acp_view) = match loaded_config.as_ref() {
        Some((resolved_path, config)) => {
            let memory_config =
                mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                    &config.memory,
                );
            let session_store_config =
                mvp::session::store::SessionStoreConfig::from(&memory_config);
            let session_id = current_session_id.unwrap_or("default");
            println!(
                "loong control plane session view rooted at `{session_id}` from {}",
                resolved_path.display()
            );
            (
                Some(Arc::new(
                    mvp::control_plane::ControlPlaneRepositoryView::new(
                        session_store_config,
                        config.tools.clone(),
                        session_id,
                    ),
                )),
                Some(Arc::new(mvp::control_plane::ControlPlaneAcpView::new(
                    config.clone(),
                    session_id,
                ))),
            )
        }
        None => (None, None),
    };
    #[cfg(feature = "memory-sqlite")]
    let pairing_registry = match loaded_config.as_ref() {
        Some((_, config)) => {
            let memory_config =
                mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                    &config.memory,
                );
            let session_store_config =
                mvp::session::store::SessionStoreConfig::from(&memory_config);
            Arc::new(
                mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(
                    session_store_config,
                )?,
            )
        }
        None => Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new()),
    };
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
