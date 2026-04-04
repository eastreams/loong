use std::io::ErrorKind;
use std::process::{Command as BlockingCommand, Output, Stdio};
use std::time::Duration;

use loongclaw_app as mvp;
use wait_timeout::ChildExt;

pub(crate) const BROWSER_COMPANION_INSTALL_CHECK_NAME: &str = "browser companion install";
pub(crate) const BROWSER_COMPANION_RUNTIME_GATE_CHECK_NAME: &str = "browser companion runtime gate";

const BROWSER_COMPANION_VERSION_ARG: &str = "--version";
const BROWSER_COMPANION_PROBE_ATTEMPTS: usize = 3;

fn browser_companion_probe_timeout_seconds(timeout_seconds: u64) -> u64 {
    timeout_seconds.max(1)
}

// Shared readiness snapshot for doctor/onboard so the companion lane is probed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserCompanionDiagnostics {
    pub(crate) command: Option<String>,
    pub(crate) expected_version: Option<String>,
    pub(crate) observed_version: Option<String>,
    pub(crate) runtime_ready: bool,
    pub(crate) install_status: BrowserCompanionInstallStatus,
}

impl BrowserCompanionDiagnostics {
    pub(crate) fn install_ready(&self) -> bool {
        matches!(self.install_status, BrowserCompanionInstallStatus::Ready)
    }

    pub(crate) fn install_detail(&self) -> String {
        match &self.install_status {
            BrowserCompanionInstallStatus::MissingCommand => {
                "browser companion is enabled, but no command is configured under `tools.browser_companion.command`"
                    .to_owned()
            }
            BrowserCompanionInstallStatus::MissingBinary { command } => {
                format!("command `{command}` was not found on PATH")
            }
            BrowserCompanionInstallStatus::ProbeTimedOut {
                command,
                timeout_seconds,
            } => {
                let timeout_seconds = browser_companion_probe_timeout_seconds(*timeout_seconds);
                format!(
                    "command `{command} {BROWSER_COMPANION_VERSION_ARG}` timed out after {}s",
                    timeout_seconds
                )
            }
            BrowserCompanionInstallStatus::ProbeFailed { command, error } => {
                format!(
                    "command `{command} {BROWSER_COMPANION_VERSION_ARG}` failed before reporting a version: {error}"
                )
            }
            BrowserCompanionInstallStatus::ProbeExited {
                command,
                observed,
                exit_status,
            } => {
                let exit_status = exit_status
                    .map_or_else(|| "signal".to_owned(), |code| code.to_string());
                format!(
                    "command `{command} {BROWSER_COMPANION_VERSION_ARG}` exited with status {exit_status}: {observed}"
                )
            }
            BrowserCompanionInstallStatus::VersionMismatch {
                command,
                expected_version,
                observed_version,
            } => {
                format!(
                    "command `{command}` responded, but expected_version={expected_version} observed_version={observed_version}"
                )
            }
            BrowserCompanionInstallStatus::Ready => {
                let command = self.command.as_deref().unwrap_or("browser companion");
                let observed_version = self.observed_version.as_deref().unwrap_or("(empty)");
                format!("command `{command}` responded with `{observed_version}`")
            }
        }
    }

