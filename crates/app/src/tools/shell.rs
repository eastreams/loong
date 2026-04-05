#[cfg(feature = "tool-shell")]
use super::process_exec;
use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-shell")]
use serde_json::{Value, json};
#[cfg(feature = "tool-shell")]
use std::path::{Path, PathBuf};

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
        let cwd = resolve_shell_cwd_with_config(payload, config)?;
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
fn resolve_shell_cwd_with_config(
    payload: &serde_json::Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    let raw_cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let default_cwd = config.default_working_directory();
    let resolved_cwd = match raw_cwd {
        Some(raw_cwd) => resolve_shell_cwd_override(raw_cwd, config)?,
        None => default_cwd,
    };
    if !resolved_cwd.is_dir() {
        let display_path = resolved_cwd.display();
        let error = format!("shell.exec cwd `{display_path}` is not a directory");
        return Err(error);
    }
    Ok(resolved_cwd)
}

#[cfg(feature = "tool-shell")]
fn resolve_shell_cwd_override(
    raw_cwd: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    if config.file_root.is_some() {
        return super::file::resolve_safe_file_path_with_config(raw_cwd, config);
    }

    let requested_path = PathBuf::from(raw_cwd);
    let base_path = if requested_path.is_absolute() {
        requested_path
    } else {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        current_dir.join(requested_path)
    };
    canonicalize_existing_directory(base_path.as_path())
}

#[cfg(feature = "tool-shell")]
fn canonicalize_existing_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        let display_path = path.display();
        format!("failed to inspect shell cwd `{display_path}`: {error}")
    })?;
    if !metadata.is_dir() {
        let display_path = path.display();
        let error = format!("shell.exec cwd `{display_path}` is not a directory");
        return Err(error);
    }
    std::fs::canonicalize(path).map_err(|error| {
        let display_path = path.display();
        format!("failed to canonicalize shell cwd `{display_path}`: {error}")
    })
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

#[cfg(all(test, feature = "tool-shell"))]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_dir;
    use crate::tools::runtime_config::ToolRuntimeConfig;
    use serde_json::json;

    fn shell_test_config(root: &Path) -> ToolRuntimeConfig {
        ToolRuntimeConfig {
            file_root: Some(root.to_path_buf()),
            shell_allow: ["pwd".to_owned()].into_iter().collect(),
            ..ToolRuntimeConfig::default()
        }
    }

    #[test]
    fn shell_exec_defaults_cwd_to_configured_file_root() {
        let root = unique_temp_dir("loongclaw-shell-default-cwd");
        std::fs::create_dir_all(&root).expect("create shell root");
        let config = shell_test_config(&root);
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "pwd"
            }),
        };

        let outcome =
            execute_shell_tool_with_config(request, &config).expect("shell.exec should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["cwd"], root.display().to_string());
        let stdout = outcome.payload["stdout"]
            .as_str()
            .expect("stdout should be text");
        assert!(
            stdout.ends_with(root.display().to_string().as_str()),
            "expected pwd output to resolve inside file_root, got: {stdout}"
        );
    }

    #[test]
    fn shell_exec_rejects_cwd_that_escapes_configured_file_root() {
        let root = unique_temp_dir("loongclaw-shell-cwd-root");
        let outside = unique_temp_dir("loongclaw-shell-cwd-outside");
        std::fs::create_dir_all(&root).expect("create shell root");
        std::fs::create_dir_all(&outside).expect("create outside dir");
        let config = shell_test_config(&root);
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": outside.display().to_string()
            }),
        };

        let error =
            execute_shell_tool_with_config(request, &config).expect_err("escape should fail");

        assert!(
            error.contains("escapes configured file root"),
            "error: {error}"
        );
    }

    #[test]
    fn shell_exec_rejects_non_directory_cwd() {
        let root = unique_temp_dir("loongclaw-shell-cwd-file");
        std::fs::create_dir_all(&root).expect("create shell root");
        let file_path = root.join("note.txt");
        std::fs::write(&file_path, "hello").expect("write shell cwd file");
        let config = shell_test_config(&root);
        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": "note.txt"
            }),
        };

        let error =
            execute_shell_tool_with_config(request, &config).expect_err("file cwd should fail");

        assert!(error.contains("is not a directory"), "error: {error}");
    }
}
