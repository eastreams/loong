use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-shell")]
use serde_json::{Value, json};
#[cfg(feature = "tool-shell")]
use std::path::PathBuf;

#[cfg(feature = "tool-shell")]
use super::process_exec;

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
        let approval_key =
            crate::tools::shell_policy_ext::shell_exec_approval_key_for_normalized_command(
                basename,
            );
        let approved_by_internal_context =
            crate::tools::shell_policy_ext::shell_exec_matches_trusted_internal_approval(
                payload,
                approval_key.as_str(),
            );
        if !explicitly_allowed && !default_allows && !approved_by_internal_context {
            return Err(format!(
                "policy_denied: shell command `{basename}` is not in the allow list (default-deny policy)"
            ));
        }

        let output = run_shell_async(run_shell_command_with_timeout(
            normalized_command.as_str(),
            &args,
            cwd.as_path(),
            timeout_ms,
        ))??;

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
fn parse_shell_timeout_ms(payload: &serde_json::Map<String, Value>) -> Result<u64, String> {
    let timeout_ms = match payload.get("timeout_ms") {
        Some(timeout_ms) => timeout_ms
            .as_u64()
            .ok_or_else(|| "shell.exec payload.timeout_ms must be an integer".to_owned())?,
        None => process_exec::DEFAULT_TIMEOUT_MS,
    };

    Ok(timeout_ms.clamp(1_000, process_exec::MAX_TIMEOUT_MS))
}

#[cfg(feature = "tool-shell")]
fn run_shell_async<F>(future: F) -> Result<F::Output, String>
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    process_exec::run_tool_async(future, "shell tool")
}

#[cfg(feature = "tool-shell")]
async fn run_shell_command_with_timeout(
    command: &str,
    args: &[String],
    cwd: &std::path::Path,
    timeout_ms: u64,
) -> Result<std::process::Output, String> {
    process_exec::run_process_with_timeout(command, args, cwd, timeout_ms, "shell command").await
}
