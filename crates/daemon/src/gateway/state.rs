use std::{
    fs,
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::{task::JoinHandle, time::sleep};

use crate::{
    CliResult, mvp,
    supervisor::{RuntimeOwnerPhase, SupervisorState, SurfacePhase},
};

const GATEWAY_RUNTIME_HEARTBEAT_MS: u64 = 5_000;
const GATEWAY_RUNTIME_STALE_MS: u64 = 15_000;
const GATEWAY_STOP_REQUEST_POLL_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayOwnerMode {
    GatewayHeadless,
    GatewayAttachedCli,
    MultiChannelServe,
}

impl GatewayOwnerMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GatewayHeadless => "gateway_headless",
            Self::GatewayAttachedCli => "gateway_attached_cli",
            Self::MultiChannelServe => "multi_channel_serve",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayOwnerStatus {
    pub runtime_dir: String,
    pub phase: String,
    pub running: bool,
    pub stale: bool,
    pub pid: Option<u32>,
    pub mode: GatewayOwnerMode,
    pub version: String,
    pub config_path: String,
    pub attached_cli_session: Option<String>,
    pub started_at_ms: u64,
    pub last_heartbeat_at: u64,
    pub stopped_at_ms: Option<u64>,
    pub shutdown_reason: Option<String>,
    pub last_error: Option<String>,
    pub configured_surface_count: usize,
    pub running_surface_count: usize,
    pub bind_address: Option<String>,
    pub port: Option<u16>,
    pub token_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayControlSurfaceBinding {
    pub bind_address: String,
    pub port: u16,
    pub token_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayStopRequestOutcome {
    Requested,
    AlreadyRequested,
    AlreadyStopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedGatewayOwnerState {
    phase: String,
    running: bool,
    pid: Option<u32>,
    mode: GatewayOwnerMode,
    version: String,
    config_path: String,
    attached_cli_session: Option<String>,
    started_at_ms: u64,
    last_heartbeat_at: u64,
    stopped_at_ms: Option<u64>,
    shutdown_reason: Option<String>,
    last_error: Option<String>,
    configured_surface_count: usize,
    running_surface_count: usize,
    bind_address: Option<String>,
    port: Option<u16>,
    token_path: Option<String>,
    owner_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedGatewayStopRequest {
    requested_at_ms: u64,
    requested_by_pid: u32,
    target_owner_token: String,
}

pub struct GatewayOwnerTracker {
    active_owner_path: PathBuf,
    status_snapshot_path: PathBuf,
    stop_request_path: PathBuf,
    owner_token: String,
    owner_file: Arc<Mutex<fs::File>>,
    state: Arc<Mutex<PersistedGatewayOwnerState>>,
    heartbeat_stopped: Arc<AtomicBool>,
    heartbeat_task: Mutex<Option<JoinHandle<()>>>,
}

impl GatewayOwnerTracker {
    pub fn acquire(
        runtime_dir: &Path,
        mode: GatewayOwnerMode,
        config_path: &Path,
        attached_cli_session: Option<&str>,
        configured_surface_count: usize,
    ) -> CliResult<Self> {
        let active_owner_path = active_gateway_owner_path(runtime_dir);
        let status_snapshot_path = gateway_status_snapshot_path(runtime_dir);
        let stop_request_path = gateway_stop_request_path(runtime_dir);

        let process_id = std::process::id();
        let owner_token = new_gateway_owner_token(process_id);
        let started_at_ms = now_ms();
        let initial_state = PersistedGatewayOwnerState {
            phase: runtime_owner_phase_text(RuntimeOwnerPhase::Starting).to_owned(),
            running: true,
            pid: Some(process_id),
            mode,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            config_path: config_path.display().to_string(),
            attached_cli_session: normalize_optional_text(attached_cli_session),
            started_at_ms,
            last_heartbeat_at: started_at_ms,
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            configured_surface_count,
            running_surface_count: 0,
            bind_address: None,
            port: None,
            token_path: None,
            owner_token: owner_token.clone(),
        };
        let owner_file = acquire_active_owner_file(active_owner_path.as_path(), &initial_state)?;
        write_status_snapshot(status_snapshot_path.as_path(), &initial_state)?;

        let owner_file = Arc::new(Mutex::new(owner_file));
        let state = Arc::new(Mutex::new(initial_state));
        let heartbeat_stopped = Arc::new(AtomicBool::new(false));
        let heartbeat_owner_file = owner_file.clone();
        let heartbeat_state = state.clone();
        let heartbeat_stopped_flag = heartbeat_stopped.clone();
        let heartbeat_status_path = status_snapshot_path.clone();
        let heartbeat_task = tokio::spawn(async move {
            while !heartbeat_stopped_flag.load(Ordering::SeqCst) {
                sleep(Duration::from_millis(GATEWAY_RUNTIME_HEARTBEAT_MS)).await;
                if heartbeat_stopped_flag.load(Ordering::SeqCst) {
                    break;
                }

                let persisted_state = {
                    let state_guard = heartbeat_state.lock();
                    let Ok(mut state_guard) = state_guard else {
                        break;
                    };
                    state_guard.last_heartbeat_at = now_ms();
                    state_guard.clone()
                };

                let write_owner_result =
                    write_active_owner_state(heartbeat_owner_file.as_ref(), &persisted_state);
                if write_owner_result.is_err() {
                    break;
                }

                let write_status_result =
                    write_status_snapshot(heartbeat_status_path.as_path(), &persisted_state);
                if write_status_result.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            active_owner_path,
            status_snapshot_path,
            stop_request_path,
            owner_token,
            owner_file,
            state,
            heartbeat_stopped,
            heartbeat_task: Mutex::new(Some(heartbeat_task)),
        })
    }

    pub fn owner_token(&self) -> &str {
        self.owner_token.as_str()
    }

    pub fn sync_from_supervisor(&self, supervisor: &SupervisorState) -> CliResult<()> {
        let running_surface_count = count_running_surfaces(supervisor);
        let shutdown_reason = supervisor.shutdown_reason().map(ToString::to_string);
        let last_error = match supervisor.phase() {
            RuntimeOwnerPhase::Failed => supervisor.failure_summary(),
            RuntimeOwnerPhase::Starting
            | RuntimeOwnerPhase::Running
            | RuntimeOwnerPhase::Stopping
            | RuntimeOwnerPhase::Stopped => None,
        };
        let runtime_phase = runtime_owner_phase_text(supervisor.phase());

        self.update_state(
            runtime_phase,
            true,
            None,
            shutdown_reason,
            last_error,
            running_surface_count,
        )
    }

    pub fn set_control_surface_binding(
        &self,
        binding: &GatewayControlSurfaceBinding,
    ) -> CliResult<()> {
        let persisted_state = {
            let state_guard = self.state.lock();
            let mut state_guard = state_guard
                .map_err(|error| format!("gateway owner state lock poisoned: {error}"))?;
            state_guard.bind_address = Some(binding.bind_address.clone());
            state_guard.port = Some(binding.port);
            state_guard.token_path = Some(binding.token_path.display().to_string());
            state_guard.last_heartbeat_at = now_ms();
            state_guard.clone()
        };

        write_active_owner_state(self.owner_file.as_ref(), &persisted_state)?;
        write_status_snapshot(self.status_snapshot_path.as_path(), &persisted_state)
    }

    pub fn finalize_from_supervisor(&self, supervisor: &SupervisorState) -> CliResult<()> {
        self.heartbeat_stopped.store(true, Ordering::SeqCst);
        let heartbeat_task = self
            .heartbeat_task
            .lock()
            .map_err(|error| format!("gateway owner heartbeat task lock poisoned: {error}"))?
            .take();
        if let Some(heartbeat_task) = heartbeat_task {
            heartbeat_task.abort();
        }

        let running_surface_count = count_running_surfaces(supervisor);
        let shutdown_reason = supervisor.shutdown_reason().map(ToString::to_string);
        let last_error = match supervisor.phase() {
            RuntimeOwnerPhase::Failed => supervisor.failure_summary(),
            RuntimeOwnerPhase::Starting
            | RuntimeOwnerPhase::Running
            | RuntimeOwnerPhase::Stopping
            | RuntimeOwnerPhase::Stopped => None,
        };
        let final_phase = runtime_owner_phase_text(supervisor.phase());

        self.update_state(
            final_phase,
            false,
            Some(now_ms()),
            shutdown_reason,
            last_error,
            running_surface_count,
        )?;
        remove_stop_request_file(self.stop_request_path.as_path())?;
        remove_active_owner_if_owned(self.active_owner_path.as_path(), self.owner_token.as_str())
    }

    pub fn finalize_with_error(&self, error: &str) -> CliResult<()> {
        self.heartbeat_stopped.store(true, Ordering::SeqCst);
        let heartbeat_task = self
            .heartbeat_task
            .lock()
            .map_err(|lock_error| {
                format!("gateway owner heartbeat task lock poisoned: {lock_error}")
            })?
            .take();
        if let Some(heartbeat_task) = heartbeat_task {
            heartbeat_task.abort();
        }

        self.update_state(
            "failed",
            false,
            Some(now_ms()),
            Some(error.to_owned()),
            Some(error.to_owned()),
            0,
        )?;
        remove_stop_request_file(self.stop_request_path.as_path())?;
        remove_active_owner_if_owned(self.active_owner_path.as_path(), self.owner_token.as_str())
    }

    fn update_state(
        &self,
        phase: &str,
        running: bool,
        stopped_at_ms: Option<u64>,
        shutdown_reason: Option<String>,
        last_error: Option<String>,
        running_surface_count: usize,
    ) -> CliResult<()> {
        let persisted_state = {
            let state_guard = self.state.lock();
            let mut state_guard = state_guard
                .map_err(|error| format!("gateway owner state lock poisoned: {error}"))?;
            state_guard.phase = phase.to_owned();
            state_guard.running = running;
            state_guard.running_surface_count = running_surface_count;
            state_guard.shutdown_reason = shutdown_reason;
            state_guard.last_error = last_error;
            state_guard.last_heartbeat_at = now_ms();
            if let Some(stopped_at_ms) = stopped_at_ms {
                state_guard.stopped_at_ms = Some(stopped_at_ms);
            }
            if !running {
                state_guard.bind_address = None;
                state_guard.port = None;
                state_guard.token_path = None;
            }
            state_guard.clone()
        };

        if running {
            write_active_owner_state(self.owner_file.as_ref(), &persisted_state)?;
        }
        write_status_snapshot(self.status_snapshot_path.as_path(), &persisted_state)
    }
}

impl Drop for GatewayOwnerTracker {
    fn drop(&mut self) {
        self.heartbeat_stopped.store(true, Ordering::SeqCst);
        if let Ok(mut heartbeat_task) = self.heartbeat_task.lock() {
            let heartbeat_task = heartbeat_task.take();
            if let Some(heartbeat_task) = heartbeat_task {
                heartbeat_task.abort();
            }
        }
    }
}

pub fn default_gateway_runtime_state_dir() -> PathBuf {
    mvp::config::default_loongclaw_home().join("gateway-runtime")
}

pub fn load_gateway_owner_status(runtime_dir: &Path) -> Option<GatewayOwnerStatus> {
    let now_ms = now_ms();
    let status_snapshot_path = gateway_status_snapshot_path(runtime_dir);
    let active_owner_path = active_gateway_owner_path(runtime_dir);
    let status_snapshot = read_persisted_gateway_owner_state(status_snapshot_path.as_path());
    let active_owner_snapshot = read_persisted_gateway_owner_state(active_owner_path.as_path());
    let persisted_state = select_preferred_snapshot(status_snapshot, active_owner_snapshot)?;
    Some(build_gateway_owner_status(
        runtime_dir,
        &persisted_state,
        now_ms,
    ))
}

pub fn request_gateway_stop(runtime_dir: &Path) -> CliResult<GatewayStopRequestOutcome> {
    let active_owner_path = active_gateway_owner_path(runtime_dir);
    let stop_request_path = gateway_stop_request_path(runtime_dir);
    let now_ms = now_ms();
    let active_owner = read_persisted_gateway_owner_state(active_owner_path.as_path());
    let Some(active_owner) = active_owner else {
        return Ok(GatewayStopRequestOutcome::AlreadyStopped);
    };
    let active_owner_status = build_gateway_owner_status(runtime_dir, &active_owner, now_ms);
    if !active_owner_status.running || active_owner_status.stale {
        return Ok(GatewayStopRequestOutcome::AlreadyStopped);
    }
    let existing_stop_request = read_persisted_gateway_stop_request(stop_request_path.as_path());
    let stop_request_already_targets_active_owner = existing_stop_request
        .as_ref()
        .map(|stop_request| stop_request.target_owner_token == active_owner.owner_token)
        .unwrap_or(false);
    if stop_request_already_targets_active_owner {
        return Ok(GatewayStopRequestOutcome::AlreadyRequested);
    }

    let stop_request = PersistedGatewayStopRequest {
        requested_at_ms: now_ms,
        requested_by_pid: std::process::id(),
        target_owner_token: active_owner.owner_token,
    };
    write_json_path(
        stop_request_path.as_path(),
        &stop_request,
        "gateway stop request",
    )?;
    Ok(GatewayStopRequestOutcome::Requested)
}

pub async fn wait_for_gateway_stop_request(runtime_dir: &Path, owner_token: &str) -> CliResult<()> {
    let stop_request_path = gateway_stop_request_path(runtime_dir);
    loop {
        let stop_request = read_persisted_gateway_stop_request(stop_request_path.as_path());
        let stop_request_targets_owner = stop_request
            .as_ref()
            .map(|stop_request| stop_request.target_owner_token == owner_token)
            .unwrap_or(false);
        if stop_request_targets_owner {
            return Ok(());
        }
        sleep(Duration::from_millis(GATEWAY_STOP_REQUEST_POLL_MS)).await;
    }
}

#[cfg(test)]
pub(crate) fn write_gateway_owner_snapshot_for_test(
    runtime_dir: &Path,
    persisted_state: &GatewayOwnerStatus,
) -> CliResult<()> {
    let status_snapshot_path = gateway_status_snapshot_path(runtime_dir);
    let owner_token = "test-owner".to_owned();
    let raw_state = PersistedGatewayOwnerState {
        phase: persisted_state.phase.clone(),
        running: persisted_state.running,
        pid: persisted_state.pid,
        mode: persisted_state.mode,
        version: persisted_state.version.clone(),
        config_path: persisted_state.config_path.clone(),
        attached_cli_session: persisted_state.attached_cli_session.clone(),
        started_at_ms: persisted_state.started_at_ms,
        last_heartbeat_at: persisted_state.last_heartbeat_at,
        stopped_at_ms: persisted_state.stopped_at_ms,
        shutdown_reason: persisted_state.shutdown_reason.clone(),
        last_error: persisted_state.last_error.clone(),
        configured_surface_count: persisted_state.configured_surface_count,
        running_surface_count: persisted_state.running_surface_count,
        bind_address: persisted_state.bind_address.clone(),
        port: persisted_state.port,
        token_path: persisted_state.token_path.clone(),
        owner_token,
    };
    write_status_snapshot(status_snapshot_path.as_path(), &raw_state)
}

fn select_preferred_snapshot(
    status_snapshot: Option<PersistedGatewayOwnerState>,
    active_owner_snapshot: Option<PersistedGatewayOwnerState>,
) -> Option<PersistedGatewayOwnerState> {
    match (status_snapshot, active_owner_snapshot) {
        (Some(status_snapshot), Some(active_owner_snapshot)) => {
            if active_owner_snapshot.last_heartbeat_at >= status_snapshot.last_heartbeat_at {
                return Some(active_owner_snapshot);
            }
            Some(status_snapshot)
        }
        (Some(status_snapshot), None) => Some(status_snapshot),
        (None, Some(active_owner_snapshot)) => Some(active_owner_snapshot),
        (None, None) => None,
    }
}

fn build_gateway_owner_status(
    runtime_dir: &Path,
    persisted_state: &PersistedGatewayOwnerState,
    now_ms: u64,
) -> GatewayOwnerStatus {
    let stale = persisted_gateway_owner_is_stale(persisted_state, now_ms);
    let running = persisted_state.running && !stale;

    GatewayOwnerStatus {
        runtime_dir: runtime_dir.display().to_string(),
        phase: persisted_state.phase.clone(),
        running,
        stale,
        pid: persisted_state.pid,
        mode: persisted_state.mode,
        version: persisted_state.version.clone(),
        config_path: persisted_state.config_path.clone(),
        attached_cli_session: persisted_state.attached_cli_session.clone(),
        started_at_ms: persisted_state.started_at_ms,
        last_heartbeat_at: persisted_state.last_heartbeat_at,
        stopped_at_ms: persisted_state.stopped_at_ms,
        shutdown_reason: persisted_state.shutdown_reason.clone(),
        last_error: persisted_state.last_error.clone(),
        configured_surface_count: persisted_state.configured_surface_count,
        running_surface_count: persisted_state.running_surface_count,
        bind_address: persisted_state.bind_address.clone(),
        port: persisted_state.port,
        token_path: persisted_state.token_path.clone(),
    }
}

fn count_running_surfaces(supervisor: &SupervisorState) -> usize {
    let mut count = 0_usize;
    for surface in &supervisor.spec().surfaces {
        let surface_state = supervisor.surface_state(surface);
        let Some(surface_state) = surface_state else {
            continue;
        };
        if surface_state.phase == SurfacePhase::Running {
            count = count.saturating_add(1);
        }
    }
    count
}

fn active_gateway_owner_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("owner-active.json")
}

fn gateway_status_snapshot_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("status.json")
}

fn gateway_stop_request_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("stop-request.json")
}

pub fn gateway_control_token_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("control-token")
}

fn read_persisted_gateway_owner_state(path: &Path) -> Option<PersistedGatewayOwnerState> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<PersistedGatewayOwnerState>(&raw).ok()
}

fn read_persisted_gateway_stop_request(path: &Path) -> Option<PersistedGatewayStopRequest> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<PersistedGatewayStopRequest>(&raw).ok()
}

