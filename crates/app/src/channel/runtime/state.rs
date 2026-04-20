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

use crate::{CliResult, config::default_loong_home};

use super::super::ChannelPlatform;

const CHANNEL_RUNTIME_HEARTBEAT_MS: u64 = 5_000;
const CHANNEL_RUNTIME_STALE_MS: u64 = 15_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelOperationRuntime {
    pub running: bool,
    pub stale: bool,
    pub busy: bool,
    pub active_runs: usize,
    pub last_run_activity_at: Option<u64>,
    pub last_heartbeat_at: Option<u64>,
    pub pid: Option<u32>,
    pub account_id: Option<String>,
    pub account_label: Option<String>,
    pub instance_count: usize,
    pub running_instances: usize,
    pub stale_instances: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct PersistedChannelOperationRuntime {
    running: bool,
    busy: bool,
    active_runs: usize,
    last_run_activity_at: Option<u64>,
    last_heartbeat_at: Option<u64>,
    pid: Option<u32>,
    account_id: Option<String>,
    account_label: Option<String>,
    owner_token: Option<String>,
}

impl PersistedChannelOperationRuntime {
    fn to_runtime_view(&self, now_ms: u64) -> ChannelOperationRuntime {
        let stale = self.running
            && self
                .last_heartbeat_at
                .map(|heartbeat| now_ms.saturating_sub(heartbeat) > CHANNEL_RUNTIME_STALE_MS)
                .unwrap_or(true);
        ChannelOperationRuntime {
            running: self.running && !stale,
            stale,
            busy: self.busy,
            active_runs: self.active_runs,
            last_run_activity_at: self.last_run_activity_at,
            last_heartbeat_at: self.last_heartbeat_at,
            pid: self.pid,
            account_id: self.account_id.clone(),
            account_label: self.account_label.clone(),
            instance_count: 0,
            running_instances: 0,
            stale_instances: 0,
        }
    }
}

pub struct ChannelOperationRuntimeTracker {
    path: PathBuf,
    state: Arc<Mutex<PersistedChannelOperationRuntime>>,
    stopped: Arc<AtomicBool>,
    heartbeat_task: Mutex<Option<JoinHandle<()>>>,
}

pub(crate) struct ChannelOperationExclusiveGuard {
    path: PathBuf,
    owner_token: String,
    stopped: Arc<AtomicBool>,
    heartbeat_task: Mutex<Option<JoinHandle<()>>>,
}

impl ChannelOperationRuntimeTracker {
    pub async fn start(
        platform: ChannelPlatform,
        operation_id: &'static str,
        account_id: &str,
        account_label: &str,
    ) -> CliResult<Self> {
        Self::start_in_dir_with_account_and_pid(
            &default_channel_runtime_state_dir(),
            platform,
            operation_id,
            account_id,
            account_label,
            CHANNEL_RUNTIME_HEARTBEAT_MS,
            std::process::id(),
        )
        .await
    }

    #[cfg(test)]
    async fn start_in_dir_with_pid(
        runtime_dir: &Path,
        platform: ChannelPlatform,
        operation_id: &'static str,
        heartbeat_ms: u64,
        process_id: u32,
    ) -> CliResult<Self> {
        Self::start_in_dir_impl(
            runtime_dir,
            platform,
            operation_id,
            None,
            None,
            heartbeat_ms,
            process_id,
        )
        .await
    }

    async fn start_in_dir_with_account_and_pid(
        runtime_dir: &Path,
        platform: ChannelPlatform,
        operation_id: &'static str,
        account_id: &str,
        account_label: &str,
        heartbeat_ms: u64,
        process_id: u32,
    ) -> CliResult<Self> {
        Self::start_in_dir_impl(
            runtime_dir,
            platform,
            operation_id,
            Some(account_id),
            Some(account_label),
            heartbeat_ms,
            process_id,
        )
        .await
    }

    async fn start_in_dir_impl(
        runtime_dir: &Path,
        platform: ChannelPlatform,
        operation_id: &'static str,
        account_id: Option<&str>,
        account_label: Option<&str>,
        heartbeat_ms: u64,
        process_id: u32,
    ) -> CliResult<Self> {
        let now = now_ms();
        prune_inactive_channel_operation_runtime_files_for_account_from_dir(
            runtime_dir,
            platform,
            operation_id,
            account_id,
            now,
        )?;
        let path = channel_operation_runtime_path(
            runtime_dir,
            platform,
            operation_id,
            account_id,
            Some(process_id),
        );
        let initial = PersistedChannelOperationRuntime {
            running: true,
            busy: false,
            active_runs: 0,
            last_run_activity_at: None,
            last_heartbeat_at: Some(now),
            pid: Some(process_id),
            account_id: normalize_optional_account_value(account_id),
            account_label: normalize_optional_account_value(account_label),
            owner_token: None,
        };
        write_runtime_state(&path, &initial)?;

        let state = Arc::new(Mutex::new(initial));
        let stopped = Arc::new(AtomicBool::new(false));
        let heartbeat_state = state.clone();
        let heartbeat_stopped = stopped.clone();
        let heartbeat_path = path.clone();
        let task = tokio::spawn(async move {
            while !heartbeat_stopped.load(Ordering::SeqCst) {
                sleep(Duration::from_millis(heartbeat_ms)).await;
                if heartbeat_stopped.load(Ordering::SeqCst) {
                    break;
                }
                let snapshot = {
                    let Ok(mut state) = heartbeat_state.lock() else {
                        break;
                    };
                    state.last_heartbeat_at = Some(now_ms());
                    state.clone()
                };
                let _ = write_runtime_state(&heartbeat_path, &snapshot);
            }
        });

        Ok(Self {
            path,
            state,
            stopped,
            heartbeat_task: Mutex::new(Some(task)),
        })
    }

    pub async fn mark_run_start(&self) -> CliResult<()> {
        self.update_state(|state| {
            state.active_runs = state.active_runs.saturating_add(1);
            state.busy = true;
            let now = now_ms();
            state.last_run_activity_at = Some(now);
            state.last_heartbeat_at = Some(now);
        })
        .await
    }

    pub async fn mark_run_end(&self) -> CliResult<()> {
        self.update_state(|state| {
            state.active_runs = state.active_runs.saturating_sub(1);
            state.busy = state.active_runs > 0;
            let now = now_ms();
            state.last_run_activity_at = Some(now);
            state.last_heartbeat_at = Some(now);
        })
        .await
    }

    pub async fn shutdown(&self) -> CliResult<()> {
        self.stopped.store(true, Ordering::SeqCst);
        let task = self
            .heartbeat_task
            .lock()
            .map_err(|error| format!("channel runtime heartbeat task lock poisoned: {error}"))?
            .take();
        if let Some(task) = task {
            task.abort();
        }
        self.update_state(|state| {
            state.running = false;
            state.busy = false;
            state.active_runs = 0;
            state.last_heartbeat_at = Some(now_ms());
        })
        .await
    }

    async fn update_state(
        &self,
        mutate: impl FnOnce(&mut PersistedChannelOperationRuntime),
    ) -> CliResult<()> {
        let snapshot = {
            let mut state = self
                .state
                .lock()
                .map_err(|error| format!("channel runtime state lock poisoned: {error}"))?;
            mutate(&mut state);
            state.clone()
        };
        write_runtime_state(&self.path, &snapshot)
    }
}

impl ChannelOperationExclusiveGuard {
    pub(crate) async fn acquire(
        platform: ChannelPlatform,
        operation_id: &'static str,
        account_id: &str,
        account_label: &str,
    ) -> CliResult<Self> {
        Self::acquire_in_dir_impl(
            &default_channel_runtime_state_dir(),
            platform,
            operation_id,
            account_id,
            account_label,
            CHANNEL_RUNTIME_HEARTBEAT_MS,
            std::process::id(),
        )
        .await
    }

    async fn acquire_in_dir_impl(
        runtime_dir: &Path,
        platform: ChannelPlatform,
        operation_id: &'static str,
        account_id: &str,
        account_label: &str,
        heartbeat_ms: u64,
        process_id: u32,
    ) -> CliResult<Self> {
        let path = channel_operation_runtime_path(
            runtime_dir,
            platform,
            operation_id,
            Some(account_id),
            None,
        );
        let now = now_ms();
        let owner_token = new_runtime_owner_token(process_id);
        let initial = PersistedChannelOperationRuntime {
            running: true,
            busy: true,
            active_runs: 1,
            last_run_activity_at: Some(now),
            last_heartbeat_at: Some(now),
            pid: Some(process_id),
            account_id: normalize_optional_account_value(Some(account_id)),
            account_label: normalize_optional_account_value(Some(account_label)),
            owner_token: Some(owner_token.clone()),
        };
        let mut heartbeat_file = acquire_exclusive_runtime_state(path.as_path(), &initial)?;

        let heartbeat_state = Arc::new(Mutex::new(initial));
        let stopped = Arc::new(AtomicBool::new(false));
        let heartbeat_stopped = stopped.clone();
        let task = tokio::spawn(async move {
            while !heartbeat_stopped.load(Ordering::SeqCst) {
                sleep(Duration::from_millis(heartbeat_ms)).await;
                if heartbeat_stopped.load(Ordering::SeqCst) {
                    break;
                }
                let snapshot = {
                    let Ok(mut state) = heartbeat_state.lock() else {
                        break;
                    };
                    let heartbeat_now = now_ms();
                    state.last_run_activity_at = Some(heartbeat_now);
                    state.last_heartbeat_at = Some(heartbeat_now);
                    state.clone()
                };
                let write_result = write_runtime_state_to_file(&mut heartbeat_file, &snapshot);
                if write_result.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            path,
            owner_token,
            stopped,
            heartbeat_task: Mutex::new(Some(task)),
        })
    }

    #[cfg(test)]
    async fn acquire_in_dir_with_account_and_pid(
        runtime_dir: &Path,
        platform: ChannelPlatform,
        operation_id: &'static str,
        account_id: &str,
        account_label: &str,
        heartbeat_ms: u64,
        process_id: u32,
    ) -> CliResult<Self> {
        Self::acquire_in_dir_impl(
            runtime_dir,
            platform,
            operation_id,
            account_id,
            account_label,
            heartbeat_ms,
            process_id,
        )
        .await
    }
}

impl Drop for ChannelOperationExclusiveGuard {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Ok(mut task) = self.heartbeat_task.lock() {
            let task = task.take();
            if let Some(task) = task {
                task.abort();
            }
        }
        let _ =
            remove_exclusive_runtime_state_if_owned(self.path.as_path(), self.owner_token.as_str());
    }
}

#[cfg(test)]
pub(crate) async fn start_channel_operation_runtime_tracker_for_test(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &'static str,
    account_id: &str,
    account_label: &str,
    process_id: u32,
) -> CliResult<ChannelOperationRuntimeTracker> {
    ChannelOperationRuntimeTracker::start_in_dir_with_account_and_pid(
        runtime_dir,
        platform,
        operation_id,
        account_id,
        account_label,
        CHANNEL_RUNTIME_HEARTBEAT_MS,
        process_id,
    )
    .await
}

#[cfg(test)]
pub(crate) fn load_channel_operation_runtime_from_dir(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    now_ms: u64,
) -> Option<ChannelOperationRuntime> {
    load_channel_operation_runtime_for_optional_account_from_dir(
        runtime_dir,
        platform,
        operation_id,
        None,
        now_ms,
    )
}

pub(crate) fn load_channel_operation_runtime_for_account_from_dir(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: &str,
    now_ms: u64,
) -> Option<ChannelOperationRuntime> {
    load_channel_operation_runtime_for_optional_account_from_dir(
        runtime_dir,
        platform,
        operation_id,
        Some(account_id),
        now_ms,
    )
}

fn load_channel_operation_runtime_for_optional_account_from_dir(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: Option<&str>,
    now_ms: u64,
) -> Option<ChannelOperationRuntime> {
    let prefix = channel_operation_runtime_file_prefix(platform, operation_id, account_id);
    let mut candidates = Vec::new();

    if let Ok(entries) = fs::read_dir(runtime_dir) {
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if !matches_channel_operation_runtime_file(file_name.as_ref(), &prefix) {
                continue;
            }
            if let Some(runtime) = read_runtime_state(entry.path().as_path(), now_ms) {
                candidates.push(runtime);
            }
        }
    }

    if candidates.is_empty() && account_id.is_some() {
        return load_channel_operation_runtime_for_optional_account_from_dir(
            runtime_dir,
            platform,
            operation_id,
            None,
            now_ms,
        );
    }

    if candidates.is_empty() {
        let legacy_path =
            channel_operation_runtime_path(runtime_dir, platform, operation_id, None, None);
        if let Some(runtime) = read_runtime_state(&legacy_path, now_ms) {
            candidates.push(runtime);
        }
    }

    summarize_runtime_candidates(candidates)
}

