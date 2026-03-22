use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-shell")]
use serde_json::{Value, json};
#[cfg(feature = "tool-shell")]
use std::process::Stdio;
#[cfg(feature = "tool-shell")]
use std::time::Duration;
#[cfg(feature = "tool-shell")]
use std::{future::Future, path::PathBuf, thread};
#[cfg(feature = "tool-shell")]
use tokio::{io::AsyncReadExt, process::Command};

pub(super) fn execute_shell_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-shell"))]
    {
        let _ = (request, config);
        return Err(
            "shell tool is disabled in this build (enable feature `tool-shell`)".to_owned(),
        );
    }

    #[cfg(feature = "tool-shell")]
    {
        let payload = request
            .payload
            .as_object()
            .ok_or_else(|| "shell.exec payload must be an object".to_owned())?;
        let command = payload
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "shell.exec requires payload.command".to_owned())?;
        let args = payload
            .get("args")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cwd = payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let timeout_ms = parse_shell_timeout_ms(payload)?;

        let normalized_command =
            crate::tools::shell_policy_ext::validate_shell_command_name(command)?;
        let basename = normalized_command.as_str();

        if config.shell_deny.contains(basename) {
            return Err(format!(
                "policy_denied: shell command `{basename}` is blocked by shell policy"
            ));
        }

        let explicitly_allowed = config.shell_allow.contains(basename);
        let default_allows = matches!(
            config.shell_default_mode,
            crate::tools::shell_policy_ext::ShellPolicyDefault::Allow
        );
        if !explicitly_allowed && !default_allows {
            return Err(format!(
                "policy_denied: shell command `{basename}` is not in the allow list (default-deny policy)"
            ));
        }

        let output = run_shell_async(async {
            run_shell_command_with_timeout(normalized_command.as_str(), &args, &cwd, timeout_ms)
                .await
        })??;

        Ok(ToolCoreOutcome {
            status: if output.status.success() {
                "ok".to_owned()
            } else {
                "failed".to_owned()
            },
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "command": command,
                "args": args,
                "cwd": cwd.display().to_string(),
                "exit_code": output.status.code(),
                "stdout": String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                "stderr": String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            }),
        })
    }
}

#[cfg(feature = "tool-shell")]
const SHELL_EXEC_DEFAULT_TIMEOUT_MS: u64 = 10_000;
#[cfg(feature = "tool-shell")]
const SHELL_EXEC_MAX_TIMEOUT_MS: u64 = 120_000;

#[cfg(feature = "tool-shell")]
fn parse_shell_timeout_ms(payload: &serde_json::Map<String, Value>) -> Result<u64, String> {
    let timeout_ms = match payload.get("timeout_ms") {
        Some(timeout_ms) => timeout_ms
            .as_u64()
            .ok_or_else(|| "shell.exec payload.timeout_ms must be an integer".to_owned())?,
        None => SHELL_EXEC_DEFAULT_TIMEOUT_MS,
    };

    Ok(timeout_ms.clamp(1, SHELL_EXEC_MAX_TIMEOUT_MS))
}

#[cfg(feature = "tool-shell")]
fn run_shell_async<F>(future: F) -> Result<F::Output, String>
where
    F: Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            Ok(tokio::task::block_in_place(|| handle.block_on(future)))
        }
        Ok(_) => thread::scope(|scope| {
            scope
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create tokio runtime for shell tool: {error}")
                        })?;
                    Ok(rt.block_on(future))
                })
                .join()
                .map_err(|_panic| "shell tool async worker thread panicked".to_owned())?
        }),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    format!("failed to create tokio runtime for shell tool: {error}")
                })?;
            Ok(rt.block_on(future))
        }
    }
}

#[cfg(feature = "tool-shell")]
async fn run_shell_command_with_timeout(
    command: &str,
    args: &[String],
    cwd: &PathBuf,
    timeout_ms: u64,
) -> Result<std::process::Output, String> {
    let mut child = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("shell command spawn failed: {error}"))?;

    let duration = Duration::from_millis(timeout_ms.max(1));
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "shell command stdout pipe missing".to_owned())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "shell command stderr pipe missing".to_owned())?;

    let stdout_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stdout
            .read_to_end(&mut output)
            .await
            .map_err(|error| format!("shell command stdout read failed: {error}"))?;
        Ok::<Vec<u8>, String>(output)
    });
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr
            .read_to_end(&mut output)
            .await
            .map_err(|error| format!("shell command stderr read failed: {error}"))?;
        Ok::<Vec<u8>, String>(output)
    });

    match tokio::time::timeout(duration, child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout_result, stderr_result) = tokio::join!(stdout_task, stderr_task);
            let stdout = stdout_result.map_err(|join_error| {
                format!("shell command stdout reader panicked: {join_error}")
            })??;
            let stderr = stderr_result.map_err(|join_error| {
                format!("shell command stderr reader panicked: {join_error}")
            })??;

            Ok(std::process::Output {
                status,
                stdout,
                stderr,
            })
        }
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = tokio::join!(stdout_task, stderr_task);
            Err(format!("shell command wait failed: {error}"))
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = tokio::join!(stdout_task, stderr_task);
            Err(format!("shell command timed out after {timeout_ms}ms"))
        }
    }
}