fn persisted_gateway_owner_is_stale(
    persisted_state: &PersistedGatewayOwnerState,
    now_ms: u64,
) -> bool {
    if !persisted_state.running {
        return false;
    }

    now_ms.saturating_sub(persisted_state.last_heartbeat_at) > GATEWAY_RUNTIME_STALE_MS
}

fn persisted_gateway_owner_is_inactive(
    persisted_state: &PersistedGatewayOwnerState,
    now_ms: u64,
) -> bool {
    if !persisted_state.running {
        return true;
    }

    persisted_gateway_owner_is_stale(persisted_state, now_ms)
}

fn acquire_active_owner_file(
    path: &Path,
    persisted_state: &PersistedGatewayOwnerState,
) -> CliResult<fs::File> {
    ensure_parent_dir(path, "gateway runtime state directory")?;
    let encoded = serialize_json_pretty(persisted_state, "gateway owner state")?;

    let mut attempts = 0_u8;
    loop {
        attempts = attempts.saturating_add(1);
        let open_result = OpenOptions::new().write(true).create_new(true).open(path);
        let mut file = match open_result {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let now_ms = now_ms();
                let existing_state = read_persisted_gateway_owner_state(path);
                let existing_is_inactive = existing_state
                    .as_ref()
                    .map(|existing_state| {
                        persisted_gateway_owner_is_inactive(existing_state, now_ms)
                    })
                    .unwrap_or(false);
                if existing_is_inactive && attempts < 3 {
                    match fs::remove_file(path) {
                        Ok(()) => {}
                        Err(remove_error)
                            if remove_error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(remove_error) => {
                            return Err(format!(
                                "remove inactive gateway owner slot failed for {}: {remove_error}",
                                path.display()
                            ));
                        }
                    }
                    continue;
                }

                let existing_pid = existing_state
                    .as_ref()
                    .and_then(|existing_state| existing_state.pid)
                    .map(|process_id| process_id.to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                return Err(format!(
                    "gateway owner already active at {} (pid={existing_pid})",
                    path.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "create gateway owner slot failed for {}: {error}",
                    path.display()
                ));
            }
        };

        let write_result = file.write_all(encoded.as_bytes());
        if let Err(write_error) = write_result {
            let _ = fs::remove_file(path);
            return Err(format!(
                "write gateway owner slot failed for {}: {write_error}",
                path.display()
            ));
        }

        let sync_result = file.sync_all();
        if let Err(sync_error) = sync_result {
            let _ = fs::remove_file(path);
            return Err(format!(
                "sync gateway owner slot failed for {}: {sync_error}",
                path.display()
            ));
        }

        return Ok(file);
    }
}

