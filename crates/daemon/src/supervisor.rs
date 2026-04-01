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

use crate::{MultiChannelServeChannelAccount, mvp};

pub(crate) const GATEWAY_CLI_STACK_SIZE: usize = 8 * 1024 * 1024;

type BoxedSupervisorFuture = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'static>>;
type BoxedShutdownFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send + 'static>>;
type BackgroundChannelRunner =
    Arc<dyn Fn(BackgroundChannelRunnerRequest) -> BoxedSupervisorFuture + Send + Sync + 'static>;
type BackgroundChannelRunnerRegistry = BTreeMap<&'static str, BackgroundChannelRunner>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BackgroundChannelSurface {
    channel_id: &'static str,
    platform: mvp::channel::ChannelPlatform,
    account_id: Option<String>,
}

impl BackgroundChannelSurface {
    pub fn new(
        runtime: mvp::channel::ChannelRuntimeCommandDescriptor,
        account_id: Option<&str>,
    ) -> Self {
        let normalized_account_id = account_id.map(str::to_owned);
        Self {
            channel_id: runtime.channel_id,
            platform: runtime.platform,
            account_id: normalized_account_id,
        }
    }

    pub fn channel_id(&self) -> &'static str {
        self.channel_id
    }

    pub fn platform(&self) -> mvp::channel::ChannelPlatform {
        self.platform
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}

impl fmt::Display for BackgroundChannelSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.account_id {
            Some(account_id) => write!(f, "{}(account={account_id})", self.channel_id),
            None => write!(f, "{}", self.channel_id),
        }
    }
}

fn collect_multi_channel_account_overrides(
    channel_accounts: &[MultiChannelServeChannelAccount],
) -> Result<BTreeMap<String, String>, String> {
    let mut collected_accounts = BTreeMap::new();

    for channel_account in channel_accounts {
        let previous_account = collected_accounts.insert(
            channel_account.channel_id.clone(),
            channel_account.account_id.clone(),
        );
        if previous_account.is_some() {
            return Err(format!(
                "duplicate multi-channel account selection configured for `{}`",
                channel_account.channel_id
            ));
        }
    }

    Ok(collected_accounts)
}

