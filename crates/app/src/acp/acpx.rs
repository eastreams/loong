use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::Duration;

use crate::CliResult;
use crate::config::{AcpxMcpServerConfig, LoongClawConfig};
#[cfg(test)]
use crate::process_launch::retry_executable_file_busy_async;
#[cfg(test)]
use crate::process_launch::retry_executable_file_busy_blocking as retry_spawn_blocking;

#[cfg(test)]
pub(crate) use super::acpx_mcp::probe_mcp_proxy_support_with_runtime;
pub(crate) use super::acpx_mcp::{
    AcpxMcpServerEntry, AcpxMcpServerEnvEntry, build_mcp_proxy_agent_command,
    probe_mcp_proxy_support,
};
use super::backend::{
    AcpAbortSignal, AcpBackendMetadata, AcpCapability, AcpConfigPatch, AcpDoctorReport,
    AcpRuntimeBackend, AcpSessionBootstrap, AcpSessionHandle, AcpSessionMode, AcpSessionState,
    AcpSessionStatus, AcpTurnEventSink, AcpTurnRequest, AcpTurnResult, AcpTurnStopReason,
};

pub const ACPX_BACKEND_ID: &str = "acpx";
const ACPX_VERSION_ANY: &str = "any";
const ACPX_HANDLE_PREFIX: &str = "acpx:v1:";
const ACPX_DEFAULT_COMMAND: &str = "acpx";
const ACPX_DEFAULT_AGENT: &str = "codex";
const ACPX_DEFAULT_PERMISSION_MODE: &str = "approve-reads";
const ACPX_DEFAULT_NON_INTERACTIVE_PERMISSIONS: &str = "fail";
const ACPX_DEFAULT_QUEUE_OWNER_TTL_SECONDS: f64 = 0.1;
const ACPX_PERMISSION_DENIED_EXIT_CODE: i32 = 5;
const ACPX_SPAWN_RETRY_ATTEMPTS: usize = 5;
const ACPX_SPAWN_RETRY_DELAY: Duration = Duration::from_millis(25);

mod command_probe;
#[path = "acpx_command.rs"]
mod command_support;
#[path = "acpx_events.rs"]
mod event_support;
#[path = "acpx_handle.rs"]
mod handle_support;
#[path = "acpx_process.rs"]
mod process_support;