pub(crate) fn prune_inactive_channel_operation_runtime_files_for_account_from_dir(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: Option<&str>,
    now_ms: u64,
) -> CliResult<()> {
    prune_inactive_channel_operation_runtime_files_for_optional_account_from_dir(
        runtime_dir,
        platform,
        operation_id,
        account_id,
        now_ms,
    )?;
    if account_id.is_some() {
        prune_inactive_channel_operation_runtime_files_for_optional_account_from_dir(
            runtime_dir,
            platform,
            operation_id,
            None,
            now_ms,
        )?;
    }
    Ok(())
}

fn prune_inactive_channel_operation_runtime_files_for_optional_account_from_dir(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: Option<&str>,
    now_ms: u64,
) -> CliResult<()> {
    let prefix = channel_operation_runtime_file_prefix(platform, operation_id, account_id);
    let entries = match fs::read_dir(runtime_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "read channel runtime state directory failed for {}: {error}",
                runtime_dir.display()
            ));
        }
    };

    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !matches_channel_operation_runtime_file(file_name.as_ref(), &prefix) {
            continue;
        }
        let path = entry.path();
        if !runtime_state_path_is_inactive(path.as_path(), now_ms) {
            continue;
        }
        match fs::remove_file(path.as_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // On Windows, another process/task may hold the file open, causing
            // ERROR_ACCESS_DENIED (PermissionDenied). Unlike Unix, Windows does not
            // allow unlinking an open file. Silently skip; the next prune cycle will retry.
            #[cfg(windows)]
            Err(ref error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
            Err(error) => {
                return Err(format!(
                    "remove inactive channel runtime state failed for {}: {error}",
                    path.display()
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn default_channel_runtime_state_dir() -> PathBuf {
    default_loong_home().join("channel-runtime")
}

fn channel_operation_runtime_file_prefix(
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: Option<&str>,
) -> String {
    match normalize_optional_account_value(account_id) {
        Some(account_id) => format!("{}-{operation_id}-{account_id}", platform.as_str()),
        None => format!("{}-{operation_id}", platform.as_str()),
    }
}

fn channel_operation_runtime_path(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: Option<&str>,
    process_id: Option<u32>,
) -> PathBuf {
    let prefix = channel_operation_runtime_file_prefix(platform, operation_id, account_id);
    match process_id {
        Some(process_id) => runtime_dir.join(format!("{prefix}-{process_id}.json")),
        None => runtime_dir.join(format!("{prefix}.json")),
    }
}

fn matches_channel_operation_runtime_file(file_name: &str, prefix: &str) -> bool {
    if file_name == format!("{prefix}.json") {
        return true;
    }

    file_name
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('-'))
        .and_then(|suffix| suffix.strip_suffix(".json"))
        .map(|pid| !pid.is_empty() && pid.chars().all(|value| value.is_ascii_digit()))
        .unwrap_or(false)
}

fn read_runtime_state(path: &Path, now_ms: u64) -> Option<ChannelOperationRuntime> {
    let state = read_persisted_runtime_state(path)?;
    Some(state.to_runtime_view(now_ms))
}

fn read_persisted_runtime_state(path: &Path) -> Option<PersistedChannelOperationRuntime> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<PersistedChannelOperationRuntime>(&raw).ok()
}

fn runtime_state_path_is_inactive(path: &Path, now_ms: u64) -> bool {
    read_runtime_state(path, now_ms)
        .map(|runtime| !runtime.running)
        .unwrap_or(false)
}

fn select_preferred_runtime(
    candidates: Vec<ChannelOperationRuntime>,
) -> Option<ChannelOperationRuntime> {
    candidates.into_iter().max_by_key(|runtime| {
        (
            runtime.running,
            !runtime.stale,
            runtime.last_heartbeat_at.unwrap_or(0),
            runtime.last_run_activity_at.unwrap_or(0),
            runtime.pid.unwrap_or(0),
        )
    })
}

fn summarize_runtime_candidates(
    candidates: Vec<ChannelOperationRuntime>,
) -> Option<ChannelOperationRuntime> {
    if candidates.is_empty() {
        return None;
    }

    let instance_count = candidates.len();
    let running_instances = candidates.iter().filter(|runtime| runtime.running).count();
    let stale_instances = candidates.iter().filter(|runtime| runtime.stale).count();
    let mut preferred = select_preferred_runtime(candidates)?;
    preferred.instance_count = instance_count;
    preferred.running_instances = running_instances;
    preferred.stale_instances = stale_instances;
    Some(preferred)
}

fn normalize_optional_account_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn acquire_exclusive_runtime_state(
    path: &Path,
    state: &PersistedChannelOperationRuntime,
) -> CliResult<fs::File> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create channel runtime state directory failed: {error}"))?;
    }

    let encoded = serde_json::to_string_pretty(state)
        .map_err(|error| format!("serialize channel runtime owner state failed: {error}"))?;

    let mut attempts = 0_u8;
    loop {
        attempts = attempts.saturating_add(1);
        let open_result = OpenOptions::new().write(true).create_new(true).open(path);
        let mut file = match open_result {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let now = now_ms();
                let existing = read_runtime_state(path, now);
                let is_inactive = existing
                    .as_ref()
                    .map(|runtime| !runtime.running)
                    .unwrap_or(false);
                if is_inactive && attempts < 3 {
                    match fs::remove_file(path) {
                        Ok(()) => {}
                        Err(remove_error)
                            if remove_error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(remove_error) => {
                            let display_path = path.display();
                            return Err(format!(
                                "remove inactive exclusive channel runtime owner failed for {display_path}: {remove_error}"
                            ));
                        }
                    }
                    continue;
                }

                let existing_pid = existing
                    .as_ref()
                    .and_then(|runtime| runtime.pid)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                let display_path = path.display();
                return Err(format!(
                    "exclusive channel runtime owner already active at {display_path} (pid={existing_pid})"
                ));
            }
            Err(error) => {
                let display_path = path.display();
                return Err(format!(
                    "create exclusive channel runtime owner failed for {display_path}: {error}"
                ));
            }
        };

        let write_result = file.write_all(encoded.as_bytes());
        if let Err(error) = write_result {
            _ = fs::remove_file(path);
            let display_path = path.display();
            return Err(format!(
                "write exclusive channel runtime owner failed for {display_path}: {error}"
            ));
        }

        let sync_result = file.sync_all();
        if let Err(error) = sync_result {
            _ = fs::remove_file(path);
            let display_path = path.display();
            return Err(format!(
                "sync exclusive channel runtime owner failed for {display_path}: {error}"
            ));
        }

        return Ok(file);
    }
}