pub fn collect_loaded_background_surfaces(
    config: &mvp::config::LoongClawConfig,
    channel_accounts: &[MultiChannelServeChannelAccount],
) -> Result<Vec<BackgroundChannelSurface>, String> {
    let account_overrides = collect_multi_channel_account_overrides(channel_accounts)?;
    let mut surfaces = Vec::new();

    let runtime_descriptors = mvp::channel::background_channel_runtime_descriptors();

    for runtime_descriptor in runtime_descriptors {
        let selected_account_id = account_overrides
            .get(runtime_descriptor.channel_id)
            .map(String::as_str);
        let surface_is_enabled = mvp::channel::is_background_channel_surface_enabled(
            runtime_descriptor.channel_id,
            config,
            selected_account_id,
        )?;
        if !surface_is_enabled {
            continue;
        }

        let surface = BackgroundChannelSurface::new(runtime_descriptor, selected_account_id);
        surfaces.push(surface);
    }

    Ok(surfaces)
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
    GatewayAttachedCli { cli_session: String },
    GatewayHeadless,
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
            Self::GatewayAttachedCli { cli_session } => {
                if cli_session.trim().is_empty() {
                    return Err("gateway attached mode requires a non-empty CLI session".into());
                }
                Ok(())
            }
            Self::GatewayHeadless => Ok(()),
        }
    }

    fn attached_cli_session(&self) -> Option<&str> {
        match self {
            Self::MultiChannelServe { cli_session } => Some(cli_session.as_str()),
            Self::GatewayAttachedCli { cli_session } => Some(cli_session.as_str()),
            Self::GatewayHeadless => None,
        }
    }

    fn owner_label(&self) -> &'static str {
        match self {
            Self::MultiChannelServe { .. } => "multi-channel supervisor",
            Self::GatewayAttachedCli { .. } | Self::GatewayHeadless => "gateway service",
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

    pub fn from_loaded_multi_channel_serve(
        session: &str,
        config: &mvp::config::LoongClawConfig,
        channel_accounts: &[MultiChannelServeChannelAccount],
    ) -> Result<Self, String> {
        let surfaces = collect_loaded_background_surfaces(config, channel_accounts)?;

        if surfaces.is_empty() {
            return Err(
                "multi-channel supervisor requires at least one enabled runtime-backed service channel"
                    .to_owned(),
            );
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

        let initial_phase = if spec.surfaces.is_empty() {
            RuntimeOwnerPhase::Running
        } else {
            RuntimeOwnerPhase::Starting
        };

        Self {
            spec,
            phase: initial_phase,
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
        let owner_label = self.spec.mode.owner_label();
        match self.shutdown_reason() {
            Some(SupervisorShutdownReason::SurfaceFailed { surface, error }) => Some(format!(
                "{owner_label} failed because {surface} exited unexpectedly: {error}"
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
                        "{owner_label} failed because {} exited unexpectedly: {error}",
                        state.surface,
                    )
                }),
        }
    }

    pub fn final_exit_summary(&self) -> String {
        let owner_label = self.spec.mode.owner_label();
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
                "{owner_label} failed because surfaces stopped without a shutdown request: {surfaces}"
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
            Some(reason) => {
                format!("{owner_label} exited cleanly after {reason}; surfaces: {surfaces}")
            }
            None => format!("{owner_label} is still active for surfaces: {surfaces}"),
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

    pub fn finalize_after_runtime_exit(&mut self) {
        if matches!(
            self.phase,
            RuntimeOwnerPhase::Failed | RuntimeOwnerPhase::Stopped
        ) {
            return;
        }
        if self.shutdown_reason.is_some() && self.all_surfaces_terminal() {
            self.phase = RuntimeOwnerPhase::Stopped;
        }
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
    pub background_channel_runners: BackgroundChannelRunnerRegistry,
    pub wait_for_shutdown: Arc<dyn Fn() -> BoxedShutdownFuture + Send + Sync + 'static>,
    pub observe_state: Arc<dyn Fn(&SupervisorState) -> CliResult<()> + Send + Sync + 'static>,
}

impl SupervisorRuntimeHooks {
    fn production_background_channel_runners() -> BackgroundChannelRunnerRegistry {
        let mut runners = BackgroundChannelRunnerRegistry::new();

        let runtime_descriptors = mvp::channel::background_channel_runtime_descriptors();

        for runtime_descriptor in runtime_descriptors {
            let channel_id = runtime_descriptor.channel_id;
            let runner: BackgroundChannelRunner = Arc::new(
                move |request: BackgroundChannelRunnerRequest| -> BoxedSupervisorFuture {
                    Box::pin(async move {
                        mvp::channel::run_background_channel_with_stop(
                            channel_id,
                            request.resolved_path,
                            request.config,
                            request.account_id.as_deref(),
                            request.stop,
                            request.initialize_runtime_environment,
                        )
                        .await
                    })
                },
            );
            runners.insert(channel_id, runner);
        }

        runners
    }

    pub fn production() -> Self {
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
                    let handle = std::thread::Builder::new()
                        .name("gateway-cli-host".to_owned())
                        .stack_size(GATEWAY_CLI_STACK_SIZE)
                        .spawn(move || mvp::chat::run_concurrent_cli_host(&options))
                        .map_err(|error| {
                            format!("failed to spawn gateway CLI host thread: {error}")
                        })?;

                    handle
                        .join()
                        .map_err(|_panic| "gateway CLI host thread panicked".to_owned())?
                })
            }),
            background_channel_runners: Self::production_background_channel_runners(),
            wait_for_shutdown: Arc::new(|| {
                Box::pin(async { crate::wait_for_shutdown_reason().await })
            }),
            observe_state: Arc::new(|_| Ok(())),
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
pub async fn run_supervisor_with_loaded_config_for_test(
    loaded_config: LoadedSupervisorConfig,
    spec: SupervisorSpec,
    hooks: SupervisorRuntimeHooks,
) -> CliResult<SupervisorState> {
    let LoadedSupervisorConfig {
        resolved_path,
        config,
    } = loaded_config;
    let mut supervisor = SupervisorState::new(spec.clone());
    (hooks.observe_state)(&supervisor)?;

    let cli_shutdown = mvp::chat::ConcurrentCliShutdown::new();
    let mut stop_handles = Vec::new();

    let mut background_tasks = JoinSet::new();
    let mut background_task_surfaces = BTreeMap::<Id, BackgroundChannelSurface>::new();

    for surface in &spec.surfaces {
        supervisor.mark_surface_running(surface, now_ms())?;
        let run_background_channel = hooks
            .background_channel_runners
            .get(surface.channel_id())
            .cloned()
            .ok_or_else(|| {
                format!(
                    "missing background channel runner for `{}`",
                    surface.channel_id()
                )
            })?;
        let stop_handle = mvp::channel::ChannelServeStopHandle::new();
        let account_id = surface.account_id().map(str::to_owned);
        let request = BackgroundChannelRunnerRequest {
            resolved_path: resolved_path.clone(),
            config: config.clone(),
            account_id,
            stop: stop_handle.clone(),
            initialize_runtime_environment: false,
        };
        stop_handles.push(stop_handle);

        let tracked_surface = surface.clone();
        let task_surface = tracked_surface.clone();
        let task_id = background_tasks
            .spawn(async move {
                let result = run_background_channel(request).await;
                BackgroundTaskExit::Surface {
                    surface: task_surface,
                    result,
                }
            })
            .id();
        background_task_surfaces.insert(task_id, tracked_surface);
    }
    (hooks.observe_state)(&supervisor)?;

    let mut cli_host = spec.mode.attached_cli_session().map(|session_id| {
        Box::pin((hooks.run_cli_host)(mvp::chat::ConcurrentCliHostOptions {
            resolved_path: resolved_path.clone(),
            config: config.clone(),
            session_id: session_id.to_owned(),
            shutdown: cli_shutdown.clone(),
            initialize_runtime_environment: false,
        }))
    });
    let mut cli_active = cli_host.is_some();

    let mut shutdown_signal = Box::pin((hooks.wait_for_shutdown)());
    let mut signal_active = true;

    let mut foreground_failure: Option<String> = None;

    while cli_active || signal_active || !background_tasks.is_empty() {
        tokio::select! {
            cli_result = async {
                let cli_host = cli_host
                    .as_mut()
                    .expect("cli host future should exist");
                cli_host.await
            }, if cli_active => {
                cli_active = false;
                match cli_result {
                    Ok(()) => {
                        if !supervisor.shutdown_requested() {
                            supervisor.request_shutdown("foreground CLI host exited".to_owned())?;
                        }
                        forward_root_shutdown(&mut supervisor, &cli_shutdown, &stop_handles, &mut signal_active);
                        (hooks.observe_state)(&supervisor)?;
                    }
                    Err(error) => {
                        foreground_failure = Some(error.clone());
                        if !supervisor.shutdown_requested() {
                            supervisor.request_shutdown(format!("foreground CLI host failed: {error}"))?;
                        }
                        forward_root_shutdown(&mut supervisor, &cli_shutdown, &stop_handles, &mut signal_active);
                        (hooks.observe_state)(&supervisor)?;
                    }
                }
            }
            signal_result = &mut shutdown_signal, if signal_active => {
                signal_active = false;
                let shutdown_reason = signal_result?;
                if !supervisor.shutdown_requested() {
                    supervisor.request_shutdown(shutdown_reason)?;
                }
                forward_root_shutdown(&mut supervisor, &cli_shutdown, &stop_handles, &mut signal_active);
                (hooks.observe_state)(&supervisor)?;
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
                (hooks.observe_state)(&supervisor)?;
            }
        }
    }

    supervisor.finalize_after_runtime_exit();
    (hooks.observe_state)(&supervisor)?;

    if let Some(error) = foreground_failure {
        let owner_label = spec.mode.owner_label();
        return Err(format!(
            "{owner_label} failed because foreground CLI host exited unexpectedly: {error}"
        ));
    }

    Ok(supervisor)
}

#[doc(hidden)]
pub async fn run_multi_channel_serve_with_hooks_for_test(
    config_path: Option<&str>,
    session: &str,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
    hooks: SupervisorRuntimeHooks,
) -> CliResult<SupervisorState> {
    let loaded_config = (hooks.load_config)(config_path)?;
    (hooks.initialize_runtime_environment)(&loaded_config);
    let spec = SupervisorSpec::from_loaded_multi_channel_serve(
        session,
        &loaded_config.config,
        &channel_accounts,
    )?;
    run_supervisor_with_loaded_config_for_test(loaded_config, spec, hooks).await
}

pub async fn run_multi_channel_serve(
    config_path: Option<&str>,
    session: &str,
    channel_accounts: Vec<MultiChannelServeChannelAccount>,
) -> CliResult<()> {
    let supervisor = run_multi_channel_serve_with_hooks_for_test(
        config_path,
        session,
        channel_accounts,
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
    use crate::mvp;

    fn telegram_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
        BackgroundChannelSurface::new(
            mvp::channel::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
            account_id,
        )
    }

    fn feishu_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
        BackgroundChannelSurface::new(mvp::channel::FEISHU_RUNTIME_COMMAND_DESCRIPTOR, account_id)
    }

    fn matrix_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
        BackgroundChannelSurface::new(mvp::channel::MATRIX_RUNTIME_COMMAND_DESCRIPTOR, account_id)
    }

    fn wecom_surface(account_id: Option<&str>) -> BackgroundChannelSurface {
        BackgroundChannelSurface::new(mvp::channel::WECOM_RUNTIME_COMMAND_DESCRIPTOR, account_id)
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
    async fn loaded_multi_channel_spec_includes_all_enabled_runtime_backed_service_channels() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.telegram.enabled = true;
        config.feishu.enabled = true;
        config.matrix.enabled = true;
        config.wecom.enabled = true;

        let spec = SupervisorSpec::from_loaded_multi_channel_serve(
            "cli-supervisor",
            &config,
            &[
                crate::MultiChannelServeChannelAccount {
                    channel_id: "telegram".to_owned(),
                    account_id: "bot_123456".to_owned(),
                },
                crate::MultiChannelServeChannelAccount {
                    channel_id: "feishu".to_owned(),
                    account_id: "alerts".to_owned(),
                },
                crate::MultiChannelServeChannelAccount {
                    channel_id: "matrix".to_owned(),
                    account_id: "bridge-sync".to_owned(),
                },
                crate::MultiChannelServeChannelAccount {
                    channel_id: "wecom".to_owned(),
                    account_id: "robot-prod".to_owned(),
                },
            ],
        )
        .expect("build loaded spec");

        assert_eq!(
            spec.surfaces,
            vec![
                telegram_surface(Some("bot_123456")),
                feishu_surface(Some("alerts")),
                matrix_surface(Some("bridge-sync")),
                wecom_surface(Some("robot-prod")),
            ]
        );
    }

    #[test]
    fn loaded_multi_channel_spec_rejects_duplicate_channel_account_overrides() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.telegram.enabled = true;

        let error = SupervisorSpec::from_loaded_multi_channel_serve(
            "cli-supervisor",
            &config,
            &[
                crate::MultiChannelServeChannelAccount {
                    channel_id: "telegram".to_owned(),
                    account_id: "bot_123456".to_owned(),
                },
                crate::MultiChannelServeChannelAccount {
                    channel_id: "telegram".to_owned(),
                    account_id: "bot_backup".to_owned(),
                },
            ],
        )
        .expect_err("duplicate channel selection should fail");

        assert_eq!(
            error,
            "duplicate multi-channel account selection configured for `telegram`"
        );
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

    #[test]
    fn gateway_cli_run_cli_host_uses_increased_stack_size() {
        use std::thread;

        let handle = thread::Builder::new()
            .stack_size(super::GATEWAY_CLI_STACK_SIZE)
            .spawn(|| {
                let deep_recursion_limit = 100_000;
                fn recursive_fn(n: usize) -> usize {
                    if n == 0 { 1 } else { 1 + recursive_fn(n - 1) }
                }
                recursive_fn(deep_recursion_limit)
            })
            .expect("spawn thread with production stack size");

        let result = handle.join();
        assert!(
            result.is_ok(),
            "production stack size ({}MB) should handle deep recursion without overflow",
            super::GATEWAY_CLI_STACK_SIZE / 1024 / 1024
        );

        assert_eq!(super::GATEWAY_CLI_STACK_SIZE, 8 * 1024 * 1024,);

        assert_eq!(
            super::GATEWAY_CLI_STACK_SIZE,
            8 * 1024 * 1024,
            "production stack size constant must be 8MB to prevent gateway stack overflow"
        );
    }
}
