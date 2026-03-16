use std::io::ErrorKind;
use std::time::Duration;

use loongclaw_app as mvp;
use tokio::process::Command;
use tokio::time::timeout;

pub(crate) const BROWSER_COMPANION_INSTALL_CHECK_NAME: &str = "browser companion install";
pub(crate) const BROWSER_COMPANION_RUNTIME_GATE_CHECK_NAME: &str = "browser companion runtime gate";

const BROWSER_COMPANION_VERSION_ARG: &str = "--version";
const BROWSER_COMPANION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

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
            BrowserCompanionInstallStatus::ProbeTimedOut { command } => {
                format!(
                    "command `{command} {BROWSER_COMPANION_VERSION_ARG}` timed out after {}s",
                    BROWSER_COMPANION_PROBE_TIMEOUT.as_secs()
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
    let Some(command) = runtime.command else {
        return Some(BrowserCompanionDiagnostics {
            command: None,
            expected_version,
            observed_version: None,
            runtime_ready,
            install_status: BrowserCompanionInstallStatus::MissingCommand,
        });
    };

    match probe_browser_companion_version(&command).await {
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
        Err(BrowserCompanionProbeError::TimedOut) => Some(BrowserCompanionDiagnostics {
            command: Some(command.clone()),
            expected_version,
            observed_version: None,
            runtime_ready,
            install_status: BrowserCompanionInstallStatus::ProbeTimedOut { command },
        }),
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
) -> Result<String, BrowserCompanionProbeError> {
    let mut probe = Command::new(command);
    probe.arg(BROWSER_COMPANION_VERSION_ARG);
    probe.kill_on_drop(true);

    match timeout(BROWSER_COMPANION_PROBE_TIMEOUT, probe.output()).await {
        Ok(Ok(output)) => {
            let observed = observed_output(&output.stdout, &output.stderr);
            if output.status.success() {
                Ok(observed)
            } else {
                Err(BrowserCompanionProbeError::Exited {
                    observed,
                    exit_status: output.status.code(),
                })
            }
        }
        Ok(Err(error)) => {
            if error.kind() == ErrorKind::NotFound {
                Err(BrowserCompanionProbeError::MissingBinary)
            } else {
                Err(BrowserCompanionProbeError::SpawnFailed(error.to_string()))
            }
        }
        Err(_) => Err(BrowserCompanionProbeError::TimedOut),
    }
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
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::sync::MutexGuard;
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
        let mut permissions = file.metadata().expect("script metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(script_path, permissions).expect("chmod browser companion script");
    }

    #[cfg(unix)]
    fn set_browser_companion_env_var(key: &str, value: &str) {
        // SAFETY: daemon tests serialize process env mutations behind
        // `lock_daemon_test_environment`, so no concurrent env readers/writers
        // observe racy updates while these tests run.
        #[allow(unsafe_code, clippy::disallowed_methods)]
        unsafe {
            std::env::set_var(key, value);
        }
    }

    #[cfg(unix)]
    fn remove_browser_companion_env_var(key: &str) {
        // SAFETY: daemon tests serialize process env mutations behind
        // `lock_daemon_test_environment`, so removing the variable here is
        // coordinated with all other env-mutating daemon tests.
        #[allow(unsafe_code, clippy::disallowed_methods)]
        unsafe {
            std::env::remove_var(key);
        }
    }

    #[cfg(unix)]
    struct BrowserCompanionEnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved_ready: Option<OsString>,
    }

    #[cfg(unix)]
    impl BrowserCompanionEnvGuard {
        fn runtime_gate_closed() -> Self {
            let lock = crate::test_support::lock_daemon_test_environment();
            let key = "LOONGCLAW_BROWSER_COMPANION_READY";
            let saved_ready = std::env::var_os(key);
            remove_browser_companion_env_var(key);
            Self {
                _lock: lock,
                saved_ready,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for BrowserCompanionEnvGuard {
        fn drop(&mut self) {
            let key = "LOONGCLAW_BROWSER_COMPANION_READY";
            match self.saved_ready.take() {
                Some(value) => set_browser_companion_env_var(key, &value.to_string_lossy()),
                None => remove_browser_companion_env_var(key),
            }
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
}