    pub(crate) fn runtime_gate_detail(&self) -> Option<String> {
        if !self.install_ready() {
            return None;
        }

        let observed_version = self.observed_version.as_deref().unwrap_or("(empty)");
        Some(if self.runtime_ready {
            format!("managed browser companion runtime is ready ({observed_version})")
        } else {
            format!(
                "install looks healthy ({observed_version}), but the runtime gate is still closed (`LOONGCLAW_BROWSER_COMPANION_READY` is false)"
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserCompanionInstallStatus {
    MissingCommand,
    MissingBinary {
        command: String,
    },
    ProbeTimedOut {
        command: String,
        timeout_seconds: u64,
    },
    ProbeFailed {
        command: String,
        error: String,
    },
    ProbeExited {
        command: String,
        observed: String,
        exit_status: Option<i32>,
    },
    VersionMismatch {
        command: String,
        expected_version: String,
        observed_version: String,
    },
    Ready,
}

#[derive(Debug)]
enum BrowserCompanionProbeError {
    MissingBinary,
    TimedOut,
    SpawnFailed(String),
    Exited {
        observed: String,
        exit_status: Option<i32>,
    },
}

pub(crate) async fn collect_browser_companion_diagnostics(
    config: &mvp::config::LoongClawConfig,
) -> Option<BrowserCompanionDiagnostics> {
    let runtime =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, None)
            .browser_companion;
    if !runtime.enabled {
        return None;
    }

    let runtime_ready = runtime.is_runtime_ready();
    let expected_version = runtime.expected_version;
    let probe_timeout_seconds = browser_companion_probe_timeout_seconds(runtime.timeout_seconds);
    let Some(command) = runtime.command else {
        return Some(BrowserCompanionDiagnostics {
            command: None,
            expected_version,
            observed_version: None,
            runtime_ready,
            install_status: BrowserCompanionInstallStatus::MissingCommand,
        });
    };

    match probe_browser_companion_version(&command, probe_timeout_seconds).await {
        Ok(observed_version) => {
            let install_status = match expected_version.as_deref() {
                Some(expected_version)
                    if !observed_version_matches_expected(&observed_version, expected_version) =>
                {
                    BrowserCompanionInstallStatus::VersionMismatch {
                        command: command.clone(),
                        expected_version: expected_version.to_owned(),
                        observed_version: observed_version.clone(),
                    }
                }
                _ => BrowserCompanionInstallStatus::Ready,
            };
            Some(BrowserCompanionDiagnostics {
                command: Some(command),
                expected_version,
                observed_version: Some(observed_version),
                runtime_ready,
                install_status,
            })
        }
        Err(BrowserCompanionProbeError::MissingBinary) => Some(BrowserCompanionDiagnostics {
            command: Some(command.clone()),
            expected_version,
            observed_version: None,
            runtime_ready,
            install_status: BrowserCompanionInstallStatus::MissingBinary { command },
        }),
        Err(BrowserCompanionProbeError::TimedOut) => {
            let timed_out_command = command.clone();
            let install_status = BrowserCompanionInstallStatus::ProbeTimedOut {
                command,
                timeout_seconds: probe_timeout_seconds,
            };
            Some(BrowserCompanionDiagnostics {
                command: Some(timed_out_command),
                expected_version,
                observed_version: None,
                runtime_ready,
                install_status,
            })
        }
        Err(BrowserCompanionProbeError::SpawnFailed(error)) => Some(BrowserCompanionDiagnostics {
            command: Some(command.clone()),
            expected_version,
            observed_version: None,
            runtime_ready,
            install_status: BrowserCompanionInstallStatus::ProbeFailed { command, error },
        }),
        Err(BrowserCompanionProbeError::Exited {
            observed,
            exit_status,
        }) => Some(BrowserCompanionDiagnostics {
            command: Some(command.clone()),
            expected_version,
            observed_version: Some(observed.clone()),
            runtime_ready,
            install_status: BrowserCompanionInstallStatus::ProbeExited {
                command,
                observed,
                exit_status,
            },
        }),
    }
}

async fn probe_browser_companion_version(
    command: &str,
    timeout_seconds: u64,
) -> Result<String, BrowserCompanionProbeError> {
    let command = command.to_owned();
    let join_result = tokio::task::spawn_blocking(move || {
        for _attempt in 0..BROWSER_COMPANION_PROBE_ATTEMPTS {
            let probe_result =
                probe_browser_companion_version_once(command.as_str(), timeout_seconds);
            match probe_result {
                Err(BrowserCompanionProbeError::TimedOut) => {}
                result => {
                    return result;
                }
            }
        }

        Err(BrowserCompanionProbeError::TimedOut)
    })
    .await;

    match join_result {
        Ok(result) => result,
        Err(error) => Err(BrowserCompanionProbeError::SpawnFailed(error.to_string())),
    }
}

fn probe_browser_companion_version_once(
    command: &str,
    timeout_seconds: u64,
) -> Result<String, BrowserCompanionProbeError> {
    let child = spawn_browser_companion_probe_process(command)?;
    let output = wait_for_browser_companion_probe_output(child, timeout_seconds)?;
    interpret_browser_companion_probe_output(output)
}

fn spawn_browser_companion_probe_process(
    command: &str,
) -> Result<std::process::Child, BrowserCompanionProbeError> {
    let mut process = BlockingCommand::new(command);
    process.arg(BROWSER_COMPANION_VERSION_ARG);
    process.stdin(Stdio::null());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());

    let spawn_result = process.spawn();
    match spawn_result {
        Ok(child) => Ok(child),
        Err(error) => {
            if error.kind() == ErrorKind::NotFound {
                return Err(BrowserCompanionProbeError::MissingBinary);
            }

            let error_message = error.to_string();
            Err(BrowserCompanionProbeError::SpawnFailed(error_message))
        }
    }
}

fn wait_for_browser_companion_probe_output(
    mut child: std::process::Child,
    timeout_seconds: u64,
) -> Result<Output, BrowserCompanionProbeError> {
    let timeout_seconds = browser_companion_probe_timeout_seconds(timeout_seconds);
    let timeout_duration = Duration::from_secs(timeout_seconds);
    let wait_result = child.wait_timeout(timeout_duration);
    let status_option = wait_result.map_err(|error| {
        let error_message = error.to_string();
        BrowserCompanionProbeError::SpawnFailed(error_message)
    })?;

    if status_option.is_some() {
        let output_result = child.wait_with_output();
        return output_result.map_err(|error| {
            let error_message = error.to_string();
            BrowserCompanionProbeError::SpawnFailed(error_message)
        });
    }

    let _ = child.kill();
    let _ = child.wait();
    Err(BrowserCompanionProbeError::TimedOut)
}

fn interpret_browser_companion_probe_output(
    output: Output,
) -> Result<String, BrowserCompanionProbeError> {
    let observed = observed_output(&output.stdout, &output.stderr);
    let status = output.status;

    if status.success() {
        return Ok(observed);
    }

    let exit_status = status.code();
    Err(BrowserCompanionProbeError::Exited {
        observed,
        exit_status,
    })
}

fn observed_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout} | {stderr}"),
        (true, true) => "(empty)".to_owned(),
    }
}

