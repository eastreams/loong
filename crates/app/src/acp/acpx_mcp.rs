use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::CliResult;

const ACPX_MCP_PROXY_NODE_COMMAND: &str = "node";
const ACPX_MCP_PROXY_SCRIPT_STEM: &str = "loongclaw-acpx-mcp-proxy";
const ACPX_MCP_PROXY_SCRIPT_EXTENSION: &str = "mjs";
const ACPX_MCP_PROXY_SCRIPT_HASH_PREFIX_LENGTH: usize = 12;
const ACPX_MCP_PROXY_SCRIPT_SOURCE: &str = include_str!("assets/acpx-mcp-proxy.mjs");
const LOONGCLAW_TEMP_DIR_NAME: &str = "loongclaw";
const ACPX_MCP_PAYLOAD_DIR_NAME: &str = "acpx-mcp-payloads";
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
    probe.kill_on_drop(true);
    probe.stdout(Stdio::piped());
    probe.stderr(Stdio::piped());

    if let Some(cwd) = cwd {
        probe.current_dir(cwd);
    }

    let mut child = probe.spawn().map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            format!("embedded ACPX MCP proxy requires `{node_command}` on PATH")
        } else {
            format!("probe embedded ACPX MCP proxy runtime failed: {error}")
        }
    })?;
    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "capture embedded ACPX MCP proxy stdout failed".to_owned())?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "capture embedded ACPX MCP proxy stderr failed".to_owned())?;
    let stdout_task = tokio::spawn(read_probe_pipe(stdout_pipe, "stdout"));
    let stderr_task = tokio::spawn(read_probe_pipe(stderr_pipe, "stderr"));

    let output_status = timeout(timeout_duration, child.wait())
        .await
        .map_err(|_timeout_error| "embedded ACPX MCP proxy runtime probe timed out".to_owned());

    let output_status = match output_status {
        Ok(wait_result) => wait_result
            .map_err(|error| format!("wait for embedded ACPX MCP proxy runtime failed: {error}"))?,
        Err(timeout_message) => {
            terminate_probe_child_process(&mut child).await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(timeout_message);
        }
    };
    let stdout_bytes = await_probe_pipe(stdout_task, "stdout").await?;
    let stderr_bytes = await_probe_pipe(stderr_task, "stderr").await?;

    let stdout = String::from_utf8_lossy(stdout_bytes.as_slice())
        .trim()
        .to_owned();
    let stderr = String::from_utf8_lossy(stderr_bytes.as_slice())
        .trim()
        .to_owned();
    let observed = observed_command_output(stdout.as_str(), stderr.as_str());

    if !output_status.success() {
        let exit_code = output_status
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
    let loongclaw_dir = loongclaw_temp_dir();
    ensure_private_directory(loongclaw_dir.as_path(), "ACPX MCP proxy directory")?;

    let script_file_name = versioned_mcp_proxy_script_file_name();
    let path = loongclaw_dir.join(script_file_name);
    let path_exists = path.exists();

    if !path_exists {
        persist_named_file(
            path.as_path(),
            ACPX_MCP_PROXY_SCRIPT_SOURCE.as_bytes(),
            0o755,
            "ACPX MCP proxy script",
        )?;
    }
    set_path_permissions(path.as_path(), 0o755, "ACPX MCP proxy script")?;

    let script_path = path.display().to_string();
    Ok(script_path)
}

