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
                Some(expected_version) if !observed_version.contains(expected_version) => {
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
