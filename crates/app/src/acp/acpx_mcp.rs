use std::io::ErrorKind;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;

use crate::CliResult;
use crate::config::LoongClawConfig;
use crate::mcp::{McpRegistry, McpStdioServerLaunchSpec};

use super::backend::AcpSessionBootstrap;

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
    #[serde(default)]
    pub(crate) cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AcpxMcpServerEnvEntry {
    pub(crate) name: String,
    pub(crate) value: String,
}

pub(crate) fn validate_requested_mcp_servers(
    config: &LoongClawConfig,
    request: &AcpSessionBootstrap,
) -> CliResult<Vec<String>> {
    if request.mcp_servers.is_empty() {
        return Ok(Vec::new());
    }
    if !config.acp.allow_mcp_server_injection {
        return Err(
            "ACPX bootstrap requested MCP server injection but acp.allow_mcp_server_injection=false"
                .to_owned(),
        );
    }

    let registry = McpRegistry::from_config(config)?;
    let selected = registry.resolve_selected_server_names(&request.mcp_servers)?;
    let _launch_specs = registry.resolve_injectable_stdio_launch_specs(&selected)?;

    Ok(selected)
}

pub(crate) fn injectable_mcp_server_count(config: &LoongClawConfig) -> CliResult<usize> {
    let registry = McpRegistry::from_config(config)?;
    let count = registry.injectable_stdio_server_count();
    Ok(count)
}

pub(crate) fn build_mcp_proxy_agent_command_for_selection(
    config: &LoongClawConfig,
    target_command: &str,
    selected_mcp_servers: &[String],
) -> CliResult<String> {
    let mcp_servers = resolve_selected_mcp_server_entries(config, selected_mcp_servers)?;
    build_mcp_proxy_agent_command(target_command, &mcp_servers)
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
    Ok(join_command_line(&[
        ACPX_MCP_PROXY_NODE_COMMAND.to_owned(),
        script_path,
        "--payload-file".to_owned(),
        payload_path,
    ]))
}

pub(crate) async fn probe_mcp_proxy_support(cwd: Option<&str>) -> CliResult<(String, String)> {
    let script_path = ensure_mcp_proxy_script_path()?;
    let mut probe = Command::new(ACPX_MCP_PROXY_NODE_COMMAND);
    probe.arg(script_path.as_str());
    probe.arg("--version");
    if let Some(cwd) = cwd {
        probe.current_dir(cwd);
    }
    let output = probe.output().await.map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            format!("embedded ACPX MCP proxy requires `{ACPX_MCP_PROXY_NODE_COMMAND}` on PATH")
        } else {
            format!("probe embedded ACPX MCP proxy runtime failed: {error}")
        }
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let observed = match (stdout.is_empty(), stderr.is_empty()) {
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout} | {stderr}"),
        (true, true) => "(empty)".to_owned(),
    };
    if !output.status.success() {
        return Err(format!(
            "embedded ACPX MCP proxy runtime probe exited with code {}: {observed}",
            output
                .status
                .code()
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string())
        ));
    }
    Ok((script_path, observed))
}

fn resolve_selected_mcp_server_entries(
    config: &LoongClawConfig,
    selected_mcp_servers: &[String],
) -> CliResult<Vec<AcpxMcpServerEntry>> {
    let registry = McpRegistry::from_config(config)?;
    let resolved_servers = registry.resolve_injectable_stdio_launch_specs(selected_mcp_servers)?;

    let mut entries = Vec::new();
    for server in resolved_servers {
        let entry = acpx_mcp_server_entry_from_registry_server(server);
        entries.push(entry);
    }

    Ok(entries)
}

fn acpx_mcp_server_entry_from_registry_server(
    server: McpStdioServerLaunchSpec,
) -> AcpxMcpServerEntry {
    let mut env_entries = Vec::new();
    for (name, value) in server.env {
        let env_entry = AcpxMcpServerEnvEntry { name, value };
        env_entries.push(env_entry);
    }

    AcpxMcpServerEntry {
        name: server.name,
        command: server.command,
        args: server.args,
        env: env_entries,
        cwd: server.cwd,
    }
}

fn ensure_mcp_proxy_script_path() -> CliResult<String> {
    ACPX_MCP_PROXY_SCRIPT_PATH
        .get_or_init(materialize_mcp_proxy_script)
        .clone()
}

fn materialize_mcp_proxy_script() -> Result<String, String> {
    let path = std::env::temp_dir()
        .join("loongclaw")
        .join(ACPX_MCP_PROXY_SCRIPT_NAME);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create ACPX MCP proxy directory failed: {error}"))?;
    }
    std::fs::write(&path, ACPX_MCP_PROXY_SCRIPT_SOURCE)
        .map_err(|error| format!("write ACPX MCP proxy script failed: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&path)
            .map_err(|error| format!("stat ACPX MCP proxy script failed: {error}"))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .map_err(|error| format!("chmod ACPX MCP proxy script failed: {error}"))?;
    }
    Ok(path.display().to_string())
}

fn materialize_mcp_proxy_payload_path(payload: &[u8]) -> CliResult<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("read system time for ACPX MCP payload failed: {error}"))?;
    let payload_file_name = format!(
        "acpx-mcp-payload-{}-{}.json",
        std::process::id(),
        timestamp.as_nanos()
    );
    let payload_path = std::env::temp_dir()
        .join("loongclaw")
        .join("acpx-mcp-payloads")
        .join(payload_file_name);
    if let Some(parent) = payload_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create ACPX MCP payload directory failed: {error}"))?;
    }
    std::fs::write(&payload_path, payload)
        .map_err(|error| format!("write ACPX MCP payload failed: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&payload_path)
            .map_err(|error| format!("stat ACPX MCP payload failed: {error}"))?
            .permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(&payload_path, permissions)
            .map_err(|error| format!("chmod ACPX MCP payload failed: {error}"))?;
    }
    Ok(payload_path.display().to_string())
}

fn join_command_line(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| quote_command_part(part.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_command_part(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_./:@".contains(character))
    {
        return value.to_owned();
    }

    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