fn materialize_mcp_proxy_payload_path(payload: &[u8]) -> CliResult<String> {
    let loongclaw_dir = loongclaw_temp_dir();
    ensure_private_directory(loongclaw_dir.as_path(), "ACPX MCP payload root directory")?;
    let payload_dir = loongclaw_dir.join(ACPX_MCP_PAYLOAD_DIR_NAME);
    ensure_private_directory(payload_dir.as_path(), "ACPX MCP payload directory")?;

    let mut payload_file = tempfile::Builder::new()
        .prefix("acpx-mcp-payload-")
        .suffix(".json")
        .tempfile_in(payload_dir.as_path())
        .map_err(|error| format!("create ACPX MCP payload temp file failed: {error}"))?;
    write_temp_file_bytes(&mut payload_file, payload, "ACPX MCP payload")?;
    set_path_permissions(payload_file.path(), 0o600, "ACPX MCP payload")?;

    let keep_result = payload_file.keep();
    let (_file, payload_path) =
        keep_result.map_err(|error| format!("persist ACPX MCP payload failed: {}", error.error))?;
    let payload_path = payload_path.display().to_string();
    Ok(payload_path)
}

fn loongclaw_temp_dir() -> PathBuf {
    let temp_dir = std::env::temp_dir();
    temp_dir.join(LOONGCLAW_TEMP_DIR_NAME)
}

fn versioned_mcp_proxy_script_file_name() -> String {
    let digest = Sha256::digest(ACPX_MCP_PROXY_SCRIPT_SOURCE.as_bytes());
    let digest_hex = hex::encode(digest);
    let digest_prefix = digest_hex
        .chars()
        .take(ACPX_MCP_PROXY_SCRIPT_HASH_PREFIX_LENGTH)
        .collect::<String>();
    let file_name =
        format!("{ACPX_MCP_PROXY_SCRIPT_STEM}-{digest_prefix}.{ACPX_MCP_PROXY_SCRIPT_EXTENSION}");
    file_name
}

fn ensure_private_directory(path: &Path, subject: &str) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| format!("create {subject} failed: {error}"))?;
    set_path_permissions(path, 0o700, subject)?;
    Ok(())
}

fn persist_named_file(
    target_path: &Path,
    contents: &[u8],
    mode: u32,
    subject: &str,
) -> Result<(), String> {
    let parent_dir = target_path.parent().ok_or_else(|| {
        format!(
            "{subject} path `{}` has no parent directory",
            target_path.display()
        )
    })?;
    let mut staged_file = NamedTempFile::new_in(parent_dir)
        .map_err(|error| format!("create staged {subject} failed: {error}"))?;

    write_temp_file_bytes(&mut staged_file, contents, subject)?;
    set_path_permissions(staged_file.path(), mode, subject)?;

    let persist_result = staged_file.persist_noclobber(target_path);

    match persist_result {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(format!("persist {subject} failed: {}", error.error)),
    }
}

fn write_temp_file_bytes(
    temp_file: &mut NamedTempFile,
    contents: &[u8],
    subject: &str,
) -> Result<(), String> {
    use std::io::Write;

    temp_file
        .write_all(contents)
        .map_err(|error| format!("write {subject} failed: {error}"))?;
    temp_file
        .flush()
        .map_err(|error| format!("flush {subject} failed: {error}"))?;
    temp_file
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync {subject} failed: {error}"))?;
    Ok(())
}

#[cfg(unix)]
fn set_path_permissions(path: &Path, mode: u32, subject: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let metadata =
        std::fs::metadata(path).map_err(|error| format!("stat {subject} failed: {error}"))?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| format!("chmod {subject} failed: {error}"))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_path_permissions(_path: &Path, _mode: u32, _subject: &str) -> Result<(), String> {
    Ok(())
}

async fn read_probe_pipe<T>(mut pipe: T, stream_name: &str) -> Result<Vec<u8>, String>
where
    T: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("read embedded ACPX MCP proxy {stream_name} failed: {error}"))?;
    Ok(bytes)
}

async fn await_probe_pipe(
    task: tokio::task::JoinHandle<Result<Vec<u8>, String>>,
    stream_name: &str,
) -> Result<Vec<u8>, String> {
    task.await.map_err(|error| {
        format!("join embedded ACPX MCP proxy {stream_name} reader failed: {error}")
    })?
}

async fn terminate_probe_child_process(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
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