fn write_active_owner_state(
    owner_file: &Mutex<fs::File>,
    persisted_state: &PersistedGatewayOwnerState,
) -> CliResult<()> {
    let encoded = serialize_json_pretty(persisted_state, "gateway owner state")?;
    let owner_file_guard = owner_file.lock();
    let mut owner_file_guard =
        owner_file_guard.map_err(|error| format!("gateway owner file lock poisoned: {error}"))?;
    owner_file_guard
        .set_len(0)
        .map_err(|error| format!("truncate gateway owner slot failed: {error}"))?;
    owner_file_guard
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek gateway owner slot failed: {error}"))?;
    owner_file_guard
        .write_all(encoded.as_bytes())
        .map_err(|error| format!("write gateway owner slot failed: {error}"))?;
    owner_file_guard
        .sync_all()
        .map_err(|error| format!("sync gateway owner slot failed: {error}"))
}

fn write_status_snapshot(
    path: &Path,
    persisted_state: &PersistedGatewayOwnerState,
) -> CliResult<()> {
    write_json_path(path, persisted_state, "gateway status snapshot")
}

fn write_json_path<T: Serialize>(path: &Path, value: &T, context: &str) -> CliResult<()> {
    ensure_parent_dir(path, context)?;
    let encoded = serialize_json_pretty(value, context)?;
    let open_result = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path);
    let mut file = open_result
        .map_err(|error| format!("open {context} failed for {}: {error}", path.display()))?;
    file.write_all(encoded.as_bytes())
        .map_err(|error| format!("write {context} failed for {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync {context} failed for {}: {error}", path.display()))
}