fn write_runtime_state_to_file(
    file: &mut fs::File,
    state: &PersistedChannelOperationRuntime,
) -> CliResult<()> {
    let encoded = serde_json::to_string_pretty(state)
        .map_err(|error| format!("serialize channel runtime owner state failed: {error}"))?;
    file.set_len(0)
        .map_err(|error| format!("truncate channel runtime owner file failed: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek channel runtime owner file failed: {error}"))?;
    file.write_all(encoded.as_bytes())
        .map_err(|error| format!("write channel runtime owner file failed: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync channel runtime owner file failed: {error}"))
}

fn remove_exclusive_runtime_state_if_owned(path: &Path, owner_token: &str) -> CliResult<()> {
    let current_owner_token =
        read_persisted_runtime_state(path).and_then(|state| state.owner_token);
    if current_owner_token.as_deref() != Some(owner_token) {
        return Ok(());
    }

    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "remove exclusive channel runtime owner failed for {}: {error}",
            path.display()
        )),
    }
}

fn write_runtime_state(path: &Path, state: &PersistedChannelOperationRuntime) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create channel runtime state directory failed: {error}"))?;
    }
    let encoded = serde_json::to_string_pretty(state)
        .map_err(|error| format!("serialize channel runtime state failed: {error}"))?;
    fs::write(path, encoded).map_err(|error| format!("write channel runtime state failed: {error}"))
}