use command_probe::{CommandOutputError, wait_for_command_output};
use command_support::*;
use event_support::*;
use handle_support::*;
use process_support::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AcpxRuntimeHandleState {
    name: String,
    agent: String,
    cwd: String,
    mode: String,
    #[serde(default)]
    mcp_servers: Vec<String>,
    #[serde(default)]
    acpx_record_id: Option<String>,
    #[serde(default)]
    backend_session_id: Option<String>,
    #[serde(default)]
    agent_session_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedAcpxProfile {
    command: String,
    cwd: Option<String>,
    permission_mode: String,
    non_interactive_permissions: String,
    timeout_seconds: Option<f64>,
    queue_owner_ttl_seconds: f64,
    mcp_servers: BTreeMap<String, AcpxMcpServerConfig>,
}

#[derive(Default)]
pub struct AcpxCliProbeBackend;

#[async_trait]
impl AcpRuntimeBackend for AcpxCliProbeBackend {
    fn id(&self) -> &'static str {
        ACPX_BACKEND_ID
    }

    fn metadata(&self) -> AcpBackendMetadata {
        AcpBackendMetadata::new(
            self.id(),
            [
                AcpCapability::SessionLifecycle,
                AcpCapability::TurnExecution,
                AcpCapability::TurnEventStreaming,
                AcpCapability::Cancellation,
                AcpCapability::StatusInspection,
                AcpCapability::ModeSwitching,
                AcpCapability::ConfigPatching,
                AcpCapability::McpServerInjection,
                AcpCapability::Doctor,
            ],
            "ACPX runtime backend with command-based session lifecycle, turn execution, controls, and diagnostics.",
        )
    }

    async fn ensure_session(
        &self,
        config: &LoongClawConfig,
        request: &AcpSessionBootstrap,
    ) -> CliResult<AcpSessionHandle> {
        let profile = resolve_profile(config)?;
        let selected_mcp_servers = validate_requested_mcp_servers(config, &profile, request)?;
        let cwd =
            resolve_effective_cwd(request.working_directory.as_ref(), profile.cwd.as_deref())?;
        let agent = derive_agent_id(config, request.session_key.as_str(), &request.metadata)?;

        let ensure_args = build_verb_args(
            &profile,
            config.acp.startup_timeout_ms(),
            agent.as_str(),
            cwd.as_str(),
            selected_mcp_servers.as_slice(),
            build_control_args(&cwd),
            [
                "sessions".to_owned(),
                "ensure".to_owned(),
                "--name".to_owned(),
                request.session_key.clone(),
            ],
        )
        .await?;
        let mut events = run_json_command(
            &profile,
            ensure_args,
            cwd.as_str(),
            config.acp.startup_timeout_ms(),
            None,
            false,
        )
        .await?;
        let mut identifiers = extract_identifiers(&events);

        if !identifiers.ready() {
            let new_args = build_verb_args(
                &profile,
                config.acp.startup_timeout_ms(),
                agent.as_str(),
                cwd.as_str(),
                selected_mcp_servers.as_slice(),
                build_control_args(&cwd),
                [
                    "sessions".to_owned(),
                    "new".to_owned(),
                    "--name".to_owned(),
                    request.session_key.clone(),
                ],
            )
            .await?;
            events = run_json_command(
                &profile,
                new_args,
                cwd.as_str(),
                config.acp.startup_timeout_ms(),
                None,
                false,
            )
            .await?;
            identifiers = extract_identifiers(&events);
        }

        if !identifiers.ready() {
            return Err(format!(
                "ACPX ensure_session did not return any session identifiers for `{}`",
                request.session_key
            ));
        }

        let backend_session_id = identifiers
            .backend_session_id
            .clone()
            .or_else(|| identifiers.acpx_record_id.clone());
        let state = AcpxRuntimeHandleState {
            name: request.session_key.clone(),
            agent,
            cwd: cwd.clone(),
            mode: "persistent".to_owned(),
            mcp_servers: selected_mcp_servers,
            acpx_record_id: identifiers.acpx_record_id,
            backend_session_id: backend_session_id.clone(),
            agent_session_id: identifiers.agent_session_id.clone(),
        };

        Ok(AcpSessionHandle {
            session_key: request.session_key.clone(),
            backend_id: self.id().to_owned(),
            runtime_session_name: encode_runtime_handle_state(&state)?,
            working_directory: Some(PathBuf::from(&cwd)),
            backend_session_id,
            agent_session_id: identifiers.agent_session_id,
            binding: request.binding.clone(),
        })
    }

    async fn run_turn(
        &self,
        config: &LoongClawConfig,
        session: &AcpSessionHandle,
        request: &AcpTurnRequest,
    ) -> CliResult<AcpTurnResult> {
        self.run_turn_with_sink(config, session, request, None, None)
            .await
    }

    async fn run_turn_with_sink(
        &self,
        config: &LoongClawConfig,
        session: &AcpSessionHandle,
        request: &AcpTurnRequest,
        abort: Option<AcpAbortSignal>,
        sink: Option<&dyn AcpTurnEventSink>,
    ) -> CliResult<AcpTurnResult> {
        let profile = resolve_profile(config)?;
        let state = resolve_handle_state(&profile, session)?;
        let prompt_args = build_prompt_args(
            &profile,
            config.acp.startup_timeout_ms(),
            state.agent.as_str(),
            state.cwd.as_str(),
            state.mcp_servers.as_slice(),
        )
        .await?
        .into_iter()
        .chain([
            "prompt".to_owned(),
            "--session".to_owned(),
            state.name.clone(),
            "--file".to_owned(),
            "-".to_owned(),
        ])
        .collect::<Vec<_>>();

        run_prompt_process(
            profile.command.as_str(),
            &prompt_args,
            state.cwd.as_str(),
            config.acp.turn_timeout_ms(),
            request.input.as_str(),
            abort,
            sink,
        )
        .await
    }

    async fn cancel(&self, config: &LoongClawConfig, session: &AcpSessionHandle) -> CliResult<()> {
        let profile = resolve_profile(config)?;
        let state = resolve_handle_state(&profile, session)?;
        let args = build_verb_args(
            &profile,
            config.acp.startup_timeout_ms(),
            state.agent.as_str(),
            state.cwd.as_str(),
            state.mcp_servers.as_slice(),
            build_control_args(state.cwd.as_str()),
            [
                "cancel".to_owned(),
                "--session".to_owned(),
                state.name.clone(),
            ],
        )
        .await?;
        let _events = run_json_command(
            &profile,
            args,
            state.cwd.as_str(),
            config.acp.startup_timeout_ms(),
            None,
            true,
        )
        .await?;
        Ok(())
    }

    async fn close(&self, config: &LoongClawConfig, session: &AcpSessionHandle) -> CliResult<()> {
        let profile = resolve_profile(config)?;
        let state = resolve_handle_state(&profile, session)?;
        let args = build_verb_args(
            &profile,
            config.acp.startup_timeout_ms(),
            state.agent.as_str(),
            state.cwd.as_str(),
            state.mcp_servers.as_slice(),
            build_control_args(state.cwd.as_str()),
            [
                "sessions".to_owned(),
                "close".to_owned(),
                state.name.clone(),
            ],
        )
        .await?;
        let _events = run_json_command(
            &profile,
            args,
            state.cwd.as_str(),
            config.acp.startup_timeout_ms(),
            None,
            true,
        )
        .await?;
        Ok(())
    }

    async fn get_status(
        &self,
        config: &LoongClawConfig,
        session: &AcpSessionHandle,
    ) -> CliResult<Option<AcpSessionStatus>> {
        let profile = resolve_profile(config)?;
        let state = resolve_handle_state(&profile, session)?;
        let args = build_verb_args(
            &profile,
            config.acp.startup_timeout_ms(),
            state.agent.as_str(),
            state.cwd.as_str(),
            state.mcp_servers.as_slice(),
            build_control_args(state.cwd.as_str()),
            [
                "status".to_owned(),
                "--session".to_owned(),
                state.name.clone(),
            ],
        )
        .await?;
        let events = run_json_command(
            &profile,
            args,
            state.cwd.as_str(),
            config.acp.startup_timeout_ms(),
            None,
            true,
        )
        .await?;

        let no_session = event_code(&events).as_deref() == Some("NO_SESSION");
        let detail = events
            .iter()
            .find(|event| value_string(event, "type") != Some("error".to_owned()))
            .cloned();
        let status_name = if no_session {
            Some("closed".to_owned())
        } else {
            detail
                .as_ref()
                .and_then(|event| value_string(event, "status"))
        };
        let last_error = if no_session {
            None
        } else {
            event_error_message(&events, false)
        };

        Ok(Some(AcpSessionStatus {
            session_key: session.session_key.clone(),
            backend_id: self.id().to_owned(),
            conversation_id: None,
            binding: session.binding.clone(),
            activation_origin: None,
            state: map_status_state(status_name.as_deref()),
            mode: None,
            pending_turns: 0,
            active_turn_id: None,
            last_activity_ms: now_ms(),
            last_error,
        }))
    }

    async fn set_mode(
        &self,
        config: &LoongClawConfig,
        session: &AcpSessionHandle,
        mode: AcpSessionMode,
    ) -> CliResult<()> {
        let profile = resolve_profile(config)?;
        let state = resolve_handle_state(&profile, session)?;
        let args = build_verb_args(
            &profile,
            config.acp.startup_timeout_ms(),
            state.agent.as_str(),
            state.cwd.as_str(),
            state.mcp_servers.as_slice(),
            build_control_args(state.cwd.as_str()),
            [
                "set-mode".to_owned(),
                mode_label(mode).to_owned(),
                "--session".to_owned(),
                state.name.clone(),
            ],
        )
        .await?;
        let _events = run_json_command(
            &profile,
            args,
            state.cwd.as_str(),
            config.acp.startup_timeout_ms(),
            None,
            false,
        )
        .await?;
        Ok(())
    }

    async fn set_config_option(
        &self,
        config: &LoongClawConfig,
        session: &AcpSessionHandle,
        patch: &AcpConfigPatch,
    ) -> CliResult<()> {
        let profile = resolve_profile(config)?;
        let state = resolve_handle_state(&profile, session)?;
        let key = normalized_non_empty(patch.key.as_str())
            .ok_or_else(|| "ACPX config option key must not be empty".to_owned())?;
        let value = normalized_non_empty(patch.value.as_str())
            .ok_or_else(|| "ACPX config option value must not be empty".to_owned())?;
        let args = build_verb_args(
            &profile,
            config.acp.startup_timeout_ms(),
            state.agent.as_str(),
            state.cwd.as_str(),
            state.mcp_servers.as_slice(),
            build_control_args(state.cwd.as_str()),
            [
                "set".to_owned(),
                key,
                value,
                "--session".to_owned(),
                state.name.clone(),
            ],
        )
        .await?;
        let _events = run_json_command(
            &profile,
            args,
            state.cwd.as_str(),
            config.acp.startup_timeout_ms(),
            None,
            false,
        )
        .await?;
        Ok(())
    }

    async fn doctor(&self, config: &LoongClawConfig) -> CliResult<Option<AcpDoctorReport>> {
        let raw_profile = config.acp.acpx_profile().cloned().unwrap_or_default();
        let command = raw_profile
            .command()
            .unwrap_or_else(|| ACPX_DEFAULT_COMMAND.to_owned());
        let expected_version = raw_profile.expected_version();
        let cwd = raw_profile.cwd();
        let mut diagnostics = BTreeMap::from([
            ("backend".to_owned(), self.id().to_owned()),
            ("command".to_owned(), command.clone()),
            (
                "expected_version".to_owned(),
                expected_version
                    .clone()
                    .unwrap_or_else(|| ACPX_VERSION_ANY.to_owned()),
            ),
            (
                "mcp_server_count".to_owned(),
                raw_profile.mcp_servers.len().to_string(),
            ),
        ]);

        if let Some(permission_mode) = raw_profile.permission_mode() {
            diagnostics.insert("permission_mode".to_owned(), permission_mode);
        }
        if let Some(non_interactive_permissions) = raw_profile.non_interactive_permissions() {
            diagnostics.insert(
                "non_interactive_permissions".to_owned(),
                non_interactive_permissions,
            );
        }
        if let Some(timeout_seconds) = raw_profile.timeout_seconds {
            diagnostics.insert("timeout_seconds".to_owned(), timeout_seconds.to_string());
        }
        if let Some(queue_owner_ttl_seconds) = raw_profile.queue_owner_ttl_seconds {
            diagnostics.insert(
                "queue_owner_ttl_seconds".to_owned(),
                queue_owner_ttl_seconds.to_string(),
            );
        }
        if let Some(strict_windows_cmd_wrapper) = raw_profile.strict_windows_cmd_wrapper {
            diagnostics.insert(
                "strict_windows_cmd_wrapper".to_owned(),
                strict_windows_cmd_wrapper.to_string(),
            );
        }
        if let Some(cwd) = cwd.clone() {
            diagnostics.insert("cwd".to_owned(), cwd);
        }
        if config.acp.allow_mcp_server_injection
            && let Err(error) = crate::mcp::McpRegistry::from_config(config)
        {
            diagnostics.insert("status".to_owned(), "invalid_config".to_owned());
            diagnostics.insert("error".to_owned(), error);
            return Ok(Some(AcpDoctorReport {
                healthy: false,
                diagnostics,
            }));
        }
        if let Err(error) = resolve_profile(config) {
            diagnostics.insert("status".to_owned(), "invalid_config".to_owned());
            diagnostics.insert("error".to_owned(), error);
            return Ok(Some(AcpDoctorReport {
                healthy: false,
                diagnostics,
            }));
        }

        let probe_timeout = Duration::from_millis(config.acp.startup_timeout_ms());
        let mut mcp_proxy_ready = true;
        if raw_profile.mcp_servers.is_empty() {
            diagnostics.insert(
                "mcp_runtime_proxy".to_owned(),
                "disabled_no_backend_mcp_servers".to_owned(),
            );
        } else if !config.acp.allow_mcp_server_injection {
            diagnostics.insert(
                "mcp_runtime_proxy".to_owned(),
                "available_but_disabled_by_policy".to_owned(),
            );
        } else {
            match probe_mcp_proxy_support(cwd.as_deref(), probe_timeout).await {
                Ok((script_path, node_version)) => {
                    diagnostics.insert(
                        "mcp_runtime_proxy".to_owned(),
                        "embedded_node_proxy".to_owned(),
                    );
                    diagnostics.insert("mcp_runtime_proxy_script".to_owned(), script_path);
                    diagnostics.insert("mcp_runtime_proxy_node".to_owned(), node_version);
                }
                Err(error) => {
                    diagnostics.insert("mcp_runtime_proxy".to_owned(), "unavailable".to_owned());
                    diagnostics.insert("mcp_runtime_proxy_error".to_owned(), error);
                    mcp_proxy_ready = false;
                }
            }
        }

        if let Some(cwd) = cwd.as_deref()
            && !Path::new(cwd).exists()
        {
            diagnostics.insert("status".to_owned(), "missing_cwd".to_owned());
            diagnostics.insert(
                "error".to_owned(),
                format!("ACP runtime working directory does not exist: {cwd}"),
            );
            return Ok(Some(AcpDoctorReport {
                healthy: false,
                diagnostics,
            }));
        }

        let mut probe = Command::new(&command);
        probe.arg("--version");
        if let Some(cwd) = cwd {
            probe.current_dir(cwd);
        }

        match wait_for_command_output(&mut probe, probe_timeout).await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                let observed = match (stdout.is_empty(), stderr.is_empty()) {
                    (false, true) => stdout,
                    (true, false) => stderr,
                    (false, false) => format!("{stdout} | {stderr}"),
                    (true, true) => "(empty)".to_owned(),
                };
                diagnostics.insert("observed_output".to_owned(), observed.clone());
                diagnostics.insert(
                    "exit_status".to_owned(),
                    output
                        .status
                        .code()
                        .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
                );

                let version_matches = expected_version
                    .as_deref()
                    .map(|value| {
                        value.eq_ignore_ascii_case(ACPX_VERSION_ANY) || observed.contains(value)
                    })
                    .unwrap_or(true);
                let healthy = output.status.success() && version_matches && mcp_proxy_ready;
                diagnostics.insert(
                    "status".to_owned(),
                    if !mcp_proxy_ready {
                        "mcp_proxy_unavailable".to_owned()
                    } else if healthy {
                        "ready".to_owned()
                    } else if !output.status.success() {
                        "execution_failed".to_owned()
                    } else {
                        "version_mismatch".to_owned()
                    },
                );
                Ok(Some(AcpDoctorReport {
                    healthy,
                    diagnostics,
                }))
            }
            Err(error) => {
                let (status, error_message) = match error {
                    CommandOutputError::TimedOut => {
                        ("timed_out", "acpx version probe timed out".to_owned())
                    }
                    CommandOutputError::Io(error) => {
                        let status = if error.kind() == ErrorKind::NotFound {
                            "missing_command"
                        } else {
                            "spawn_failed"
                        };
                        let error_message = error.to_string();
                        (status, error_message)
                    }
                };
                diagnostics.insert("status".to_owned(), status.to_owned());
                diagnostics.insert("error".to_owned(), error_message);
                Ok(Some(AcpDoctorReport {
                    healthy: false,
                    diagnostics,
                }))
            }
        }
    }
}