fn serialize_json_pretty<T: Serialize>(value: &T, context: &str) -> CliResult<String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize {context} failed: {error}"))
}

fn ensure_parent_dir(path: &Path, context: &str) -> CliResult<()> {
    let parent = path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent)
        .map_err(|error| format!("create {context} parent directory failed: {error}"))
}

fn remove_active_owner_if_owned(path: &Path, owner_token: &str) -> CliResult<()> {
    let current_owner_token =
        read_persisted_gateway_owner_state(path).map(|persisted_state| persisted_state.owner_token);
    if current_owner_token.as_deref() != Some(owner_token) {
        return Ok(());
    }

    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "remove gateway owner slot failed for {}: {error}",
            path.display()
        )),
    }
}

fn remove_stop_request_file(path: &Path) -> CliResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "remove gateway stop request failed for {}: {error}",
            path.display()
        )),
    }
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn runtime_owner_phase_text(phase: RuntimeOwnerPhase) -> &'static str {
    match phase {
        RuntimeOwnerPhase::Starting => "starting",
        RuntimeOwnerPhase::Running => "running",
        RuntimeOwnerPhase::Stopping => "stopping",
        RuntimeOwnerPhase::Stopped => "stopped",
        RuntimeOwnerPhase::Failed => "failed",
    }
}

