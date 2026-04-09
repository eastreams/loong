use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

use crate::CliResult;
use crate::mvp;

const DETACHED_DELEGATE_CHILD_COMMAND: &str = "delegate-child-run";
const DETACHED_DELEGATE_CHILD_CONFIG_ARG: &str = "--config-path";
const DETACHED_DELEGATE_CHILD_PAYLOAD_ARG: &str = "--payload-file";
const DETACHED_DELEGATE_CHILD_EXECUTABLE_ENV: &str = "CARGO_BIN_EXE_loong";
const DETACHED_DELEGATE_CHILD_KERNEL_SCOPE: &str = "delegate-child-worker";
const DETACHED_DELEGATE_CHILD_PASSTHROUGH_ENV_KEYS: &[&str] =
    &["LOONGCLAW_CONFIG_PATH", "LOONG_HOME", "LOONGCLAW_HOME"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DetachedDelegateChildBinding {
    Kernel,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DetachedDelegateChildPayload {
    child_session_id: String,
    parent_session_id: String,
    task: String,
    label: Option<String>,
    profile: Option<mvp::conversation::DelegateBuiltinProfile>,
    execution: mvp::conversation::ConstrainedSubagentExecution,
    runtime_self_continuity: Option<serde_json::Value>,
    timeout_seconds: u64,
    binding: DetachedDelegateChildBinding,
}

impl DetachedDelegateChildPayload {
    fn from_request(request: &mvp::conversation::AsyncDelegateSpawnRequest) -> Self {
        let binding = if request.binding.is_kernel_bound() {
            DetachedDelegateChildBinding::Kernel
        } else {
            DetachedDelegateChildBinding::Direct
        };

        Self {
            child_session_id: request.child_session_id.clone(),
            parent_session_id: request.parent_session_id.clone(),
            task: request.task.clone(),
            label: request.label.clone(),
            profile: request.profile,
            execution: request.execution.clone(),
            runtime_self_continuity: request
                .runtime_self_continuity_json()
                .expect("delegate payload serialization should succeed"),
            timeout_seconds: request.timeout_seconds,
            binding,
        }
    }

    fn into_spawn_request(
        self,
        binding: mvp::conversation::OwnedConversationRuntimeBinding,
    ) -> CliResult<mvp::conversation::AsyncDelegateSpawnRequest> {
        mvp::conversation::async_delegate_spawn_request_from_serialized_parts(
            self.child_session_id,
            self.parent_session_id,
            self.task,
            self.label,
            self.profile,
            self.execution,
            self.runtime_self_continuity,
            self.timeout_seconds,
            binding,
        )
    }
}

pub(crate) fn spawn_detached_delegate_child_process(
    request: &mvp::conversation::AsyncDelegateSpawnRequest,
) -> CliResult<()> {
    let executable_path = resolve_detached_delegate_child_executable_path()?;
    let config_path = resolve_detached_delegate_child_config_path()?;
    let payload = DetachedDelegateChildPayload::from_request(request);
    let payload_path = materialize_detached_delegate_child_payload_file(&payload)?;

    let mut command = std::process::Command::new(&executable_path);
    command.arg(DETACHED_DELEGATE_CHILD_COMMAND);
    command.arg(DETACHED_DELEGATE_CHILD_CONFIG_ARG);
    command.arg(config_path.as_os_str());
    command.arg(DETACHED_DELEGATE_CHILD_PAYLOAD_ARG);
    command.arg(payload_path.as_os_str());
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::piped());
    propagate_detached_delegate_child_environment(&mut command);

    let spawn_result = command.spawn();

    match spawn_result {
        Ok(mut child) => {
            let startup_timeout = std::time::Duration::from_millis(500);
            let startup_status = child.wait_timeout(startup_timeout).map_err(|error| {
                format!("wait for detached delegate child startup failed: {error}")
            })?;

            if let Some(exit_status) = startup_status {
                let stderr = read_detached_delegate_child_stderr(&mut child);
                remove_detached_delegate_child_payload_file(payload_path.as_path());
                let startup_failure =
                    detached_delegate_child_startup_failure(&exit_status, stderr.as_str());

                if let Some(startup_failure) = startup_failure {
                    return Err(startup_failure);
                }

                return Ok(());
            }

            Ok(())
        }
        Err(error) => {
            remove_detached_delegate_child_payload_file(payload_path.as_path());
            Err(format!(
                "delegate_async_process_spawn_failed: could not launch detached delegate child via `{}`: {error}",
                executable_path.display()
            ))
        }
    }
}

fn read_detached_delegate_child_stderr(child: &mut std::process::Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };

    let mut buffer = String::new();
    let _ = stderr.read_to_string(&mut buffer);

    buffer
}

fn detached_delegate_child_startup_failure(
    exit_status: &std::process::ExitStatus,
    stderr: &str,
) -> Option<String> {
    let exited_successfully = exit_status.success();

    if exited_successfully {
        return None;
    }

    let status_code = exit_status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_owned());
    let trimmed_stderr = stderr.trim();
    let detail = if trimmed_stderr.is_empty() {
        "(empty stderr)".to_owned()
    } else {
        trimmed_stderr.to_owned()
    };
    let failure = format!(
        "delegate_async_process_spawn_failed: detached delegate child exited during startup with status {status_code}: {detail}"
    );

    Some(failure)
}

