use std::io::ErrorKind;
use std::process::{Output, Stdio};
use std::time::Duration;

use loongclaw_app as mvp;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::path::Path;
use tokio::process::Command;
use tokio::time::timeout;

pub(crate) const BROWSER_COMPANION_INSTALL_CHECK_NAME: &str = "browser companion install";
pub(crate) const BROWSER_COMPANION_RUNTIME_GATE_CHECK_NAME: &str = "browser companion runtime gate";

const BROWSER_COMPANION_VERSION_ARG: &str = "--version";
const BROWSER_COMPANION_PROBE_ATTEMPTS: usize = 3;
#[cfg(test)]
const TEST_BROWSER_COMPANION_VERSION_PREFIX: &str = "test-browser-companion-version:";
#[cfg(unix)]
const POSIX_SH_PATH: &str = "/bin/sh";

fn browser_companion_probe_timeout_seconds(timeout_seconds: u64) -> u64 {
    timeout_seconds.max(1)
}

fn browser_companion_probe_timeout_duration(timeout_seconds: u64) -> Duration {
    let normalized_seconds = browser_companion_probe_timeout_seconds(timeout_seconds);
    let base_duration = Duration::from_secs(normalized_seconds);
    let slack_millis = normalized_seconds.saturating_mul(100);
    let bounded_slack_millis = slack_millis.min(500);
    let slack_duration = Duration::from_millis(bounded_slack_millis);
    base_duration.saturating_add(slack_duration)
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

#[cfg(test)]
pub(crate) fn fake_browser_companion_version_command(version: &str) -> String {
    format!("{TEST_BROWSER_COMPANION_VERSION_PREFIX}{version}")
}

async fn probe_browser_companion_version(
    command: &str,
    timeout_seconds: u64,
) -> Result<String, BrowserCompanionProbeError> {
    let timeout_duration = browser_companion_probe_timeout_duration(timeout_seconds);

    #[cfg(test)]
    if let Some(version) = command.strip_prefix(TEST_BROWSER_COMPANION_VERSION_PREFIX) {
        return Ok(format!("loongclaw-browser-companion {version}"));
    }

    for _attempt in 0..BROWSER_COMPANION_PROBE_ATTEMPTS {
        let mut probe = if should_probe_browser_companion_via_sh(command) {
            let shell = resolve_browser_companion_shell(command);
            let mut probe = Command::new(shell);
            probe.arg(command);
            probe
        } else {
            Command::new(command)
        };
        probe.arg(BROWSER_COMPANION_VERSION_ARG);
        probe.kill_on_drop(true);
        probe.stdout(Stdio::piped());
        probe.stderr(Stdio::piped());

        let probe_result = timeout(timeout_duration, probe.output()).await;
        match probe_result {
            Ok(Ok(output)) => return interpret_browser_companion_probe_output(output),
            Ok(Err(error)) => {
                if error.kind() == ErrorKind::NotFound {
                    return Err(BrowserCompanionProbeError::MissingBinary);
                }

                let error_message = error.to_string();
                return Err(BrowserCompanionProbeError::SpawnFailed(error_message));
            }
            Err(_) => {}
        }
    }

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

fn should_probe_browser_companion_via_sh(command: &str) -> bool {
    #[cfg(unix)]
    {
        let path = Path::new(command);
        if !path.exists() || !path.is_file() {
            return false;
        }
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        let mut reader = BufReader::new(file);
        let mut first_line = String::new();
        if reader.read_line(&mut first_line).is_err() {
            return false;
        }

        let Some(shebang) = first_line.strip_prefix("#!").map(str::trim) else {
            return false;
        };
        let mut tokens = shebang.split_ascii_whitespace();
        let Some(first_token) = tokens.next() else {
            return false;
        };
        let first_name = Path::new(first_token)
            .file_name()
            .and_then(|name| name.to_str());
        let interpreter = match first_name {
            Some("env") => tokens.find(|token| !token.starts_with('-')),
            Some(name) => Some(name),
            None => None,
        };

        matches!(interpreter, Some("sh" | "bash" | "zsh" | "dash"))
    }

    #[cfg(not(unix))]
    {
        let _ = command;
        false
    }
}

#[cfg(unix)]
fn resolve_browser_companion_shell(command: &str) -> String {
    let path = Path::new(command);
    let Ok(file) = std::fs::File::open(path) else {
        return POSIX_SH_PATH.to_owned();
    };
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).is_err() {
        return POSIX_SH_PATH.to_owned();
    }

    let Some(shebang) = first_line.strip_prefix("#!").map(str::trim) else {
        return POSIX_SH_PATH.to_owned();
    };
    let mut tokens = shebang.split_ascii_whitespace();
    let Some(first_token) = tokens.next() else {
        return POSIX_SH_PATH.to_owned();
    };
    let first_name = Path::new(first_token)
        .file_name()
        .and_then(|name| name.to_str());
    match first_name {
        Some("env") => tokens
            .find(|token| !token.starts_with('-'))
            .unwrap_or("sh")
            .to_owned(),
        Some(_) => first_token.to_owned(),
        None => POSIX_SH_PATH.to_owned(),
    }
}

#[cfg(not(unix))]
fn resolve_browser_companion_shell(_command: &str) -> String {
    "sh".to_owned()
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
        crate::test_support::write_executable_script_atomically(script_path, body);
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
    fn rustc_version_probe() -> (String, String, String) {
        let output = std::process::Command::new("rustc")
            .arg("--version")
            .output()
            .expect("run rustc --version");
        let observed_version = observed_output(&output.stdout, &output.stderr);
        let version_token = observed_version
            .split_whitespace()
            .nth(1)
            .expect("rustc --version should include a semantic version")
            .to_owned();
        let partial_components = version_token.split('.').collect::<Vec<_>>();
        let partial_version =
            partial_components[..partial_components.len().saturating_sub(1)].join(".");

        ("rustc".to_owned(), observed_version, partial_version)
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn collect_browser_companion_diagnostics_rejects_partial_expected_version_matches() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let (command, actual_observed_version, partial_version) = rustc_version_probe();

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(command);
        config.tools.browser_companion.expected_version = Some(partial_version.clone());

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
                } if expected_version == &partial_version
                    && observed_version == &actual_observed_version
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

    #[cfg(unix)]
    #[test]
    fn should_probe_browser_companion_via_sh_accepts_env_sh_script() {
        let temp_dir = browser_companion_temp_dir("env-sh");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(&script_path, "#!/usr/bin/env sh\necho ok\n");
        let script_path_text = script_path.to_str().expect("utf8 path");

        assert!(should_probe_browser_companion_via_sh(script_path_text));
    }

    #[cfg(unix)]
    #[test]
    fn should_probe_browser_companion_via_sh_rejects_non_posix_env_script() {
        let temp_dir = browser_companion_temp_dir("env-fish");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(&script_path, "#!/usr/bin/env fish\necho ok\n");
        let script_path_text = script_path.to_str().expect("utf8 path");

        assert!(!should_probe_browser_companion_via_sh(script_path_text));
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

    #[test]
    fn browser_companion_probe_timeout_duration_saturates() {
        let timeout_duration = browser_companion_probe_timeout_duration(u64::MAX);
        let expected = Duration::new(u64::MAX, 500_000_000);

        assert_eq!(timeout_duration, expected);
    }
}