fn new_gateway_owner_token(process_id: u32) -> String {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{process_id}-{now_nanos}")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_gateway_runtime_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let runtime_dir =
            std::env::temp_dir().join(format!("loongclaw-gateway-runtime-{label}-{suffix}"));
        fs::create_dir_all(&runtime_dir).expect("create gateway runtime dir");
        runtime_dir
    }

    fn sample_status(running: bool, last_heartbeat_at: u64) -> GatewayOwnerStatus {
        GatewayOwnerStatus {
            runtime_dir: "/tmp/loongclaw-gateway-runtime".to_owned(),
            phase: if running {
                "running".to_owned()
            } else {
                "stopped".to_owned()
            },
            running,
            stale: false,
            pid: Some(4242),
            mode: GatewayOwnerMode::GatewayHeadless,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            config_path: "/tmp/loongclaw.toml".to_owned(),
            attached_cli_session: None,
            started_at_ms: 1_710_000_000_000,
            last_heartbeat_at,
            stopped_at_ms: if running {
                None
            } else {
                Some(1_710_000_001_000)
            },
            shutdown_reason: None,
            last_error: None,
            configured_surface_count: 2,
            running_surface_count: if running { 2 } else { 0 },
            bind_address: None,
            port: None,
            token_path: None,
        }
    }

    #[test]
    fn gateway_owner_state_status_marks_running_snapshot_stale_after_heartbeat_budget() {
        let runtime_dir = temp_gateway_runtime_dir("stale-status");
        let stale_heartbeat_at = now_ms().saturating_sub(GATEWAY_RUNTIME_STALE_MS + 1);
        let status = sample_status(true, stale_heartbeat_at);
        write_gateway_owner_snapshot_for_test(runtime_dir.as_path(), &status)
            .expect("write gateway status");

        let loaded_status =
            load_gateway_owner_status(runtime_dir.as_path()).expect("load gateway status");

        assert!(loaded_status.stale);
        assert!(!loaded_status.running);
        assert_eq!(loaded_status.phase, "running");
    }

    #[tokio::test]
    async fn gateway_owner_state_acquire_and_finalize_preserve_last_stopped_snapshot() {
        let runtime_dir = temp_gateway_runtime_dir("finalize");
        let tracker = GatewayOwnerTracker::acquire(
            runtime_dir.as_path(),
            GatewayOwnerMode::GatewayHeadless,
            Path::new("/tmp/loongclaw.toml"),
            None,
            0,
        )
        .expect("acquire gateway owner");

        let spec = crate::supervisor::SupervisorSpec::new(
            crate::supervisor::RuntimeOwnerMode::GatewayHeadless,
            Vec::new(),
        )
        .expect("build supervisor spec");
        let mut supervisor = crate::supervisor::SupervisorState::new(spec);
        supervisor
            .request_shutdown("gateway stop requested".to_owned())
            .expect("request shutdown");
        supervisor.finalize_after_runtime_exit();

        tracker
            .finalize_from_supervisor(&supervisor)
            .expect("finalize owner state");

        let active_owner_path = active_gateway_owner_path(runtime_dir.as_path());
        assert!(!active_owner_path.exists());

        let loaded_status =
            load_gateway_owner_status(runtime_dir.as_path()).expect("load final gateway status");
        assert_eq!(loaded_status.phase, "stopped");
        assert!(!loaded_status.running);
        assert_eq!(
            loaded_status.shutdown_reason.as_deref(),
            Some("shutdown requested: gateway stop requested")
        );
    }

    #[tokio::test]
    async fn gateway_owner_state_reclaims_stale_owner_slot() {
        let runtime_dir = temp_gateway_runtime_dir("reclaim-stale");
        let active_owner_path = active_gateway_owner_path(runtime_dir.as_path());
        let stale_owner_state = PersistedGatewayOwnerState {
            phase: "running".to_owned(),
            running: true,
            pid: Some(7001),
            mode: GatewayOwnerMode::GatewayHeadless,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            config_path: "/tmp/stale.toml".to_owned(),
            attached_cli_session: None,
            started_at_ms: 1_710_000_000_000,
            last_heartbeat_at: now_ms().saturating_sub(GATEWAY_RUNTIME_STALE_MS + 1),
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            configured_surface_count: 1,
            running_surface_count: 1,
            bind_address: None,
            port: None,
            token_path: None,
            owner_token: "stale-owner".to_owned(),
        };
        write_json_path(
            active_owner_path.as_path(),
            &stale_owner_state,
            "stale gateway owner slot",
        )
        .expect("write stale owner slot");

        let tracker = GatewayOwnerTracker::acquire(
            runtime_dir.as_path(),
            GatewayOwnerMode::GatewayHeadless,
            Path::new("/tmp/fresh.toml"),
            None,
            0,
        )
        .expect("reclaim stale owner slot");
        drop(tracker);

        let active_owner = read_persisted_gateway_owner_state(active_owner_path.as_path())
            .expect("active owner should be replaced");
        assert_eq!(active_owner.config_path, "/tmp/fresh.toml");
    }

    #[test]
    fn gateway_owner_state_stop_request_returns_already_stopped_without_active_owner() {
        let runtime_dir = temp_gateway_runtime_dir("stop-idempotent");

        let outcome =
            request_gateway_stop(runtime_dir.as_path()).expect("request stop without active owner");

        assert_eq!(outcome, GatewayStopRequestOutcome::AlreadyStopped);
    }

    #[tokio::test]
    async fn gateway_owner_state_stop_request_writes_request_for_running_owner() {
        let runtime_dir = temp_gateway_runtime_dir("stop-request");
        let tracker = GatewayOwnerTracker::acquire(
            runtime_dir.as_path(),
            GatewayOwnerMode::GatewayHeadless,
            Path::new("/tmp/loongclaw.toml"),
            None,
            0,
        )
        .expect("acquire gateway owner");

        let outcome = request_gateway_stop(runtime_dir.as_path()).expect("request stop");

        assert_eq!(outcome, GatewayStopRequestOutcome::Requested);
        let stop_request_path = gateway_stop_request_path(runtime_dir.as_path());
        assert!(stop_request_path.exists());
        drop(tracker);
    }
}