fn observed_version_matches_expected(observed_version: &str, expected_version: &str) -> bool {
    observed_version
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '_')
        .filter(|token| !token.is_empty())
        .any(|token| token == expected_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    fn browser_companion_temp_dir(label: &str) -> PathBuf {
        static NEXT_TEMP_DIR_SEED: AtomicU64 = AtomicU64::new(1);
        let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-browser-companion-diagnostics-{label}-{}-{seed}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create browser companion diagnostics temp dir");
        temp_dir
    }

    #[cfg(unix)]
    fn write_browser_companion_script(script_path: &Path, body: &str) {
        let mut file = std::fs::File::create(script_path).expect("create browser companion script");
        file.write_all(body.as_bytes())
            .expect("write browser companion script");
        file.sync_all()
            .expect("sync browser companion script to disk");
        drop(file);
        let mut permissions = std::fs::metadata(script_path)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(script_path, permissions).expect("chmod browser companion script");
    }

    #[cfg(unix)]
    struct BrowserCompanionEnvGuard {
        _env: crate::test_support::ScopedEnv,
    }

    #[cfg(unix)]
    impl BrowserCompanionEnvGuard {
        fn runtime_gate_closed() -> Self {
            let key = "LOONGCLAW_BROWSER_COMPANION_READY";
            let mut env = crate::test_support::ScopedEnv::new();
            env.remove(key.to_owned());
            Self { _env: env }
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn collect_browser_companion_diagnostics_rejects_partial_expected_version_matches() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let temp_dir = browser_companion_temp_dir("partial-version-match");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\necho 'loongclaw-browser-companion 11.5.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());
        config.tools.browser_companion.timeout_seconds = 3;

        let diagnostics = collect_browser_companion_diagnostics(&config)
            .await
            .expect("diagnostics should be collected");

        assert!(
            matches!(
                diagnostics.install_status,
                BrowserCompanionInstallStatus::VersionMismatch {
                    ref expected_version,
                    ref observed_version,
                    ..
                } if expected_version == "1.5.0"
                    && observed_version == "loongclaw-browser-companion 11.5.0"
            ),
            "partial version matches should still warn as mismatches: {diagnostics:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn collect_browser_companion_diagnostics_tolerates_slow_version_mismatches() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let temp_dir = browser_companion_temp_dir("slow-version-mismatch");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\nsleep 4\necho 'loongclaw-browser-companion 11.5.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());
        config.tools.browser_companion.timeout_seconds = 5;

        let diagnostics = collect_browser_companion_diagnostics(&config)
            .await
            .expect("diagnostics should be collected");

        assert!(
            matches!(
                diagnostics.install_status,
                BrowserCompanionInstallStatus::VersionMismatch {
                    ref expected_version,
                    ref observed_version,
                    ..
                } if expected_version == "1.5.0"
                    && observed_version == "loongclaw-browser-companion 11.5.0"
            ),
            "slow version probes should still surface mismatches before timing out: {diagnostics:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn collect_browser_companion_diagnostics_retries_transient_probe_timeouts() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let temp_dir = browser_companion_temp_dir("transient-timeout");
        let script_path = temp_dir.join("browser-companion");
        let state_path = temp_dir.join("probe-state");
        let script_body = format!(
            "#!/bin/sh\nstate_path='{}'\nif [ ! -f \"$state_path\" ]; then\n  touch \"$state_path\"\n  /bin/sleep 6\nfi\necho 'loongclaw-browser-companion 1.5.0'\n",
            state_path.display()
        );
        write_browser_companion_script(&script_path, script_body.as_str());

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());
        config.tools.browser_companion.timeout_seconds = 5;

        let diagnostics = collect_browser_companion_diagnostics(&config)
            .await
            .expect("diagnostics should be collected");

        assert_eq!(
            diagnostics.install_status,
            BrowserCompanionInstallStatus::Ready,
            "transient probe timeouts should retry before surfacing an install warning: {diagnostics:#?}"
        );
        assert_eq!(
            diagnostics.observed_version.as_deref(),
            Some("loongclaw-browser-companion 1.5.0")
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn collect_browser_companion_diagnostics_recovers_after_two_transient_timeouts() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let temp_dir = browser_companion_temp_dir("double-transient-timeout");
        let script_path = temp_dir.join("browser-companion");
        let first_timeout_path = temp_dir.join("probe-timeout-1");
        let second_timeout_path = temp_dir.join("probe-timeout-2");
        let script_body = format!(
            "#!/bin/sh\nfirst_timeout_path='{}'\nsecond_timeout_path='{}'\nif [ ! -f \"$first_timeout_path\" ]; then\n  touch \"$first_timeout_path\"\n  /bin/sleep 6\nfi\nif [ ! -f \"$second_timeout_path\" ]; then\n  touch \"$second_timeout_path\"\n  /bin/sleep 6\nfi\necho 'loongclaw-browser-companion 1.5.0'\n",
            first_timeout_path.display(),
            second_timeout_path.display()
        );
        write_browser_companion_script(&script_path, script_body.as_str());

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());
        config.tools.browser_companion.timeout_seconds = 5;

        let diagnostics = collect_browser_companion_diagnostics(&config)
            .await
            .expect("diagnostics should be collected");

        assert_eq!(
            diagnostics.install_status,
            BrowserCompanionInstallStatus::Ready,
            "two transient probe timeouts should still recover before surfacing an install warning: {diagnostics:#?}"
        );
        assert_eq!(
            diagnostics.observed_version.as_deref(),
            Some("loongclaw-browser-companion 1.5.0")
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn collect_browser_companion_diagnostics_times_out_stalled_probe() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let temp_dir = browser_companion_temp_dir("stalled-probe");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\n/bin/sleep 2\necho 'loongclaw-browser-companion 1.5.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());
        config.tools.browser_companion.timeout_seconds = 1;

        let diagnostics = collect_browser_companion_diagnostics(&config)
            .await
            .expect("diagnostics should be collected");

        let install_status = &diagnostics.install_status;
        let timed_out = matches!(
            install_status,
            BrowserCompanionInstallStatus::ProbeTimedOut { .. }
        );

        assert!(
            timed_out,
            "stalled probes should time out deterministically: {diagnostics:#?}"
        );
    }

    #[test]
    fn observed_version_matches_expected_accepts_exact_tokens() {
        assert!(observed_version_matches_expected(
            "loongclaw-browser-companion 1.5.0",
            "1.5.0"
        ));
    }

    #[test]
    fn observed_version_matches_expected_rejects_suffix_variants() {
        assert!(!observed_version_matches_expected(
            "loongclaw-browser-companion 1.5.0-beta",
            "1.5.0"
        ));
    }

    #[test]
    fn observed_version_matches_expected_rejects_partial_numeric_matches() {
        assert!(!observed_version_matches_expected(
            "loongclaw-browser-companion 11.5.0",
            "1.5.0"
        ));
    }
}
