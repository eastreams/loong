use std::io::ErrorKind;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;
use tokio::time::timeout;

use crate::CliResult;

const ACPX_MCP_PROXY_NODE_COMMAND: &str = "node";
const ACPX_MCP_PROXY_SCRIPT_NAME: &str = "loongclaw-acpx-mcp-proxy.mjs";
const ACPX_MCP_PROXY_SCRIPT_SOURCE: &str = include_str!("assets/acpx-mcp-proxy.mjs");
static ACPX_MCP_PROXY_SCRIPT_PATH: OnceLock<Result<String, String>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AcpxMcpServerEntry {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<AcpxMcpServerEnvEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AcpxMcpServerEnvEntry {
    pub(crate) name: String,
    pub(crate) value: String,
}

pub(crate) fn build_mcp_proxy_agent_command(
    target_command: &str,
    mcp_servers: &[AcpxMcpServerEntry],
) -> CliResult<String> {
    let script_path = ensure_mcp_proxy_script_path()?;
    let payload = serde_json::to_vec(&json!({
        "targetCommand": target_command,
        "mcpServers": mcp_servers,
    }))
    .map_err(|error| format!("serialize ACPX MCP proxy payload failed: {error}"))?;
    let payload_path = materialize_mcp_proxy_payload_path(payload.as_slice())?;
    let command_parts = vec![
        ACPX_MCP_PROXY_NODE_COMMAND.to_owned(),
        script_path,
        "--payload-file".to_owned(),
        payload_path,
    ];
    let command_line = join_command_line(command_parts.as_slice());
    Ok(command_line)
}

pub(crate) async fn probe_mcp_proxy_support(
    cwd: Option<&str>,
    timeout_duration: Duration,
) -> CliResult<(String, String)> {
    let script_path = ensure_mcp_proxy_script_path()?;
    probe_mcp_proxy_support_with_runtime(
        ACPX_MCP_PROXY_NODE_COMMAND,
        script_path.as_str(),
        cwd,
        timeout_duration,
    )
    .await
}

pub(crate) async fn probe_mcp_proxy_support_with_runtime(
    node_command: &str,
    script_path: &str,
    cwd: Option<&str>,
    timeout_duration: Duration,
) -> CliResult<(String, String)> {
    let mut probe = Command::new(node_command);
    probe.arg(script_path);
    probe.arg("--version");

    if let Some(cwd) = cwd {
        probe.current_dir(cwd);
    }

    let timed_output = timeout(timeout_duration, probe.output())
        .await
        .map_err(|_timeout_error| "embedded ACPX MCP proxy runtime probe timed out".to_owned())?;
    let output = timed_output.map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            format!("embedded ACPX MCP proxy requires `{node_command}` on PATH")
        } else {
            format!("probe embedded ACPX MCP proxy runtime failed: {error}")
        }
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let observed = observed_command_output(stdout.as_str(), stderr.as_str());

    if !output.status.success() {
        let exit_code = output
            .status
            .code()
            .map_or_else(|| "unknown".to_owned(), |code| code.to_string());
        let message = format!(
            "embedded ACPX MCP proxy runtime probe exited with code {exit_code}: {observed}"
        );
        return Err(message);
    }

    let script_path = script_path.to_owned();
    Ok((script_path, observed))
}

fn observed_command_output(stdout: &str, stderr: &str) -> String {
    let stdout_empty = stdout.is_empty();
    let stderr_empty = stderr.is_empty();

    if !stdout_empty && stderr_empty {
        return stdout.to_owned();
    }

    if stdout_empty && !stderr_empty {
        return stderr.to_owned();
    }

    if !stdout_empty && !stderr_empty {
        return format!("{stdout} | {stderr}");
    }

    "(empty)".to_owned()
}

fn ensure_mcp_proxy_script_path() -> CliResult<String> {
    ACPX_MCP_PROXY_SCRIPT_PATH
        .get_or_init(materialize_mcp_proxy_script)
        .clone()
}

fn materialize_mcp_proxy_script() -> Result<String, String> {
    let temp_dir = std::env::temp_dir();
    let loongclaw_dir = temp_dir.join("loongclaw");
    let path = loongclaw_dir.join(ACPX_MCP_PROXY_SCRIPT_NAME);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create ACPX MCP proxy directory failed: {error}"))?;
    }

    std::fs::write(&path, ACPX_MCP_PROXY_SCRIPT_SOURCE)
        .map_err(|error| format!("write ACPX MCP proxy script failed: {error}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("stat ACPX MCP proxy script failed: {error}"))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .map_err(|error| format!("chmod ACPX MCP proxy script failed: {error}"))?;
    }

    let script_path = path.display().to_string();
    Ok(script_path)
}

fn materialize_mcp_proxy_payload_path(payload: &[u8]) -> CliResult<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("read system time for ACPX MCP payload failed: {error}"))?;
    let process_id = std::process::id();
    let payload_file_name = format!(
        "acpx-mcp-payload-{process_id}-{}.json",
        timestamp.as_nanos()
    );
    let temp_dir = std::env::temp_dir();
    let loongclaw_dir = temp_dir.join("loongclaw");
    let payload_dir = loongclaw_dir.join("acpx-mcp-payloads");
    let payload_path = payload_dir.join(payload_file_name);

    if let Some(parent) = payload_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create ACPX MCP payload directory failed: {error}"))?;
    }

    std::fs::write(&payload_path, payload)
        .map_err(|error| format!("write ACPX MCP payload failed: {error}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = std::fs::metadata(&payload_path)
            .map_err(|error| format!("stat ACPX MCP payload failed: {error}"))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(&payload_path, permissions)
            .map_err(|error| format!("chmod ACPX MCP payload failed: {error}"))?;
    }

    let payload_path = payload_path.display().to_string();
    Ok(payload_path)
}

fn join_command_line(parts: &[String]) -> String {
    let quoted_parts = parts
        .iter()
        .map(|part| quote_command_part(part.as_str()))
        .collect::<Vec<_>>();
    quoted_parts.join(" ")
}

fn quote_command_part(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }

    let is_simple = value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_./:@".contains(character));
    if is_simple {
        return value.to_owned();
    }

    let escaped_backslashes = value.replace('\\', "\\\\");
    let escaped_value = escaped_backslashes.replace('"', "\\\"");
    let quoted_value = format!("\"{escaped_value}\"");
    quoted_value
}