pub async fn run_detached_delegate_child_cli(
    config_path: &str,
    payload_file: &str,
) -> CliResult<()> {
    let payload_path = PathBuf::from(payload_file);
    let payload = read_detached_delegate_child_payload_file(payload_path.as_path())?;
    remove_detached_delegate_child_payload_file(payload_path.as_path());

    let (resolved_path, config) = mvp::config::load(Some(config_path))?;
    mvp::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));

    let binding = owned_binding_from_detached_payload(payload.binding, &config)?;
    let spawn_request = payload.into_spawn_request(binding)?;

    mvp::conversation::execute_async_delegate_spawn_request(&config, spawn_request).await?;

    Ok(())
}

fn resolve_detached_delegate_child_executable_path() -> CliResult<PathBuf> {
    let env_path = std::env::var_os(DETACHED_DELEGATE_CHILD_EXECUTABLE_ENV);

    if let Some(env_path) = env_path {
        let candidate_path = PathBuf::from(env_path);
        return Ok(candidate_path);
    }

    let executable_path = std::env::current_exe()
        .map_err(|error| format!("resolve detached delegate executable failed: {error}"))?;

    Ok(executable_path)
}

fn resolve_detached_delegate_child_config_path() -> CliResult<PathBuf> {
    let config_path = std::env::var_os("LOONGCLAW_CONFIG_PATH")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "delegate_async_process_spawn_failed: LOONGCLAW_CONFIG_PATH is not set for detached delegate child startup"
                .to_owned()
        })?;

    Ok(config_path)
}

fn materialize_detached_delegate_child_payload_file(
    payload: &DetachedDelegateChildPayload,
) -> CliResult<PathBuf> {
    let payload_directory = std::env::temp_dir()
        .join("loongclaw")
        .join("delegate-child-payloads");
    std::fs::create_dir_all(&payload_directory)
        .map_err(|error| format!("create detached delegate payload directory failed: {error}"))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            format!("read system clock for detached delegate payload failed: {error}")
        })?;
    let payload_file_name = format!(
        "delegate-child-{}-{}.json",
        std::process::id(),
        timestamp.as_nanos()
    );
    let payload_path = payload_directory.join(payload_file_name);
    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|error| format!("serialize detached delegate payload failed: {error}"))?;
    std::fs::write(&payload_path, payload_bytes)
        .map_err(|error| format!("write detached delegate payload failed: {error}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = std::fs::metadata(&payload_path)
            .map_err(|error| format!("stat detached delegate payload failed: {error}"))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(&payload_path, permissions)
            .map_err(|error| format!("chmod detached delegate payload failed: {error}"))?;
    }

    Ok(payload_path)
}

fn read_detached_delegate_child_payload_file(
    payload_path: &Path,
) -> CliResult<DetachedDelegateChildPayload> {
    let payload_bytes = std::fs::read(payload_path)
        .map_err(|error| format!("read detached delegate payload failed: {error}"))?;
    let payload = serde_json::from_slice::<DetachedDelegateChildPayload>(&payload_bytes)
        .map_err(|error| format!("parse detached delegate payload failed: {error}"))?;

    Ok(payload)
}

fn remove_detached_delegate_child_payload_file(payload_path: &Path) {
    let _ = std::fs::remove_file(payload_path);
}

fn propagate_detached_delegate_child_environment(command: &mut std::process::Command) {
    for env_key in DETACHED_DELEGATE_CHILD_PASSTHROUGH_ENV_KEYS {
        let env_value = std::env::var_os(env_key);

        if let Some(env_value) = env_value {
            command.env(env_key, env_value);
        }
    }
}

fn owned_binding_from_detached_payload(
    binding: DetachedDelegateChildBinding,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<mvp::conversation::OwnedConversationRuntimeBinding> {
    match binding {
        DetachedDelegateChildBinding::Kernel => {
            let kernel_context = mvp::context::bootstrap_kernel_context_with_config(
                DETACHED_DELEGATE_CHILD_KERNEL_SCOPE,
                mvp::context::DEFAULT_TOKEN_TTL_S,
                config,
            )?;
            let owned_binding =
                mvp::conversation::OwnedConversationRuntimeBinding::kernel(kernel_context);
            Ok(owned_binding)
        }
        DetachedDelegateChildBinding::Direct => {
            let owned_binding = mvp::conversation::OwnedConversationRuntimeBinding::direct();
            Ok(owned_binding)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::detached_delegate_child_startup_failure;

    #[cfg(unix)]
    fn exit_status_for(command: &str) -> std::process::ExitStatus {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .status()
            .expect("spawn shell command")
    }

    #[cfg(windows)]
    fn exit_status_for(command: &str) -> std::process::ExitStatus {
        std::process::Command::new("cmd")
            .args(["/C", command])
            .status()
            .expect("spawn cmd command")
    }

    #[test]
    fn detached_delegate_child_startup_failure_ignores_fast_success_with_warning_stderr() {
        #[cfg(unix)]
        let exit_status = exit_status_for("exit 0");
        #[cfg(windows)]
        let exit_status = exit_status_for("exit 0");

        let warning_output = "WARN optional runtime-self source missing";
        let failure = detached_delegate_child_startup_failure(&exit_status, warning_output);

        assert_eq!(failure, None);
    }

    #[test]
    fn detached_delegate_child_startup_failure_surfaces_non_zero_exit() {
        #[cfg(unix)]
        let exit_status = exit_status_for("exit 7");
        #[cfg(windows)]
        let exit_status = exit_status_for("exit 7");

        let failure = detached_delegate_child_startup_failure(&exit_status, "spawn failure");

        let failure = failure.expect("non-zero exit should be reported");
        assert!(failure.contains("status 7"), "failure={failure}");
        assert!(failure.contains("spawn failure"), "failure={failure}");
    }
}
