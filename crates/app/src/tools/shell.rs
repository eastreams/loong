#[cfg(feature = "tool-shell")]
use std::{path::PathBuf, process::Command};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-shell")]
use serde_json::{Value, json};

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

        let normalized_command = command.to_ascii_lowercase();

        // Reject commands with embedded whitespace to ensure the policy check
        // and execution operate on the same value.
        if normalized_command.contains(char::is_whitespace) {
            return Err(
                "policy_denied: shell command must not contain embedded whitespace; use `args` instead"
                    .to_owned(),
            );
        }

        let basename = &normalized_command;

        // Reject path-qualified commands (e.g. /tmp/git, ./git, ..\git) to
        // prevent allowlist bypass via an absolute or relative path.
        if basename.contains('/') || basename.contains('\\') {
            return Err(format!(
                "policy_denied: shell command `{basename}` must be a bare command name without path separators"
            ));
        }

        if config.shell_deny.contains(basename.as_str()) {
            return Err(format!(
                "policy_denied: shell command `{basename}` is blocked by shell policy"
            ));
        }

        let explicitly_allowed = config.shell_allow.contains(basename.as_str());
        let default_allows = matches!(
            config.shell_default_mode,
            crate::tools::shell_policy_ext::ShellPolicyDefault::Allow
        );
        if !explicitly_allowed && !default_allows {
            return Err(format!(
                "policy_denied: shell command `{basename}` is not in the allow list (default-deny policy)"
            ));
        }

        let output = Command::new(&normalized_command)
            .args(&args)
            .current_dir(&cwd)
            .output()
            .map_err(|error| format!("shell command spawn failed: {error}"))?;

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
