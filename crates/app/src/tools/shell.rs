#[cfg(feature = "tool-shell")]
use std::{collections::BTreeSet, path::PathBuf, process::Command};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-shell")]
use serde_json::{json, Value};

pub(super) fn execute_shell_tool(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-shell"))]
    {
        let _ = request;
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

        let allowlist = shell_allowlist();
        let normalized_command = command.to_ascii_lowercase();
        if !allowlist.contains(&normalized_command) {
            return Err(format!(
                "shell command `{command}` is not allowed (allowlist={})",
                allowlist.iter().cloned().collect::<Vec<_>>().join(",")
            ));
        }

        let output = Command::new(command)
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

#[cfg(feature = "tool-shell")]
fn shell_allowlist() -> BTreeSet<String> {
    let from_env = std::env::var("LOONGCLAW_SHELL_ALLOWLIST")
        .ok()
        .unwrap_or_else(|| "echo,cat,ls,pwd".to_owned());
    from_env
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}