impl AcpxIdentifiers {
    fn ready(&self) -> bool {
        self.acpx_record_id.is_some()
            || self.backend_session_id.is_some()
            || self.agent_session_id.is_some()
    }
}

#[allow(dead_code)]
fn map_spawn_error(command: &str, cwd: &str, error: std::io::Error) -> String {
    if error.kind() == ErrorKind::NotFound {
        if !Path::new(cwd).exists() {
            return format!("ACP runtime working directory does not exist: {cwd}");
        }
        return format!("acpx command not found: {command}");
    }
    format!("spawn ACPX command failed: {error}")
}

#[allow(dead_code)]
async fn spawn_acpx_child(
    command: &str,
    args: &[String],
    cwd: &str,
    pipe_stdin: bool,
) -> CliResult<tokio::process::Child> {
    retry_executable_file_busy(|| {
        let mut process = Command::new(command);
        process
            .args(args)
            .current_dir(cwd)
            .stdin(if pipe_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        process.spawn()
    })
    .await
    .map_err(|error| map_spawn_error(command, cwd, error))
}

#[cfg(test)]
async fn retry_executable_file_busy<T, F>(operation: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    retry_executable_file_busy_async(operation, ACPX_SPAWN_RETRY_ATTEMPTS, ACPX_SPAWN_RETRY_DELAY)
        .await
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
fn retry_executable_file_busy_blocking<T, F>(operation: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    retry_spawn_blocking(operation, ACPX_SPAWN_RETRY_ATTEMPTS, ACPX_SPAWN_RETRY_DELAY)
}

fn format_exit_message(stderr: &str, exit_code: Option<i32>) -> String {
    let trimmed = stderr.trim();
    if exit_code == Some(ACPX_PERMISSION_DENIED_EXIT_CODE) {
        return if trimmed.is_empty() {
            "Permission denied by ACP runtime (acpx); configure permission_mode to approve-reads, approve-all, or deny-all as needed".to_owned()
        } else {
            format!(
                "{trimmed} ACPX blocked a permission request in non-interactive mode; configure permission_mode to approve-reads, approve-all, or deny-all as needed"
            )
        };
    }
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }
    format!(
        "acpx exited with code {}",
        exit_code.map_or_else(|| "unknown".to_owned(), |code| code.to_string())
    )
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::sync::OnceLock;
    #[cfg(unix)]
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[cfg(unix)]
    use tokio::sync::Mutex;

    use super::*;
    use crate::config::{AcpBackendProfilesConfig, AcpConfig, AcpxBackendConfig, LoongClawConfig};
    use crate::test_support::ScopedEnv;

    const ACPX_RUNTIME_TEST_TIMEOUT_SECONDS: f64 = 45.0;

    #[cfg(unix)]
    fn acpx_runtime_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(unix)]
    async fn lock_acpx_runtime_tests() -> tokio::sync::MutexGuard<'static, ()> {
        acpx_runtime_test_lock().lock().await
    }

    #[cfg(unix)]
    const ACPX_FAKE_RUNTIME_STARTUP_TIMEOUT_MS: u64 = 60_000;

    #[cfg(unix)]
    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static NEXT_TEMP_DIR_SEED: AtomicU64 = AtomicU64::new(1);
        let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}-{seed}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        temp_dir
    }

    #[cfg(unix)]
    fn write_executable_script_atomically(
        script_path: &Path,
        contents: &str,
    ) -> std::io::Result<()> {
        write_executable_script_atomically_with(script_path, |file| {
            std::io::Write::write_all(file, contents.as_bytes())
        })
    }

    #[cfg(unix)]
    fn write_executable_script_atomically_with<F>(
        script_path: &Path,
        writer: F,
    ) -> std::io::Result<()>
    where
        F: FnOnce(&mut std::fs::File) -> std::io::Result<()>,
    {
        static NEXT_STAGING_FILE_SEED: AtomicU64 = AtomicU64::new(1);

        let Some(parent) = script_path.parent() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "fake acpx script path `{}` has no parent directory",
                    script_path.display()
                ),
            ));
        };
        let Some(file_name) = script_path.file_name().and_then(|name| name.to_str()) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "fake acpx script path `{}` has no UTF-8 file name",
                    script_path.display()
                ),
            ));
        };

        let seed = NEXT_STAGING_FILE_SEED.fetch_add(1, Ordering::Relaxed);
        let staged_path = parent.join(format!(".{file_name}.{}.{seed}.tmp", std::process::id()));
        let mut staged_file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged_path)?;
        let write_result = writer(&mut staged_file).and_then(|()| staged_file.sync_all());
        drop(staged_file);

        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&staged_path);
            return Err(error);
        }

        let mut permissions = std::fs::metadata(&staged_path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&staged_path, permissions)?;
        if let Err(error) = std::fs::rename(&staged_path, script_path) {
            let _ = std::fs::remove_file(&staged_path);
            return Err(error);
        }

        Ok(())
    }

    #[cfg(unix)]
    fn write_fake_acpx_script(
        temp_dir: &Path,
        script_name: &str,
        log_path: &Path,
        body: &str,
    ) -> PathBuf {
        let script_path = temp_dir.join(script_name);
        let script_source = format!(
            "#!/bin/sh\nset -eu\n# Keep test helper scripts stable even when unrelated tests narrow PATH.\nPATH=\"$(command -p getconf PATH 2>/dev/null || printf '%s' '/usr/bin:/bin')\"\nexport PATH\nLOG_PATH=\"{}\"\nprintf '%s\\n' \"$*\" >> \"$LOG_PATH\"\nargs_contain() {{\n  case \"$1\" in\n    *\"$2\"*) return 0 ;;\n    *) return 1 ;;\n  esac\n}}\ndrain_stdin() {{\n  if [ ! -t 0 ]; then\n    cat >/dev/null\n  fi\n}}\n{}\n",
            log_path.display(),
            body
        );
        write_executable_script_atomically(&script_path, &script_source)
            .expect("write fake acpx script");
        script_path
    }

    #[cfg(unix)]
    mod mcp_proxy_tests;
    #[cfg(unix)]
    mod path_tests;

    #[test]
    #[cfg(unix)]
    fn write_executable_script_atomically_preserves_existing_script_when_write_fails() {
        let temp_dir = unique_temp_dir("loongclaw-acpx-script-atomic");
        let script_path = temp_dir.join("fake-acpx");

        write_executable_script_atomically(&script_path, "#!/bin/sh\necho old\n")
            .expect("write baseline fake acpx script");

        let error = write_executable_script_atomically_with(&script_path, |file| {
            std::io::Write::write_all(file, b"#!/bin/sh\necho new\n")?;
            Err(std::io::Error::other("simulated staging failure"))
        })
        .expect_err("staging failure should surface");

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(
            std::fs::read_to_string(&script_path).expect("read baseline fake acpx script"),
            "#!/bin/sh\necho old\n"
        );

        let staging_entries = std::fs::read_dir(&temp_dir)
            .expect("list temp dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(staging_entries, 0, "staging files should be cleaned up");
    }

    #[tokio::test]
    async fn retry_executable_file_busy_retries_until_success() {
        let attempts = AtomicUsize::new(0);

        let result = retry_executable_file_busy(|| {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed);
            if attempt < 2 {
                Err(std::io::Error::from(ErrorKind::ExecutableFileBusy))
            } else {
                Ok("spawned")
            }
        })
        .await
        .expect("retry should recover once the executable is no longer busy");

        assert_eq!(result, "spawned");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn retry_executable_file_busy_surfaces_non_retryable_error_immediately() {
        let attempts = AtomicUsize::new(0);

        let error = retry_executable_file_busy::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::other("boom"))
        })
        .await
        .expect_err("non-retryable spawn errors should surface immediately");

        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn retry_executable_file_busy_stops_after_retry_budget() {
        let attempts = AtomicUsize::new(0);

        let error = retry_executable_file_busy::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::from(ErrorKind::ExecutableFileBusy))
        })
        .await
        .expect_err("persistent executable-file-busy errors should stop after the retry budget");

        assert_eq!(error.kind(), ErrorKind::ExecutableFileBusy);
        assert_eq!(attempts.load(Ordering::Relaxed), ACPX_SPAWN_RETRY_ATTEMPTS);
    }

    #[test]
    fn retry_executable_file_busy_blocking_retries_until_success() {
        let attempts = AtomicUsize::new(0);

        let result = retry_executable_file_busy_blocking(|| {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed);
            if attempt < 2 {
                Err(std::io::Error::from(ErrorKind::ExecutableFileBusy))
            } else {
                Ok("spawned")
            }
        })
        .expect("retry should recover once the executable is no longer busy");

        assert_eq!(result, "spawned");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn retry_executable_file_busy_blocking_surfaces_non_retryable_error_immediately() {
        let attempts = AtomicUsize::new(0);

        let error = retry_executable_file_busy_blocking::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::other("boom"))
        })
        .expect_err("non-retryable spawn errors should surface immediately");

        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn retry_executable_file_busy_blocking_stops_after_retry_budget() {
        let attempts = AtomicUsize::new(0);

        let error = retry_executable_file_busy_blocking::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(std::io::Error::from(ErrorKind::ExecutableFileBusy))
        })
        .expect_err("persistent executable-file-busy errors should stop after the retry budget");

        assert_eq!(error.kind(), ErrorKind::ExecutableFileBusy);
        assert_eq!(attempts.load(Ordering::Relaxed), ACPX_SPAWN_RETRY_ATTEMPTS);
    }

    #[cfg(unix)]
    fn fake_acpx_config(script_path: &Path, cwd: &Path) -> LoongClawConfig {
        let startup_timeout_ms = ACPX_FAKE_RUNTIME_STARTUP_TIMEOUT_MS;

        LoongClawConfig {
            acp: AcpConfig {
                startup_timeout_ms: Some(startup_timeout_ms),
                allow_mcp_server_injection: false,
                backends: AcpBackendProfilesConfig {
                    acpx: Some(AcpxBackendConfig {
                        command: Some(script_path.display().to_string()),
                        expected_version: Some("0.1.16".to_owned()),
                        cwd: Some(cwd.display().to_string()),
                        permission_mode: Some("approve-reads".to_owned()),
                        non_interactive_permissions: Some("fail".to_owned()),
                        timeout_seconds: Some(ACPX_RUNTIME_TEST_TIMEOUT_SECONDS),
                        queue_owner_ttl_seconds: Some(0.25),
                        ..AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        }
    }

    #[test]
    #[cfg(unix)]
    fn fake_acpx_config_uses_explicit_process_test_startup_timeout() {
        let temp_dir = unique_temp_dir("loongclaw-acpx-config-timeout");
        let script_path = temp_dir.join("fake-acpx");

        let config = fake_acpx_config(&script_path, &temp_dir);
        let startup_timeout_ms = config.acp.startup_timeout_ms();

        assert_eq!(startup_timeout_ms, ACPX_FAKE_RUNTIME_STARTUP_TIMEOUT_MS);
    }

    #[tokio::test]
    async fn doctor_reports_missing_command() {
        let backend = AcpxCliProbeBackend;
        let config = LoongClawConfig {
            acp: AcpConfig {
                backends: AcpBackendProfilesConfig {
                    acpx: Some(AcpxBackendConfig {
                        command: Some("/definitely/not/a/real/acpx".to_owned()),
                        expected_version: Some("0.1.16".to_owned()),
                        ..AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let report = backend
            .doctor(&config)
            .await
            .expect("doctor should not fail")
            .expect("doctor report");
        assert!(!report.healthy);
        assert_eq!(
            report.diagnostics.get("status").map(String::as_str),
            Some("missing_command")
        );
    }

    #[test]
    fn metadata_exposes_mcp_server_injection_capability() {
        let metadata = AcpxCliProbeBackend.metadata();

        assert!(
            metadata
                .capabilities
                .contains(&AcpCapability::McpServerInjection)
        );
    }

    #[test]
    fn derive_agent_id_prefers_session_key_prefix() {
        let mut config = LoongClawConfig::default();
        config.acp.default_agent = Some("codex".to_owned());
        config.acp.allowed_agents = vec!["codex".to_owned(), "claude".to_owned()];
        let metadata = BTreeMap::from([("acp_agent".to_owned(), "claude".to_owned())]);

        let derived =
            derive_agent_id(&config, "agent:claude:session-1", &metadata).expect("derive agent");
        assert_eq!(derived, "claude");
    }

    #[test]
    fn derive_agent_id_uses_configured_default_when_session_has_no_agent_prefix() {
        let mut config = LoongClawConfig::default();
        config.acp.default_agent = Some("gemini".to_owned());
        config.acp.allowed_agents = vec!["codex".to_owned(), "gemini".to_owned()];

        let derived =
            derive_agent_id(&config, "telegram:42", &BTreeMap::new()).expect("derive agent");
        assert_eq!(derived, "gemini");
    }

    #[test]
    fn derive_agent_id_rejects_mismatched_metadata_agent() {
        let mut config = LoongClawConfig::default();
        config.acp.default_agent = Some("codex".to_owned());
        config.acp.allowed_agents = vec!["codex".to_owned(), "claude".to_owned()];
        let metadata = BTreeMap::from([("acp_agent".to_owned(), "codex".to_owned())]);

        let error = derive_agent_id(&config, "agent:claude:session-1", &metadata)
            .expect_err("mismatched ACP agent metadata must fail");
        assert!(error.contains("does not match"));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn doctor_accepts_fake_version_command() {
        let _env = crate::test_support::ScopedEnv::new();
        let temp_dir = unique_temp_dir("loongclaw-acpx-probe");
        let script_path = temp_dir.join("fake-acpx");
        write_executable_script_atomically(&script_path, "#!/bin/sh\necho 'acpx 0.1.16'\n")
            .expect("write fake acpx script");

        let backend = AcpxCliProbeBackend;
        let config = LoongClawConfig {
            acp: AcpConfig {
                backends: AcpBackendProfilesConfig {
                    acpx: Some(AcpxBackendConfig {
                        command: Some(script_path.display().to_string()),
                        expected_version: Some("0.1.16".to_owned()),
                        mcp_servers: BTreeMap::from([(
                            "filesystem".to_owned(),
                            crate::config::AcpxMcpServerConfig {
                                command: "npx".to_owned(),
                                args: vec!["@modelcontextprotocol/server-filesystem".to_owned()],
                                env: BTreeMap::new(),
                            },
                        )]),
                        ..AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let mut last_report = None;
        for attempt in 0..5 {
            let report = backend
                .doctor(&config)
                .await
                .expect("doctor should not fail")
                .expect("doctor report");
            if report.healthy {
                last_report = Some(report);
                break;
            }
            last_report = Some(report);
            if attempt < 4 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }

        let report = last_report.expect("doctor report");
        assert!(
            report.healthy,
            "doctor should accept fake version command: {:?}",
            report.diagnostics
        );
        assert_eq!(
            report.diagnostics.get("command"),
            Some(&script_path.display().to_string())
        );
        assert_eq!(
            report.diagnostics.get("mcp_server_count"),
            Some(&"1".to_owned())
        );
        assert_eq!(
            report.diagnostics.get("mcp_runtime_proxy"),
            Some(&"available_but_disabled_by_policy".to_owned())
        );

        let _ = std::fs::remove_file(&script_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn doctor_accepts_path_discovered_fake_version_command() {
        let _guard = lock_acpx_runtime_tests().await;
        let temp_dir = unique_temp_dir("loongclaw-acpx-probe-path");
        let bin_dir = temp_dir.join("bin");
        let script_path = bin_dir.join("fake-acpx");
        std::fs::create_dir_all(&bin_dir).expect("create bin dir");
        write_executable_script_atomically(&script_path, "#!/bin/sh\necho 'acpx 0.1.16'\n")
            .expect("write fake acpx script");

        let mut env = ScopedEnv::new();
        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let original_entries = std::env::split_paths(&original_path);
        let mut path_entries = vec![bin_dir.clone()];
        path_entries.extend(original_entries);
        let joined_path = std::env::join_paths(path_entries).expect("join PATH");
        env.set("PATH", joined_path);

        let backend = AcpxCliProbeBackend;
        let config = LoongClawConfig {
            acp: AcpConfig {
                backends: AcpBackendProfilesConfig {
                    acpx: Some(AcpxBackendConfig {
                        command: Some("fake-acpx".to_owned()),
                        expected_version: Some("0.1.16".to_owned()),
                        cwd: Some(temp_dir.display().to_string()),
                        ..AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let report = backend
            .doctor(&config)
            .await
            .expect("doctor should not fail")
            .expect("doctor report");

        assert!(report.healthy, "doctor should use launcher path");
        assert_eq!(
            report.diagnostics.get("command"),
            Some(&"fake-acpx".to_owned())
        );
        assert_eq!(report.diagnostics.get("status"), Some(&"ready".to_owned()));
    }

    #[tokio::test]
    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    async fn runtime_backend_uses_agent_proxy_when_mcp_servers_requested() {
        let _lock = lock_acpx_runtime_tests().await;
        let _env = crate::test_support::ScopedEnv::new();
        let temp_dir = unique_temp_dir("loongclaw-acpx-mcp-proxy");
        let log_path = temp_dir.join("calls.log");
        let script_path = write_fake_acpx_script(
            &temp_dir,
            "fake-acpx",
            &log_path,
            r#"
case "$*" in
  "--version")
    echo 'acpx 0.1.16'
    exit 0
    ;;
esac

case "$*" in
  *"config show"*)
    echo '{"agents":{"codex":{"command":"npx @zed-industries/codex-acp"}}}'
    exit 0
    ;;
esac

case "$*" in
  *"sessions ensure --name"*)
    echo '{"acpxSessionId":"sess-mcp","agentSessionId":"agent-mcp","acpxRecordId":"record-mcp"}'
    exit 0
    ;;
esac

case "$*" in
  *"prompt --session"*)
    drain_stdin
    echo '{"type":"text","content":"proxy ok"}'
    echo '{"type":"done"}'
    exit 0
    ;;
esac

exit 0
"#,
        );
        let mut config = fake_acpx_config(&script_path, &temp_dir);
        config.acp.allow_mcp_server_injection = true;
        config.acp.backends.acpx = Some(AcpxBackendConfig {
            command: Some(script_path.display().to_string()),
            expected_version: Some("0.1.16".to_owned()),
            cwd: Some(temp_dir.display().to_string()),
            permission_mode: Some("approve-reads".to_owned()),
            non_interactive_permissions: Some("fail".to_owned()),
            timeout_seconds: Some(12.5),
            queue_owner_ttl_seconds: Some(0.25),
            mcp_servers: BTreeMap::from([(
                "filesystem".to_owned(),
                crate::config::AcpxMcpServerConfig {
                    command: "npx".to_owned(),
                    args: vec![
                        "-y".to_owned(),
                        "@modelcontextprotocol/server-filesystem".to_owned(),
                        temp_dir.display().to_string(),
                    ],
                    env: BTreeMap::from([("ROOT".to_owned(), temp_dir.display().to_string())]),
                },
            )]),
            ..AcpxBackendConfig::default()
        });

        let backend = AcpxCliProbeBackend;
        let bootstrap = AcpSessionBootstrap {
            session_key: "session-proxy".to_owned(),
            conversation_id: Some("telegram:mcp".to_owned()),
            binding: None,
            working_directory: Some(temp_dir.clone()),
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: vec!["filesystem".to_owned()],
            metadata: BTreeMap::new(),
        };

        let handle = backend
            .ensure_session(&config, &bootstrap)
            .await
            .expect("ensure session with MCP proxy");
        let result = backend
            .run_turn(
                &config,
                &handle,
                &AcpTurnRequest {
                    session_key: bootstrap.session_key.clone(),
                    input: "test proxy path".to_owned(),
                    working_directory: None,
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect("run proxied turn");
        assert_eq!(result.output_text, "proxy ok");

        let log = std::fs::read_to_string(&log_path).expect("read fake acpx log");
        assert!(
            log.contains("config show"),
            "expected agent override lookup in log: {log}"
        );
        assert!(
            log.contains("--agent"),
            "expected --agent proxy flag in log: {log}"
        );
        assert!(
            log.contains("--payload-file"),
            "expected MCP proxy payload file flag in log: {log}"
        );
        assert!(
            log.contains("sessions ensure --name session-proxy"),
            "expected ensure command in log: {log}"
        );
        assert!(
            log.contains("prompt --session session-proxy --file -"),
            "expected prompt command in log: {log}"
        );
        assert!(
            !log.contains("codex sessions ensure --name session-proxy"),
            "expected raw agent positional form to be replaced by --agent proxy: {log}"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn ensure_session_rejects_unknown_requested_mcp_server_names() {
        let temp_dir = unique_temp_dir("loongclaw-acpx-mcp-unknown");
        let log_path = temp_dir.join("calls.log");
        let script_path = write_fake_acpx_script(
            &temp_dir,
            "fake-acpx",
            &log_path,
            "echo '{\"acpxSessionId\":\"unused\"}'\n",
        );
        let mut config = fake_acpx_config(&script_path, &temp_dir);
        config.acp.allow_mcp_server_injection = true;
        config.acp.backends.acpx = Some(AcpxBackendConfig {
            mcp_servers: BTreeMap::from([(
                "filesystem".to_owned(),
                crate::config::AcpxMcpServerConfig {
                    command: "npx".to_owned(),
                    args: vec!["@modelcontextprotocol/server-filesystem".to_owned()],
                    env: BTreeMap::new(),
                },
            )]),
            ..AcpxBackendConfig::default()
        });

        let backend = AcpxCliProbeBackend;
        let error = backend
            .ensure_session(
                &config,
                &AcpSessionBootstrap {
                    session_key: "session-unknown-mcp".to_owned(),
                    conversation_id: None,
                    binding: None,
                    working_directory: Some(temp_dir),
                    initial_prompt: None,
                    mode: Some(AcpSessionMode::Interactive),
                    mcp_servers: vec!["missing".to_owned()],
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect_err("unknown MCP server should fail");

        assert!(
            error.contains("missing") && error.contains("mcp_servers"),
            "expected missing MCP server validation error, got: {error}"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    async fn runtime_backend_executes_session_turn_and_controls() {
        let _lock = lock_acpx_runtime_tests().await;
        let _env = crate::test_support::ScopedEnv::new();
        let temp_dir = unique_temp_dir("loongclaw-acpx-runtime");
        let log_path = temp_dir.join("calls.log");
        let script_path = write_fake_acpx_script(
            &temp_dir,
            "fake-acpx",
            &log_path,
            r#"
case "$*" in
  "--version")
    echo 'acpx 0.1.16'
    exit 0
    ;;
esac

case "$*" in
  *"sessions ensure --name"*)
    echo '{"acpxSessionId":"sess-42","agentSessionId":"agent-42","acpxRecordId":"record-42"}'
    exit 0
    ;;
esac

case "$*" in
  *"prompt --session"*)
    drain_stdin
    echo '{"type":"text","content":"hello "}'
    echo '{"type":"text","content":"world"}'
    echo '{"type":"usage_update","used":7,"size":128}'
    echo '{"type":"done"}'
    exit 0
    ;;
esac

case "$*" in
  *"status --session"*)
    echo '{"status":"ready","acpxSessionId":"sess-42","agentSessionId":"agent-42","acpxRecordId":"record-42"}'
    exit 0
    ;;
esac

exit 0
"#,
        );
        let config = fake_acpx_config(&script_path, &temp_dir);
        let backend = AcpxCliProbeBackend;
        let bootstrap = AcpSessionBootstrap {
            session_key: "agent:codex:session-42".to_owned(),
            conversation_id: Some("telegram:42".to_owned()),
            binding: Some(crate::acp::AcpSessionBindingScope {
                route_session_id: "telegram:bot_123456:42".to_owned(),
                channel_id: Some("telegram".to_owned()),
                account_id: Some("bot_123456".to_owned()),
                conversation_id: Some("42".to_owned()),
                participant_id: None,
                thread_id: Some("thread-42".to_owned()),
            }),
            working_directory: Some(temp_dir.clone()),
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let handle = backend
            .ensure_session(&config, &bootstrap)
            .await
            .expect("ensure session");
        assert_eq!(handle.backend_id, ACPX_BACKEND_ID);
        assert_eq!(handle.backend_session_id.as_deref(), Some("sess-42"));
        assert_eq!(handle.agent_session_id.as_deref(), Some("agent-42"));
        assert_eq!(
            handle.working_directory.as_deref(),
            Some(temp_dir.as_path())
        );

        let result = backend
            .run_turn(
                &config,
                &handle,
                &AcpTurnRequest {
                    session_key: bootstrap.session_key.clone(),
                    input: "hello runtime".to_owned(),
                    working_directory: None,
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect("run turn");
        assert_eq!(result.output_text, "hello world");
        assert_eq!(result.state, AcpSessionState::Ready);
        assert_eq!(
            result.usage,
            Some(serde_json::json!({
                "used": 7,
                "size": 128,
            }))
        );

        let status = backend
            .get_status(&config, &handle)
            .await
            .expect("status should succeed")
            .expect("status payload");
        assert_eq!(status.session_key, "agent:codex:session-42");
        assert_eq!(status.backend_id, ACPX_BACKEND_ID);
        assert_eq!(
            status
                .binding
                .as_ref()
                .map(|binding| binding.route_session_id.as_str()),
            Some("telegram:bot_123456:42")
        );
        assert_eq!(
            status
                .binding
                .as_ref()
                .and_then(|binding| binding.thread_id.as_deref()),
            Some("thread-42")
        );
        assert_eq!(status.state, AcpSessionState::Ready);
        assert_eq!(status.pending_turns, 0);
        assert!(status.active_turn_id.is_none());

        backend
            .set_mode(&config, &handle, AcpSessionMode::Review)
            .await
            .expect("set mode");
        backend
            .set_config_option(
                &config,
                &handle,
                &AcpConfigPatch {
                    key: "temperature".to_owned(),
                    value: "0.1".to_owned(),
                },
            )
            .await
            .expect("set config option");
        backend
            .cancel(&config, &handle)
            .await
            .expect("cancel session");
        backend
            .close(&config, &handle)
            .await
            .expect("close session");

        let log = std::fs::read_to_string(&log_path).expect("read fake acpx log");
        assert!(
            log.contains("sessions ensure --name agent:codex:session-42"),
            "expected ensure command in log: {log}"
        );
        assert!(
            log.contains("prompt --session agent:codex:session-42 --file -"),
            "expected prompt command in log: {log}"
        );
        assert!(
            log.contains("--approve-reads"),
            "expected permission mode args in log: {log}"
        );
        assert!(
            log.contains("--non-interactive-permissions fail"),
            "expected non-interactive permissions in log: {log}"
        );
        assert!(log.contains("--ttl 0.25"), "expected ttl in log: {log}");
        assert!(
            log.contains("set-mode review --session agent:codex:session-42"),
            "expected set-mode command in log: {log}"
        );
        assert!(
            log.contains("set temperature 0.1 --session agent:codex:session-42"),
            "expected set command in log: {log}"
        );
        assert!(
            log.contains("cancel --session agent:codex:session-42"),
            "expected cancel command in log: {log}"
        );
        assert!(
            log.contains("sessions close agent:codex:session-42"),
            "expected close command in log: {log}"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    async fn runtime_backend_supports_local_abort_for_running_prompt() {
        let _lock = lock_acpx_runtime_tests().await;
        let _env = crate::test_support::ScopedEnv::new();
        let temp_dir = unique_temp_dir("loongclaw-acpx-abort");
        let log_path = temp_dir.join("calls.log");
        let script_path = write_fake_acpx_script(
            &temp_dir,
            "fake-acpx",
            &log_path,
            r#"
case "$*" in
  "--version")
    echo 'acpx 0.1.16'
    exit 0
    ;;
esac

case "$*" in
  *"sessions ensure --name"*)
    echo '{"acpxSessionId":"sess-abort","agentSessionId":"agent-abort","acpxRecordId":"record-abort"}'
    exit 0
    ;;
esac

case "$*" in
  *"prompt --session"*)
    drain_stdin
    /bin/sleep 30
    exit 0
    ;;
esac

exit 0
"#,
        );
        let config = fake_acpx_config(&script_path, &temp_dir);
        let backend = AcpxCliProbeBackend;
        let bootstrap = AcpSessionBootstrap {
            session_key: "agent:codex:session-abort".to_owned(),
            conversation_id: Some("telegram:abort".to_owned()),
            binding: None,
            working_directory: Some(temp_dir.clone()),
            initial_prompt: None,
            mode: Some(AcpSessionMode::Interactive),
            mcp_servers: Vec::new(),
            metadata: BTreeMap::new(),
        };
        let handle = backend
            .ensure_session(&config, &bootstrap)
            .await
            .expect("ensure abortable session");

        let abort_controller = crate::acp::AcpAbortController::new();
        let abort_signal = abort_controller.signal();
        let turn_task = {
            let backend = AcpxCliProbeBackend;
            let config = config.clone();
            let handle = handle.clone();
            let session_key = bootstrap.session_key.clone();
            tokio::spawn(async move {
                backend
                    .run_turn_with_sink(
                        &config,
                        &handle,
                        &AcpTurnRequest {
                            session_key,
                            input: "abort me".to_owned(),
                            working_directory: None,
                            metadata: BTreeMap::new(),
                        },
                        Some(abort_signal),
                        None,
                    )
                    .await
            })
        };

        tokio::time::sleep(Duration::from_millis(150)).await;
        abort_controller.abort();

        let result = tokio::time::timeout(Duration::from_secs(2), async {
            turn_task
                .await
                .expect("abortable turn join should succeed")
                .expect("abortable turn result should resolve")
        })
        .await
        .expect("aborted prompt should stop promptly");

        assert_eq!(result.state, AcpSessionState::Ready);
        assert_eq!(result.stop_reason, Some(AcpTurnStopReason::Cancelled));
        assert_eq!(result.output_text, "");
        assert_eq!(
            result
                .events
                .last()
                .and_then(|event| value_string(event, "stopReason")),
            Some("cancelled".to_owned())
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    #[allow(clippy::await_holding_lock)]
    async fn ensure_session_falls_back_to_sessions_new_when_ensure_has_no_identifiers() {
        let _lock = lock_acpx_runtime_tests().await;
        let _env = crate::test_support::ScopedEnv::new();
        let temp_dir = unique_temp_dir("loongclaw-acpx-fallback");
        let log_path = temp_dir.join("calls.log");
        let script_path = write_fake_acpx_script(
            &temp_dir,
            "fake-acpx",
            &log_path,
            r#"
case "$*" in
  "--version")
    echo 'acpx 0.1.16'
    exit 0
    ;;
esac

case "$*" in
  *"sessions ensure --name"*)
    echo '{}'
    exit 0
    ;;
esac

case "$*" in
  *"sessions new --name"*)
    echo '{"acpxSessionId":"sess-fallback","agentSessionId":"agent-fallback","acpxRecordId":"record-fallback"}'
    exit 0
    ;;
esac

exit 0
"#,
        );
        let config = fake_acpx_config(&script_path, &temp_dir);
        let backend = AcpxCliProbeBackend;

        let handle = backend
            .ensure_session(
                &config,
                &AcpSessionBootstrap {
                    session_key: "session-fallback".to_owned(),
                    conversation_id: None,
                    binding: None,
                    working_directory: Some(temp_dir.clone()),
                    initial_prompt: None,
                    mode: Some(AcpSessionMode::Interactive),
                    mcp_servers: Vec::new(),
                    metadata: BTreeMap::new(),
                },
            )
            .await
            .expect("fallback ensure");

        assert_eq!(handle.backend_session_id.as_deref(), Some("sess-fallback"));
        assert_eq!(handle.agent_session_id.as_deref(), Some("agent-fallback"));
        let log = std::fs::read_to_string(&log_path).expect("read fake acpx log");
        assert!(
            log.contains("sessions ensure --name session-fallback"),
            "expected ensure attempt in log: {log}"
        );
        assert!(
            log.contains("sessions new --name session-fallback"),
            "expected fallback sessions new in log: {log}"
        );
    }
}