fn new_runtime_owner_token(process_id: u32) -> String {
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
pub(crate) fn write_runtime_state_for_test(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    running: bool,
    busy: bool,
    active_runs: usize,
    last_run_activity_at: Option<u64>,
    last_heartbeat_at: Option<u64>,
    pid: Option<u32>,
) -> CliResult<()> {
    let path = channel_operation_runtime_path(runtime_dir, platform, operation_id, None, None);
    let state = PersistedChannelOperationRuntime {
        running,
        busy,
        active_runs,
        last_run_activity_at,
        last_heartbeat_at,
        pid,
        account_id: None,
        account_label: None,
        owner_token: None,
    };
    write_runtime_state(&path, &state)
}

#[cfg(test)]
pub(crate) fn write_runtime_state_for_test_with_pid(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    process_id: u32,
    running: bool,
    busy: bool,
    active_runs: usize,
    last_run_activity_at: Option<u64>,
    last_heartbeat_at: Option<u64>,
    pid: Option<u32>,
) -> CliResult<()> {
    let path =
        channel_operation_runtime_path(runtime_dir, platform, operation_id, None, Some(process_id));
    let state = PersistedChannelOperationRuntime {
        running,
        busy,
        active_runs,
        last_run_activity_at,
        last_heartbeat_at,
        pid,
        account_id: None,
        account_label: None,
        owner_token: None,
    };
    write_runtime_state(&path, &state)
}

#[cfg(test)]
pub(crate) fn write_runtime_state_for_test_with_account_and_pid(
    runtime_dir: &Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: &str,
    process_id: u32,
    running: bool,
    busy: bool,
    active_runs: usize,
    last_run_activity_at: Option<u64>,
    last_heartbeat_at: Option<u64>,
    pid: Option<u32>,
) -> CliResult<()> {
    let path = channel_operation_runtime_path(
        runtime_dir,
        platform,
        operation_id,
        Some(account_id),
        Some(process_id),
    );
    let state = PersistedChannelOperationRuntime {
        running,
        busy,
        active_runs,
        last_run_activity_at,
        last_heartbeat_at,
        pid,
        account_id: Some(account_id.to_owned()),
        account_label: Some(test_account_label(account_id)),
        owner_token: None,
    };
    write_runtime_state(&path, &state)
}

#[cfg(test)]
fn test_account_label(account_id: &str) -> String {
    match account_id.split_once('_') {
        Some((platform, rest)) if !platform.is_empty() && !rest.is_empty() => {
            format!("{platform}:{rest}")
        }
        _ => account_id.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::CHANNEL_OPERATION_SERVE_ID;
    const TEST_OWNER_OPERATION_ID: &str = "owner";

    fn temp_runtime_dir(suffix: &str) -> PathBuf {
        let unique = format!(
            "loong-channel-runtime-{suffix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[tokio::test]
    async fn runtime_tracker_persists_run_activity_and_shutdown_state() {
        let runtime_dir = temp_runtime_dir("tracker");
        let tracker = ChannelOperationRuntimeTracker::start_in_dir_with_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            20,
            4242,
        )
        .await
        .expect("start runtime tracker");

        tracker.mark_run_start().await.expect("mark run start");
        tracker.mark_run_end().await.expect("mark run end");
        sleep(Duration::from_millis(30)).await;
        tracker.shutdown().await.expect("shutdown tracker");

        let runtime = load_channel_operation_runtime_from_dir(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            now_ms(),
        )
        .expect("load runtime state");

        assert!(!runtime.running);
        assert!(!runtime.busy);
        assert_eq!(runtime.active_runs, 0);
        assert!(runtime.last_run_activity_at.is_some());
        assert!(runtime.last_heartbeat_at.is_some());
        assert_eq!(runtime.pid, Some(4242));
        let entries = fs::read_dir(&runtime_dir)
            .expect("list runtime dir")
            .map(|entry| {
                entry
                    .expect("runtime entry")
                    .file_name()
                    .into_string()
                    .expect("utf-8 file name")
            })
            .collect::<Vec<_>>();
        let expected_entry = channel_operation_runtime_path(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            None,
            Some(4242),
        )
        .file_name()
        .expect("runtime path file name")
        .to_string_lossy()
        .into_owned();
        assert!(entries.contains(&expected_entry));
    }

    #[tokio::test]
    async fn runtime_tracker_prunes_inactive_pid_files_before_restart() {
        let runtime_dir = temp_runtime_dir("tracker-restart-cleanup");
        let first = ChannelOperationRuntimeTracker::start_in_dir_with_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            20,
            4242,
        )
        .await
        .expect("start first runtime tracker");
        first
            .shutdown()
            .await
            .expect("shutdown first runtime tracker");

        let second = ChannelOperationRuntimeTracker::start_in_dir_with_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            20,
            5252,
        )
        .await
        .expect("restart runtime tracker");

        let runtime = load_channel_operation_runtime_from_dir(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            now_ms(),
        )
        .expect("load restarted runtime state");

        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(5252));
        assert_eq!(runtime.instance_count, 1);
        assert_eq!(runtime.running_instances, 1);
        assert_eq!(runtime.stale_instances, 0);

        let entries = fs::read_dir(&runtime_dir)
            .expect("list runtime dir after restart")
            .map(|entry| {
                entry
                    .expect("runtime entry after restart")
                    .file_name()
                    .into_string()
                    .expect("utf-8 file name after restart")
            })
            .collect::<Vec<_>>();
        let expected_entry = channel_operation_runtime_path(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            None,
            Some(5252),
        )
        .file_name()
        .expect("restarted runtime path file name")
        .to_string_lossy()
        .into_owned();
        assert_eq!(entries, vec![expected_entry]);

        second
            .shutdown()
            .await
            .expect("shutdown restarted runtime tracker");
    }

    #[tokio::test]
    async fn account_runtime_tracker_prunes_inactive_legacy_file_before_start() {
        let runtime_dir = temp_runtime_dir("account-tracker-legacy-cleanup");
        let now = now_ms();
        write_runtime_state_for_test(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            false,
            false,
            0,
            Some(now.saturating_sub(5_000)),
            Some(now.saturating_sub(5_000)),
            Some(4141),
        )
        .expect("write inactive legacy runtime state");

        let tracker = ChannelOperationRuntimeTracker::start_in_dir_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            &test_account_label("bot_123456"),
            20,
            5252,
        )
        .await
        .expect("start account-scoped runtime tracker");

        let runtime = load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            now_ms(),
        )
        .expect("load account-scoped runtime state");

        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(5252));
        assert_eq!(runtime.instance_count, 1);
        assert_eq!(runtime.running_instances, 1);
        assert_eq!(runtime.stale_instances, 0);

        let entries = fs::read_dir(&runtime_dir)
            .expect("list runtime dir after account startup")
            .map(|entry| {
                entry
                    .expect("runtime entry after account startup")
                    .file_name()
                    .into_string()
                    .expect("utf-8 file name after account startup")
            })
            .collect::<Vec<_>>();
        let expected_entry = channel_operation_runtime_path(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            Some("bot_123456"),
            Some(5252),
        )
        .file_name()
        .expect("account runtime path file name")
        .to_string_lossy()
        .into_owned();
        assert_eq!(entries, vec![expected_entry]);

        tracker
            .shutdown()
            .await
            .expect("shutdown account-scoped runtime tracker");
    }

    #[test]
    fn stale_runtime_is_marked_not_running() {
        let runtime_dir = temp_runtime_dir("stale");
        let now = now_ms();
        write_runtime_state_for_test(
            &runtime_dir,
            ChannelPlatform::Feishu,
            CHANNEL_OPERATION_SERVE_ID,
            true,
            true,
            1,
            Some(now.saturating_sub(30_000)),
            Some(now.saturating_sub(30_000)),
            Some(99),
        )
        .expect("write stale runtime state");

        let runtime = load_channel_operation_runtime_from_dir(
            &runtime_dir,
            ChannelPlatform::Feishu,
            CHANNEL_OPERATION_SERVE_ID,
            now,
        )
        .expect("load runtime state");

        assert!(!runtime.running);
        assert!(runtime.stale);
        assert!(runtime.busy);
        assert_eq!(runtime.active_runs, 1);
    }

    #[test]
    fn load_runtime_prefers_running_pid_scoped_state_over_newer_stopped_instance() {
        let runtime_dir = temp_runtime_dir("pid-scoped-selection");
        let now = now_ms();
        write_runtime_state_for_test_with_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            1001,
            true,
            true,
            2,
            Some(now.saturating_sub(2_000)),
            Some(now.saturating_sub(1_000)),
            Some(1001),
        )
        .expect("write running pid-scoped runtime");
        write_runtime_state_for_test_with_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            2002,
            false,
            false,
            0,
            Some(now.saturating_sub(100)),
            Some(now.saturating_sub(100)),
            Some(2002),
        )
        .expect("write stopped pid-scoped runtime");

        let runtime = load_channel_operation_runtime_from_dir(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            now,
        )
        .expect("load runtime state");

        assert!(runtime.running);
        assert!(!runtime.stale);
        assert!(runtime.busy);
        assert_eq!(runtime.active_runs, 2);
        assert_eq!(runtime.pid, Some(1001));
    }

    #[test]
    fn load_runtime_keeps_backward_compatibility_with_legacy_single_file() {
        let runtime_dir = temp_runtime_dir("legacy-runtime");
        let now = now_ms();
        write_runtime_state_for_test(
            &runtime_dir,
            ChannelPlatform::Feishu,
            CHANNEL_OPERATION_SERVE_ID,
            true,
            true,
            1,
            Some(now.saturating_sub(500)),
            Some(now.saturating_sub(200)),
            Some(9090),
        )
        .expect("write legacy runtime state");

        let runtime = load_channel_operation_runtime_from_dir(
            &runtime_dir,
            ChannelPlatform::Feishu,
            CHANNEL_OPERATION_SERVE_ID,
            now,
        )
        .expect("load runtime state");

        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(9090));
    }

    #[test]
    fn load_runtime_reads_account_scoped_pid_file() {
        let runtime_dir = temp_runtime_dir("account-runtime");
        let now = now_ms();
        write_runtime_state_for_test_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            3003,
            true,
            true,
            1,
            Some(now.saturating_sub(250)),
            Some(now.saturating_sub(100)),
            Some(3003),
        )
        .expect("write account-scoped runtime state");

        let runtime = load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            now,
        )
        .expect("load account runtime state");

        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(3003));
        assert_eq!(runtime.account_id.as_deref(), Some("bot_123456"));
    }

    #[test]
    fn account_scoped_runtime_loader_falls_back_to_legacy_operation_files() {
        let runtime_dir = temp_runtime_dir("account-runtime-legacy");
        let now = now_ms();
        write_runtime_state_for_test(
            &runtime_dir,
            ChannelPlatform::Feishu,
            CHANNEL_OPERATION_SERVE_ID,
            true,
            false,
            0,
            Some(now.saturating_sub(250)),
            Some(now.saturating_sub(100)),
            Some(8181),
        )
        .expect("write legacy runtime state");

        let runtime = load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            ChannelPlatform::Feishu,
            CHANNEL_OPERATION_SERVE_ID,
            "lark_cli_a1b2c3",
            now,
        )
        .expect("load account runtime state via legacy fallback");

        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(8181));
    }

    #[test]
    fn account_scoped_runtime_loader_reports_duplicate_running_instances() {
        let runtime_dir = temp_runtime_dir("account-runtime-duplicates");
        let now = now_ms();
        write_runtime_state_for_test_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            3003,
            true,
            true,
            1,
            Some(now.saturating_sub(300)),
            Some(now.saturating_sub(100)),
            Some(3003),
        )
        .expect("write first running runtime state");
        write_runtime_state_for_test_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            4004,
            true,
            false,
            0,
            Some(now.saturating_sub(200)),
            Some(now.saturating_sub(50)),
            Some(4004),
        )
        .expect("write second running runtime state");

        let runtime = load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            ChannelPlatform::Telegram,
            CHANNEL_OPERATION_SERVE_ID,
            "bot_123456",
            now,
        )
        .expect("load account runtime state");

        assert_eq!(runtime.instance_count, 2);
        assert_eq!(runtime.running_instances, 2);
        assert_eq!(runtime.stale_instances, 0);
        assert_eq!(runtime.pid, Some(4004));
    }

    #[tokio::test]
    async fn exclusive_guard_blocks_second_live_owner() {
        let runtime_dir = temp_runtime_dir("exclusive-owner-conflict");
        let first = ChannelOperationExclusiveGuard::acquire_in_dir_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            &test_account_label("wecom_ops"),
            20,
            7001,
        )
        .await
        .expect("acquire first owner guard");

        let acquire_result = ChannelOperationExclusiveGuard::acquire_in_dir_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            &test_account_label("wecom_ops"),
            20,
            7002,
        )
        .await;
        let error = match acquire_result {
            Ok(_guard) => panic!("second owner guard should be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("exclusive channel runtime owner already active"));
        assert!(error.contains("pid=7001"));

        let runtime = load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            now_ms(),
        )
        .expect("load first owner runtime");
        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(7001));

        drop(first);
    }

    #[tokio::test]
    async fn exclusive_guard_reclaims_stale_owner_file() {
        let runtime_dir = temp_runtime_dir("exclusive-owner-stale");
        let stale_now = now_ms();
        let stale_path = channel_operation_runtime_path(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            Some("wecom_ops"),
            None,
        );
        let stale_state = PersistedChannelOperationRuntime {
            running: true,
            busy: true,
            active_runs: 1,
            last_run_activity_at: Some(stale_now.saturating_sub(30_000)),
            last_heartbeat_at: Some(stale_now.saturating_sub(30_000)),
            pid: Some(8001),
            account_id: Some("wecom_ops".to_owned()),
            account_label: Some(test_account_label("wecom_ops")),
            owner_token: Some("stale-owner".to_owned()),
        };
        write_runtime_state(stale_path.as_path(), &stale_state)
            .expect("write stale exclusive owner");

        let guard = ChannelOperationExclusiveGuard::acquire_in_dir_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            &test_account_label("wecom_ops"),
            20,
            8002,
        )
        .await
        .expect("acquire owner guard after stale cleanup");

        let runtime = load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            now_ms(),
        )
        .expect("load reclaimed owner");
        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(8002));

        drop(guard);
    }

    #[tokio::test]
    async fn exclusive_guard_drop_keeps_reclaimed_owner_file() {
        let runtime_dir = temp_runtime_dir("exclusive-owner-reclaimed-drop");
        let first = ChannelOperationExclusiveGuard::acquire_in_dir_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            &test_account_label("wecom_ops"),
            60_000,
            8101,
        )
        .await
        .expect("acquire first owner guard");

        let owner_path = channel_operation_runtime_path(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            Some("wecom_ops"),
            None,
        );
        let mut stale_state =
            read_persisted_runtime_state(owner_path.as_path()).expect("read first owner state");
        let stale_now = now_ms();
        stale_state.last_run_activity_at = Some(stale_now.saturating_sub(30_000));
        stale_state.last_heartbeat_at = Some(stale_now.saturating_sub(30_000));
        write_runtime_state(owner_path.as_path(), &stale_state)
            .expect("rewrite first owner state as stale");

        let second = ChannelOperationExclusiveGuard::acquire_in_dir_with_account_and_pid(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            &test_account_label("wecom_ops"),
            60_000,
            8102,
        )
        .await
        .expect("acquire second owner guard");

        drop(first);

        let runtime = load_channel_operation_runtime_for_account_from_dir(
            &runtime_dir,
            ChannelPlatform::Wecom,
            TEST_OWNER_OPERATION_ID,
            "wecom_ops",
            now_ms(),
        )
        .expect("load reclaimed owner after first guard drop");
        assert!(runtime.running);
        assert_eq!(runtime.pid, Some(8102));

        drop(second);
    }
}
