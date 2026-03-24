use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use loongclaw_spec::CliResult;
use tokio::task::{Id, JoinSet};

use crate::{mvp, wait_for_shutdown_signal};

type BoxedSupervisorFuture = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BackgroundChannelSurface {
    Telegram { account_id: Option<String> },
    Feishu { account_id: Option<String> },
}

impl BackgroundChannelSurface {
    pub fn all_from_accounts(
        telegram_account: Option<&str>,
        feishu_account: Option<&str>,
    ) -> Vec<Self> {
        vec![
            Self::Telegram {
                account_id: telegram_account.map(str::to_owned),
            },
            Self::Feishu {
                account_id: feishu_account.map(str::to_owned),
            },
        ]
    }
}

impl fmt::Display for BackgroundChannelSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Telegram {
                account_id: Some(account_id),
            } => write!(f, "telegram(account={account_id})"),
            Self::Telegram { account_id: None } => write!(f, "telegram"),
            Self::Feishu {
                account_id: Some(account_id),
            } => write!(f, "feishu(account={account_id})"),
            Self::Feishu { account_id: None } => write!(f, "feishu"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfacePhase {
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceState {
    pub surface: BackgroundChannelSurface,
    pub phase: SurfacePhase,
    pub started_at_ms: Option<u64>,
    pub stopped_at_ms: Option<u64>,
    pub last_error: Option<String>,
    pub exit_reason: Option<String>,
}

impl SurfaceState {
    fn new(surface: BackgroundChannelSurface) -> Self {
        Self {
            surface,
            phase: SurfacePhase::Starting,
            started_at_ms: None,
            stopped_at_ms: None,
            last_error: None,
            exit_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOwnerPhase {
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorShutdownReason {
    Requested {
        reason: String,
    },
    SurfaceFailed {
        surface: BackgroundChannelSurface,
        error: String,
    },
}

impl SupervisorShutdownReason {
    fn surface_exit_reason(&self) -> String {
        match self {
            Self::Requested { reason } => format!("shutdown requested: {reason}"),
            Self::SurfaceFailed { surface, error } => {
                format!("shutdown after {surface} failed: {error}")
            }
        }
    }
}

impl fmt::Display for SupervisorShutdownReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requested { reason } => write!(f, "shutdown requested: {reason}"),
            Self::SurfaceFailed { surface, error } => {
                write!(f, "{surface} exited unexpectedly: {error}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeOwnerMode {
    MultiChannelServe { cli_session: String },
}

impl RuntimeOwnerMode {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::MultiChannelServe { cli_session } => {
                if cli_session.trim().is_empty() {
                    return Err("multi-channel supervisor requires a non-empty CLI session".into());
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorSpec {
    pub mode: RuntimeOwnerMode,
    pub surfaces: Vec<BackgroundChannelSurface>,
}

impl SupervisorSpec {
    pub fn new(
        mode: RuntimeOwnerMode,
        surfaces: Vec<BackgroundChannelSurface>,
    ) -> Result<Self, String> {
        mode.validate()?;
        if surfaces.is_empty() {
            return Err("supervisor requires at least one background surface".to_owned());
        }

        let mut seen = BTreeSet::new();
        for surface in &surfaces {
            if !seen.insert(surface.clone()) {
                return Err(format!(
                    "duplicate background surface configured: {surface}"
                ));
            }
        }

        Ok(Self { mode, surfaces })
    }

    pub fn from_multi_channel_serve(
        session: &str,
        telegram_account: Option<&str>,
        feishu_account: Option<&str>,
    ) -> Result<Self, String> {
        Self::new(
            RuntimeOwnerMode::MultiChannelServe {
                cli_session: session.to_owned(),
            },
            BackgroundChannelSurface::all_from_accounts(telegram_account, feishu_account),
        )
    }

    pub fn from_loaded_multi_channel_serve(
        session: &str,
        config: &mvp::config::LoongClawConfig,
        telegram_account: Option<&str>,
        feishu_account: Option<&str>,
    ) -> Result<Self, String> {
        let mut surfaces = Vec::new();

        if telegram_surface_is_enabled(config, telegram_account)? {
            surfaces.push(BackgroundChannelSurface::Telegram {
                account_id: telegram_account.map(str::to_owned),
            });
        }

        if feishu_surface_is_enabled(config, feishu_account)? {
            surfaces.push(BackgroundChannelSurface::Feishu {
                account_id: feishu_account.map(str::to_owned),
            });
        }

        Self::new(
            RuntimeOwnerMode::MultiChannelServe {
                cli_session: session.to_owned(),
            },
            surfaces,
        )
    }
}

#[derive(Debug, Clone)]
pub struct SupervisorState {
    spec: SupervisorSpec,
    phase: RuntimeOwnerPhase,
    surfaces: BTreeMap<BackgroundChannelSurface, SurfaceState>,
    shutdown_reason: Option<SupervisorShutdownReason>,
}

impl SupervisorState {
    pub fn new(spec: SupervisorSpec) -> Self {
        let surfaces = spec
            .surfaces
            .iter()
            .cloned()
            .map(|surface| {
                let state = SurfaceState::new(surface.clone());
                (surface, state)
            })
            .collect();

        Self {
            spec,
            phase: RuntimeOwnerPhase::Starting,
            surfaces,
            shutdown_reason: None,
        }
    }

    pub fn spec(&self) -> &SupervisorSpec {
        &self.spec
    }

    pub fn phase(&self) -> RuntimeOwnerPhase {
        self.phase
    }

    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_reason.is_some()
    }

    pub fn shutdown_reason(&self) -> Option<&SupervisorShutdownReason> {
        self.shutdown_reason.as_ref()
    }

    pub fn surface_state(&self, surface: &BackgroundChannelSurface) -> Option<&SurfaceState> {
        self.surfaces.get(surface)
    }

    pub fn mark_surface_running(
        &mut self,
        surface: &BackgroundChannelSurface,
        started_at_ms: u64,
    ) -> Result<(), String> {
        let owner_in_shutdown_path = self.shutdown_requested()
            || matches!(
                self.phase,
                RuntimeOwnerPhase::Stopping
                    | RuntimeOwnerPhase::Stopped
                    | RuntimeOwnerPhase::Failed
            );
        let surface_phase = self
            .surface_state(surface)
            .ok_or_else(|| format!("unknown background surface: {surface}"))?
            .phase;
        if owner_in_shutdown_path
            || matches!(
                surface_phase,
                SurfacePhase::Stopping | SurfacePhase::Stopped | SurfacePhase::Failed
            )
        {
            return Err(format!(
                "cannot mark background surface as running after shutdown/failure has begun: \
                 surface={surface}, owner_phase={:?}, surface_phase={surface_phase:?}",
                self.phase
            ));
        }

        let state = self.surface_state_mut(surface)?;
        state.phase = SurfacePhase::Running;
        state.started_at_ms = Some(started_at_ms);
        state.stopped_at_ms = None;
        state.last_error = None;
        state.exit_reason = None;

        if self.all_surfaces_in_phase(SurfacePhase::Running) {
            self.phase = RuntimeOwnerPhase::Running;
        } else if !matches!(
            self.phase,
            RuntimeOwnerPhase::Stopping | RuntimeOwnerPhase::Stopped | RuntimeOwnerPhase::Failed
        ) {
            self.phase = RuntimeOwnerPhase::Starting;
        }

        Ok(())
    }

    pub fn request_shutdown(&mut self, reason: String) -> Result<(), String> {
        if self.shutdown_reason.is_some() {
            return Ok(());
        }

        if reason.trim().is_empty() {
            return Err("shutdown reason cannot be empty".to_owned());
        }

        self.shutdown_reason = Some(SupervisorShutdownReason::Requested { reason });
        self.phase = RuntimeOwnerPhase::Stopping;

        for state in self.surfaces.values_mut() {
            if matches!(state.phase, SurfacePhase::Running | SurfacePhase::Starting) {
                state.phase = SurfacePhase::Stopping;
            }
        }

        Ok(())
    }

    pub fn record_surface_failure(
        &mut self,
        surface: &BackgroundChannelSurface,
        stopped_at_ms: u64,
        error: impl Into<String>,
    ) -> Result<(), String> {
        let current_phase = self
            .surface_state(surface)
            .ok_or_else(|| format!("unknown background surface: {surface}"))?
            .phase;
        if matches!(current_phase, SurfacePhase::Stopped | SurfacePhase::Failed) {
            return Ok(());
        }

        let error = error.into();
        let preserve_shutdown_reason = matches!(
            self.shutdown_reason,
            Some(SupervisorShutdownReason::SurfaceFailed { .. })
        );
        let state = self.surface_state_mut(surface)?;
        state.phase = SurfacePhase::Failed;
        state.stopped_at_ms = Some(stopped_at_ms);
        state.last_error = Some(error.clone());
        state.exit_reason = Some(format!("surface failed: {error}"));

        self.phase = RuntimeOwnerPhase::Failed;
        if !preserve_shutdown_reason {
            self.shutdown_reason = Some(SupervisorShutdownReason::SurfaceFailed {
                surface: surface.clone(),
                error,
            });
        }

        for (tracked_surface, tracked_state) in &mut self.surfaces {
            if tracked_surface != surface
                && matches!(
                    tracked_state.phase,
                    SurfacePhase::Starting | SurfacePhase::Running
                )
            {
                tracked_state.phase = SurfacePhase::Stopping;
            }
        }

        Ok(())
    }

    pub fn mark_surface_stopped(
        &mut self,
        surface: &BackgroundChannelSurface,
        stopped_at_ms: u64,
    ) -> Result<(), String> {
        let current_phase = self
            .surface_state(surface)
            .ok_or_else(|| format!("unknown background surface: {surface}"))?
            .phase;
        if current_phase == SurfacePhase::Failed {
            let state = self.surface_state_mut(surface)?;
            state.stopped_at_ms = Some(stopped_at_ms);
            return Ok(());
        }

        if self.shutdown_reason.is_none() {
            return self.record_surface_failure(
                surface,
                stopped_at_ms,
                "surface stopped unexpectedly without a shutdown request",
            );
        }

        let exit_reason = self.shutdown_reason.as_ref().map(|reason| match reason {
            SupervisorShutdownReason::Requested { reason } => {
                format!("shutdown requested: {reason}")
            }
            SupervisorShutdownReason::SurfaceFailed {
                surface: failed_surface,
                error,
            } if failed_surface == surface => format!("surface failed: {error}"),
            reason => reason.surface_exit_reason(),
        });

        let state = self.surface_state_mut(surface)?;
        state.phase = SurfacePhase::Stopped;
        state.stopped_at_ms = Some(stopped_at_ms);
        if state.exit_reason.is_none() {
            state.exit_reason = exit_reason;
        }

        if self.all_surfaces_terminal() && !matches!(self.phase, RuntimeOwnerPhase::Failed) {
            self.phase = RuntimeOwnerPhase::Stopped;
        }

        Ok(())
    }

    pub fn failure_summary(&self) -> Option<String> {
        match self.shutdown_reason() {
            Some(SupervisorShutdownReason::SurfaceFailed { surface, error }) => Some(format!(
                "multi-channel supervisor failed because {surface} exited unexpectedly: {error}"
            )),
            _ => self
                .surfaces
                .values()
                .find(|state| state.phase == SurfacePhase::Failed)
                .map(|state| {
                    let error = state
                        .last_error
                        .as_deref()
                        .unwrap_or("unknown background surface failure");
                    format!(
                        "multi-channel supervisor failed because {} exited unexpectedly: {error}",
                        state.surface
                    )
                }),
        }
    }

    pub fn final_exit_summary(&self) -> String {
        if let Some(summary) = self.failure_summary() {
            return summary;
        }

        if self.all_surfaces_terminal() && self.shutdown_reason().is_none() {
            let surfaces = self
                .spec
                .surfaces
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return format!(
                "multi-channel supervisor failed because surfaces stopped without a shutdown request: {surfaces}"
            );
        }

        let surfaces = self
            .spec
            .surfaces
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        match self.shutdown_reason() {
            Some(reason) => format!(
                "multi-channel supervisor exited cleanly after {reason}; surfaces: {surfaces}"
            ),
            None => format!("multi-channel supervisor is still active for surfaces: {surfaces}"),
        }
    }

    pub fn final_exit_result(&self) -> CliResult<()> {
        if let Some(summary) = self.failure_summary() {
            return Err(summary);
        }

        if self.all_surfaces_terminal() {
            return match self.shutdown_reason() {
                Some(SupervisorShutdownReason::Requested { .. }) => Ok(()),
                _ => Err(self.final_exit_summary()),
            };
        }

        Err(self.final_exit_summary())
    }

    fn surface_state_mut(
        &mut self,
        surface: &BackgroundChannelSurface,
    ) -> Result<&mut SurfaceState, String> {
        self.surfaces
            .get_mut(surface)
            .ok_or_else(|| format!("unknown background surface: {surface}"))
    }

    fn all_surfaces_in_phase(&self, phase: SurfacePhase) -> bool {
        self.surfaces.values().all(|state| state.phase == phase)
    }

    fn all_surfaces_terminal(&self) -> bool {
        self.surfaces
            .values()
            .all(|state| matches!(state.phase, SurfacePhase::Stopped | SurfacePhase::Failed))
    }
}

#[derive(Debug, Clone)]
pub struct LoadedSupervisorConfig {
    pub resolved_path: PathBuf,
    pub config: mvp::config::LoongClawConfig,
}

#[derive(Debug, Clone)]
pub struct BackgroundChannelRunnerRequest {
    pub resolved_path: PathBuf,
    pub config: mvp::config::LoongClawConfig,
    pub account_id: Option<String>,
    pub stop: mvp::channel::ChannelServeStopHandle,
    pub initialize_runtime_environment: bool,
}

#[derive(Clone)]
pub struct SupervisorRuntimeHooks {
    pub load_config:
        Arc<dyn Fn(Option<&str>) -> CliResult<LoadedSupervisorConfig> + Send + Sync + 'static>,
    pub initialize_runtime_environment:
        Arc<dyn Fn(&LoadedSupervisorConfig) + Send + Sync + 'static>,
    pub run_cli_host: Arc<
        dyn Fn(mvp::chat::ConcurrentCliHostOptions) -> BoxedSupervisorFuture
            + Send
            + Sync
            + 'static,
    >,
    pub run_telegram: Arc<
        dyn Fn(BackgroundChannelRunnerRequest) -> BoxedSupervisorFuture + Send + Sync + 'static,
    >,
    pub run_feishu: Arc<
        dyn Fn(BackgroundChannelRunnerRequest) -> BoxedSupervisorFuture + Send + Sync + 'static,
    >,
    pub wait_for_shutdown: Arc<dyn Fn() -> BoxedSupervisorFuture + Send + Sync + 'static>,
}

impl SupervisorRuntimeHooks {
    fn production() -> Self {
        Self {
            load_config: Arc::new(|config_path| {
                let (resolved_path, config) = mvp::config::load(config_path)?;
                Ok(LoadedSupervisorConfig {
                    resolved_path,
                    config,
                })
            }),
            initialize_runtime_environment: Arc::new(|loaded_config| {
                mvp::runtime_env::initialize_runtime_environment(
                    &loaded_config.config,
                    Some(loaded_config.resolved_path.as_path()),
                );
            }),
            run_cli_host: Arc::new(|options| {
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || {
                        mvp::chat::run_concurrent_cli_host(&options)
                    })
                    .await
                    .map_err(|error| format!("concurrent CLI host task failed to join: {error}"))?
                })
            }),
            run_telegram: Arc::new(|request| {
                Box::pin(async move {
                    mvp::channel::run_telegram_channel_with_stop(
                        request.resolved_path,
                        request.config,
                        false,
                        request.account_id.as_deref(),
                        request.stop,
                        request.initialize_runtime_environment,
                    )
                    .await
                })
            }),
            run_feishu: Arc::new(|request| {
                Box::pin(async move {
                    mvp::channel::run_feishu_channel_with_stop(
                        request.resolved_path,
                        request.config,
                        request.account_id.as_deref(),
                        None,
                        None,
                        request.stop,
                        request.initialize_runtime_environment,
                    )
                    .await
                })
            }),
            wait_for_shutdown: Arc::new(|| Box::pin(async { wait_for_shutdown_signal().await })),
        }
    }
}

#[derive(Debug)]
enum BackgroundTaskExit {
    Surface {
        surface: BackgroundChannelSurface,
        result: CliResult<()>,
    },
}

fn telegram_surface_is_enabled(
    config: &mvp::config::LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.telegram.enabled {
        return Ok(false);
    }
    Ok(config.telegram.resolve_account(account_id)?.enabled)
}

fn feishu_surface_is_enabled(
    config: &mvp::config::LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.feishu.enabled {
        return Ok(false);
    }
    Ok(mvp::feishu::resolve_requested_feishu_account(
        &config.feishu,
        account_id,
        "rerun with `--account <configured_account_id>` using one of those configured accounts",
    )?
    .enabled)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn forward_root_shutdown(
    supervisor: &mut SupervisorState,
    cli_shutdown: &mvp::chat::ConcurrentCliShutdown,
    stop_handles: &[mvp::channel::ChannelServeStopHandle],
    signal_active: &mut bool,
) {
    cli_shutdown.request_shutdown();
    for stop in stop_handles {
        stop.request_stop();
    }
    *signal_active = false;
    if matches!(
        supervisor.phase(),
        RuntimeOwnerPhase::Stopping | RuntimeOwnerPhase::Failed | RuntimeOwnerPhase::Stopped
    ) {
        return;
    }
    supervisor.phase = RuntimeOwnerPhase::Stopping;
}

#[doc(hidden)]
pub async fn run_multi_channel_serve_with_hooks_for_test(
    config_path: Option<&str>,
    session: &str,
    telegram_account: Option<&str>,
    feishu_account: Option<&str>,
    hooks: SupervisorRuntimeHooks,
) -> CliResult<SupervisorState> {
    let loaded_config = (hooks.load_config)(config_path)?;
    (hooks.initialize_runtime_environment)(&loaded_config);
    let LoadedSupervisorConfig {
        resolved_path,
        config,
    } = loaded_config;
    let spec = SupervisorSpec::from_loaded_multi_channel_serve(
        session,
        &config,
        telegram_account,
        feishu_account,
    )?;
    let mut supervisor = SupervisorState::new(spec.clone());

    let cli_shutdown = mvp::chat::ConcurrentCliShutdown::new();
    let telegram_stop = mvp::channel::ChannelServeStopHandle::new();
    let feishu_stop = mvp::channel::ChannelServeStopHandle::new();
    let stop_handles = vec![telegram_stop.clone(), feishu_stop.clone()];

    let mut background_tasks = JoinSet::new();
    let mut background_task_surfaces = BTreeMap::<Id, BackgroundChannelSurface>::new();

    for surface in &spec.surfaces {
        supervisor.mark_surface_running(surface, now_ms())?;
        match surface {
            BackgroundChannelSurface::Telegram { account_id } => {
                let request = BackgroundChannelRunnerRequest {
                    resolved_path: resolved_path.clone(),
                    config: config.clone(),
                    account_id: account_id.clone(),
                    stop: telegram_stop.clone(),
                    initialize_runtime_environment: false,
                };
                let run_telegram = hooks.run_telegram.clone();
                let tracked_surface = surface.clone();
                let task_surface = tracked_surface.clone();
                let task_id = background_tasks
                    .spawn(async move {
                        BackgroundTaskExit::Surface {
                            surface: task_surface,
                            result: run_telegram(request).await,
                        }
                    })
                    .id();
                background_task_surfaces.insert(task_id, tracked_surface);
            }
            BackgroundChannelSurface::Feishu { account_id } => {
                let request = BackgroundChannelRunnerRequest {
                    resolved_path: resolved_path.clone(),
                    config: config.clone(),
                    account_id: account_id.clone(),
                    stop: feishu_stop.clone(),
                    initialize_runtime_environment: false,
                };
                let run_feishu = hooks.run_feishu.clone();
                let tracked_surface = surface.clone();
                let task_surface = tracked_surface.clone();
                let task_id = background_tasks
                    .spawn(async move {
                        BackgroundTaskExit::Surface {
                            surface: task_surface,
                            result: run_feishu(request).await,
                        }
                    })
                    .id();
                background_task_surfaces.insert(task_id, tracked_surface);
            }
        }
    }

    let mut cli_host = Box::pin((hooks.run_cli_host)(mvp::chat::ConcurrentCliHostOptions {
        resolved_path: resolved_path.clone(),
        config: config.clone(),
        session_id: session.to_owned(),
        shutdown: cli_shutdown.clone(),
        initialize_runtime_environment: false,
    }));
    let mut cli_active = true;

    let mut shutdown_signal = Box::pin((hooks.wait_for_shutdown)());
    let mut signal_active = true;

    let mut foreground_failure: Option<String> = None;

    while cli_active || !background_tasks.is_empty() {
        tokio::select! {
            cli_result = &mut cli_host, if cli_active => {
                cli_active = false;
                match cli_result {
                    Ok(()) => {
                        if !supervisor.shutdown_requested() {
                            supervisor.request_shutdown("foreground CLI host exited".to_owned())?;
                        }
                        forward_root_shutdown(&mut supervisor, &cli_shutdown, &stop_handles, &mut signal_active);
                    }
                    Err(error) => {
                        foreground_failure = Some(error.clone());
                        if !supervisor.shutdown_requested() {
                            supervisor.request_shutdown(format!("foreground CLI host failed: {error}"))?;
                        }
                        forward_root_shutdown(&mut supervisor, &cli_shutdown, &stop_handles, &mut signal_active);
                    }
                }
            }
            signal_result = &mut shutdown_signal, if signal_active => {
                signal_active = false;
                signal_result?;
                if !supervisor.shutdown_requested() {
                    supervisor.request_shutdown("ctrl-c received".to_owned())?;
                }
                forward_root_shutdown(&mut supervisor, &cli_shutdown, &stop_handles, &mut signal_active);
            }
            Some(joined) = background_tasks.join_next_with_id(), if !background_tasks.is_empty() => {
                match joined {
                    Ok((task_id, BackgroundTaskExit::Surface { surface, result })) => {
                        background_task_surfaces.remove(&task_id);
                        match result {
                            Ok(()) => {
                                supervisor.mark_surface_stopped(&surface, now_ms())?;
                            }
                            Err(error) => {
                                supervisor.record_surface_failure(&surface, now_ms(), error)?;
                            }
                        }
                    }
                    Err(error) => {
                        let Some(surface) = background_task_surfaces.remove(&error.id()) else {
                            return Err(format!(
                                "background channel task failed to join and could not be attributed to a tracked surface: {error}"
                            ));
                        };
                        supervisor.record_surface_failure(
                            &surface,
                            now_ms(),
                            format!("background channel task failed to join: {error}"),
                        )?;
                    }
                }

                if supervisor.shutdown_requested() {
                    forward_root_shutdown(&mut supervisor, &cli_shutdown, &stop_handles, &mut signal_active);
                }
            }
        }
    }

    if let Some(error) = foreground_failure {
        return Err(format!(
            "multi-channel supervisor failed because foreground CLI host exited unexpectedly: {error}"
        ));
    }

    Ok(supervisor)
}

pub async fn run_multi_channel_serve(
    config_path: Option<&str>,
    session: &str,
    telegram_account: Option<&str>,
    feishu_account: Option<&str>,
) -> CliResult<()> {
    let supervisor = run_multi_channel_serve_with_hooks_for_test(
        config_path,
        session,
        telegram_account,
        feishu_account,
        SupervisorRuntimeHooks::production(),
    )
    .await?;
    supervisor.final_exit_result()
}

#[cfg(test)]
mod tests {
    use super::{
        BackgroundChannelSurface, RuntimeOwnerMode, RuntimeOwnerPhase, SupervisorShutdownReason,
        SupervisorSpec, SupervisorState, SurfacePhase,
    };

    fn telegram_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
        BackgroundChannelSurface::Telegram {
            account_id: account_id.map(str::to_owned),
        }
    }

    fn feishu_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
        BackgroundChannelSurface::Feishu {
            account_id: account_id.map(str::to_owned),
        }
    }

    fn sample_spec(surfaces: Vec<BackgroundChannelSurface>) -> Result<SupervisorSpec, String> {
        SupervisorSpec::new(
            RuntimeOwnerMode::MultiChannelServe {
                cli_session: "cli-supervisor".to_owned(),
            },
            surfaces,
        )
    }

    #[tokio::test]
    async fn background_surface_startup_records_start_timestamp_and_running_phase() {
        let telegram = telegram_surface(Some("bot_123456"));
        let mut supervisor =
            SupervisorState::new(sample_spec(vec![telegram.clone()]).expect("build spec"));

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("mark telegram running");

        let state = supervisor
            .surface_state(&telegram)
            .expect("telegram surface should be tracked");
        assert_eq!(state.phase, SurfacePhase::Running);
        assert_eq!(state.started_at_ms, Some(1_710_000_000_000));
        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Running);
    }

    #[tokio::test]
    async fn background_surface_failure_marks_runtime_owner_failed() {
        let telegram = telegram_surface(Some("bot_123456"));
        let feishu = feishu_surface(Some("alerts"));
        let mut supervisor = SupervisorState::new(
            sample_spec(vec![telegram.clone(), feishu.clone()]).expect("build spec"),
        );

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .mark_surface_running(&feishu, 1_710_000_000_100)
            .expect("start feishu");

        supervisor
            .record_surface_failure(
                &telegram,
                1_710_000_000_500,
                "telegram task exited unexpectedly",
            )
            .expect("record telegram failure");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Failed);
        assert!(supervisor.shutdown_requested());
        assert_eq!(
            supervisor.shutdown_reason(),
            Some(&SupervisorShutdownReason::SurfaceFailed {
                surface: telegram.clone(),
                error: "telegram task exited unexpectedly".to_owned(),
            })
        );
        let summary = supervisor
            .failure_summary()
            .expect("failure summary should exist");
        assert!(
            summary.contains("telegram"),
            "summary should name the failed surface: {summary}"
        );
    }

    #[tokio::test]
    async fn graceful_shutdown_marks_running_children_stopping_then_stopped() {
        let telegram = telegram_surface(Some("bot_123456"));
        let feishu = feishu_surface(Some("alerts"));
        let mut supervisor = SupervisorState::new(
            sample_spec(vec![telegram.clone(), feishu.clone()]).expect("build spec"),
        );

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .mark_surface_running(&feishu, 1_710_000_000_100)
            .expect("start feishu");

        supervisor
            .request_shutdown("ctrl-c received".to_owned())
            .expect("request shutdown");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Stopping);
        assert_eq!(
            supervisor
                .surface_state(&telegram)
                .expect("telegram surface")
                .phase,
            SurfacePhase::Stopping
        );
        assert_eq!(
            supervisor
                .surface_state(&feishu)
                .expect("feishu surface")
                .phase,
            SurfacePhase::Stopping
        );

        supervisor
            .mark_surface_stopped(&telegram, 1_710_000_000_800)
            .expect("stop telegram");
        supervisor
            .mark_surface_stopped(&feishu, 1_710_000_000_900)
            .expect("stop feishu");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Stopped);
        assert_eq!(
            supervisor
                .surface_state(&telegram)
                .expect("telegram surface")
                .phase,
            SurfacePhase::Stopped
        );
        assert_eq!(
            supervisor
                .surface_state(&telegram)
                .expect("telegram surface")
                .exit_reason
                .as_deref(),
            Some("shutdown requested: ctrl-c received")
        );
        assert!(supervisor.final_exit_result().is_ok());
    }

    #[tokio::test]
    async fn final_exit_reason_is_recorded_for_failed_child() {
        let telegram = telegram_surface(Some("bot_123456"));
        let mut supervisor =
            SupervisorState::new(sample_spec(vec![telegram.clone()]).expect("build spec"));

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .record_surface_failure(&telegram, 1_710_000_000_500, "lost upstream connection")
            .expect("record telegram failure");

        let state = supervisor
            .surface_state(&telegram)
            .expect("telegram surface should be tracked");
        assert_eq!(state.phase, SurfacePhase::Failed);
        assert_eq!(state.stopped_at_ms, Some(1_710_000_000_500));
        assert_eq!(
            state.last_error.as_deref(),
            Some("lost upstream connection")
        );
        assert_eq!(
            state.exit_reason.as_deref(),
            Some("surface failed: lost upstream connection")
        );
    }

    #[tokio::test]
    async fn child_stop_without_shutdown_request_returns_failure_result() {
        let telegram = telegram_surface(Some("bot_123456"));
        let mut supervisor =
            SupervisorState::new(sample_spec(vec![telegram.clone()]).expect("build spec"));

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .mark_surface_stopped(&telegram, 1_710_000_000_500)
            .expect("record unexpected stop");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Failed);
        let state = supervisor
            .surface_state(&telegram)
            .expect("telegram surface should be tracked");
        assert_eq!(state.phase, SurfacePhase::Failed);
        assert_eq!(
            state.last_error.as_deref(),
            Some("surface stopped unexpectedly without a shutdown request")
        );
        assert_eq!(
            state.exit_reason.as_deref(),
            Some("surface failed: surface stopped unexpectedly without a shutdown request")
        );

        let result = supervisor.final_exit_result();
        let error = result.expect_err("unexpected child exit must fail closed");
        assert!(
            error.contains("telegram"),
            "failure result should name the stopped surface: {error}"
        );
        assert!(
            error.contains("surface stopped unexpectedly without a shutdown request"),
            "failure result should explain the unexpected clean exit: {error}"
        );
    }

    #[tokio::test]
    async fn failed_child_later_reporting_stopped_preserves_failure_phase_and_result() {
        let telegram = telegram_surface(Some("bot_123456"));
        let mut supervisor =
            SupervisorState::new(sample_spec(vec![telegram.clone()]).expect("build spec"));

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .record_surface_failure(&telegram, 1_710_000_000_500, "lost upstream connection")
            .expect("record telegram failure");
        supervisor
            .mark_surface_stopped(&telegram, 1_710_000_000_800)
            .expect("record stop bookkeeping after failure");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Failed);
        let state = supervisor
            .surface_state(&telegram)
            .expect("telegram surface should be tracked");
        assert_eq!(state.phase, SurfacePhase::Failed);
        assert_eq!(state.stopped_at_ms, Some(1_710_000_000_800));
        assert_eq!(
            state.last_error.as_deref(),
            Some("lost upstream connection")
        );
        assert_eq!(
            state.exit_reason.as_deref(),
            Some("surface failed: lost upstream connection")
        );

        let result = supervisor.final_exit_result();
        let error = result.expect_err("failure result must be preserved");
        assert!(
            error.contains("lost upstream connection"),
            "failure result should keep the original failure reason: {error}"
        );
    }

    #[tokio::test]
    async fn duplicate_shutdown_request_preserves_original_reason_and_terminal_phase() {
        let telegram = telegram_surface(Some("bot_123456"));
        let mut supervisor =
            SupervisorState::new(sample_spec(vec![telegram.clone()]).expect("build spec"));

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .request_shutdown("ctrl-c received".to_owned())
            .expect("request shutdown");
        supervisor
            .mark_surface_stopped(&telegram, 1_710_000_000_500)
            .expect("stop telegram");
        supervisor
            .request_shutdown("   ".to_owned())
            .expect("duplicate shutdown should be ignored");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Stopped);
        assert_eq!(
            supervisor.shutdown_reason(),
            Some(&SupervisorShutdownReason::Requested {
                reason: "ctrl-c received".to_owned(),
            })
        );
        assert!(supervisor.final_exit_result().is_ok());
        assert_eq!(
            supervisor
                .surface_state(&telegram)
                .expect("telegram surface")
                .exit_reason
                .as_deref(),
            Some("shutdown requested: ctrl-c received")
        );
    }

    #[tokio::test]
    async fn first_surface_failure_remains_root_cause_when_sibling_fails_during_unwind() {
        let telegram = telegram_surface(Some("bot_123456"));
        let feishu = feishu_surface(Some("alerts"));
        let mut supervisor = SupervisorState::new(
            sample_spec(vec![telegram.clone(), feishu.clone()]).expect("build spec"),
        );

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .mark_surface_running(&feishu, 1_710_000_000_100)
            .expect("start feishu");

        supervisor
            .record_surface_failure(&telegram, 1_710_000_000_500, "telegram failed first")
            .expect("record telegram failure");
        supervisor
            .record_surface_failure(&feishu, 1_710_000_000_700, "feishu failed second")
            .expect("record feishu failure");

        assert_eq!(
            supervisor.shutdown_reason(),
            Some(&SupervisorShutdownReason::SurfaceFailed {
                surface: telegram.clone(),
                error: "telegram failed first".to_owned(),
            })
        );
        let error = supervisor
            .final_exit_result()
            .expect_err("first failure should keep the supervisor in failed state");
        assert!(
            error.contains("telegram failed first"),
            "root-cause summary should preserve the first failure: {error}"
        );
    }

    #[tokio::test]
    async fn late_failure_after_requested_shutdown_does_not_rewrite_clean_stop() {
        let telegram = telegram_surface(Some("bot_123456"));
        let mut supervisor =
            SupervisorState::new(sample_spec(vec![telegram.clone()]).expect("build spec"));

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .request_shutdown("ctrl-c received".to_owned())
            .expect("request shutdown");
        supervisor
            .mark_surface_stopped(&telegram, 1_710_000_000_500)
            .expect("stop telegram");
        supervisor
            .record_surface_failure(
                &telegram,
                1_710_000_000_700,
                "late failure should not rewrite a clean shutdown",
            )
            .expect("late failure after clean shutdown should be ignored");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Stopped);
        assert_eq!(
            supervisor.shutdown_reason(),
            Some(&SupervisorShutdownReason::Requested {
                reason: "ctrl-c received".to_owned(),
            })
        );
        let state = supervisor
            .surface_state(&telegram)
            .expect("telegram surface");
        assert_eq!(state.phase, SurfacePhase::Stopped);
        assert_eq!(state.last_error, None);
        assert_eq!(
            state.exit_reason.as_deref(),
            Some("shutdown requested: ctrl-c received")
        );
        assert!(supervisor.final_exit_result().is_ok());
    }

    #[tokio::test]
    async fn late_failure_after_failure_unwind_stop_does_not_rewrite_surface_state() {
        let telegram = telegram_surface(Some("bot_123456"));
        let feishu = feishu_surface(Some("alerts"));
        let mut supervisor = SupervisorState::new(
            sample_spec(vec![telegram.clone(), feishu.clone()]).expect("build spec"),
        );

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .mark_surface_running(&feishu, 1_710_000_000_100)
            .expect("start feishu");
        supervisor
            .record_surface_failure(&telegram, 1_710_000_000_500, "telegram failed first")
            .expect("record telegram failure");
        supervisor
            .mark_surface_stopped(&feishu, 1_710_000_000_600)
            .expect("stop feishu during unwind");
        supervisor
            .record_surface_failure(&feishu, 1_710_000_000_700, "late feishu failure")
            .expect("late failure after stop should be ignored");

        assert_eq!(
            supervisor.shutdown_reason(),
            Some(&SupervisorShutdownReason::SurfaceFailed {
                surface: telegram.clone(),
                error: "telegram failed first".to_owned(),
            })
        );
        let state = supervisor.surface_state(&feishu).expect("feishu surface");
        assert_eq!(state.phase, SurfacePhase::Stopped);
        assert_eq!(state.last_error, None);
        assert_eq!(
            state.exit_reason.as_deref(),
            Some("shutdown after telegram(account=bot_123456) failed: telegram failed first")
        );
    }

    #[tokio::test]
    async fn shutdown_while_surface_is_starting_does_not_allow_late_running_transition() {
        let telegram = telegram_surface(Some("bot_123456"));
        let feishu = feishu_surface(Some("alerts"));
        let mut supervisor = SupervisorState::new(
            sample_spec(vec![telegram.clone(), feishu.clone()]).expect("build spec"),
        );

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .request_shutdown("ctrl-c received".to_owned())
            .expect("request shutdown");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Stopping);
        assert_eq!(
            supervisor
                .surface_state(&feishu)
                .expect("feishu surface")
                .phase,
            SurfacePhase::Stopping
        );

        let error = supervisor
            .mark_surface_running(&feishu, 1_710_000_000_100)
            .expect_err("late startup completion should be rejected");
        assert!(
            error.contains("cannot mark background surface as running"),
            "unexpected error: {error}"
        );
        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Stopping);
        let feishu_state = supervisor
            .surface_state(&feishu)
            .expect("feishu surface should still be tracked");
        assert_eq!(feishu_state.phase, SurfacePhase::Stopping);
        assert_eq!(feishu_state.started_at_ms, None);
    }

    #[tokio::test]
    async fn sibling_failure_stops_starting_surface_and_rejects_late_running_transition() {
        let telegram = telegram_surface(Some("bot_123456"));
        let feishu = feishu_surface(Some("alerts"));
        let mut supervisor = SupervisorState::new(
            sample_spec(vec![telegram.clone(), feishu.clone()]).expect("build spec"),
        );

        supervisor
            .mark_surface_running(&telegram, 1_710_000_000_000)
            .expect("start telegram");
        supervisor
            .record_surface_failure(
                &telegram,
                1_710_000_000_500,
                "telegram task exited unexpectedly",
            )
            .expect("record telegram failure");

        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Failed);
        let feishu_state = supervisor
            .surface_state(&feishu)
            .expect("feishu surface should still be tracked");
        assert_eq!(feishu_state.phase, SurfacePhase::Stopping);

        let error = supervisor
            .mark_surface_running(&feishu, 1_710_000_000_900)
            .expect_err("late startup completion should be rejected");
        assert!(
            error.contains("cannot mark background surface as running"),
            "unexpected error: {error}"
        );
        assert_eq!(supervisor.phase(), RuntimeOwnerPhase::Failed);
        let feishu_state = supervisor
            .surface_state(&feishu)
            .expect("feishu surface should still be tracked");
        assert_eq!(feishu_state.phase, SurfacePhase::Stopping);
        assert_eq!(feishu_state.started_at_ms, None);
    }
}
