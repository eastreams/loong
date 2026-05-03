use std::process::Stdio;
use std::time::{Duration, Instant};

use loong_kernel as kernel;
use loong_protocol::{JsonLineTransport, OutboundFrame, Transport, TransportInfo};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

use crate::http_json::{BridgeExecutionFailure, BridgeExecutionSuccess};
use crate::policy::{BridgeExecutionPolicy, is_process_command_allowed, parse_process_args};
use crate::protocol::{
    ConnectorProtocolContext, ProcessStdioRuntimeEvidenceKind,
    authorize_connector_protocol_context, parse_process_timeout_ms, process_stdio_runtime_evidence,
};

const DEFAULT_PROCESS_STDIO_ENTRYPOINT_HINT: &str = "stdin/stdout::invoke";
const MAX_STDERR_BYTES: usize = 64 * 1024;
const STDERR_READ_CHUNK_BYTES: usize = 4 * 1024;

pub struct ProcessStdioExchangeOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_json: Value,
    pub response_method: String,
    pub response_id: Option<String>,
}

pub async fn execute_process_stdio_bridge_call(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &kernel::ConnectorCommand,
    runtime_policy: &BridgeExecutionPolicy,
) -> Result<BridgeExecutionSuccess, BridgeExecutionFailure> {
    if !runtime_policy.execute_process_stdio {
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason: "process_stdio execution is disabled by runtime policy".to_owned(),
            runtime_evidence: Value::Null,
        });
    }

    let program = resolved_process_stdio_program(provider);
    let Some(program) = program else {
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason: "process_stdio execution requires provider metadata.command or provider metadata.entrypoint".to_owned(),
            runtime_evidence: Value::Null,
        });
    };

    let command_allowed =
        is_process_command_allowed(&program, &runtime_policy.allowed_process_commands);
    if !command_allowed {
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason: format!("process command {program} is not allowed by runtime policy"),
            runtime_evidence: Value::Null,
        });
    }

    let args = parse_process_args(provider);
    let timeout_ms = parse_process_timeout_ms(provider);
    let envelope = json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    });
    let mut protocol_context =
        ConnectorProtocolContext::from_connector_command(provider, channel, command);
    let authorized = authorize_connector_protocol_context(&mut protocol_context);
    if let Err(reason) = authorized {
        let reason = format!("process_stdio {reason}");
        let execution_tier = runtime_policy.process_stdio_execution_security_tier();
        let runtime_evidence = process_stdio_runtime_evidence(
            &protocol_context,
            execution_tier,
            &program,
            &args,
            timeout_ms,
            ProcessStdioRuntimeEvidenceKind::BaseOnly,
        );
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason,
            runtime_evidence,
        });
    }

    let outbound_frame = protocol_context.outbound_frame(envelope);
    let exchange_result =
        run_process_stdio_json_line_exchange(&program, &args, timeout_ms, outbound_frame).await;

    match exchange_result {
        Ok(outcome) => {
            let execution_tier = runtime_policy.process_stdio_execution_security_tier();
            let response_payload = outcome.stdout_json.clone();
            let runtime_evidence = process_stdio_runtime_evidence(
                &protocol_context,
                execution_tier,
                &program,
                &args,
                timeout_ms,
                ProcessStdioRuntimeEvidenceKind::Execution {
                    exit_code: outcome.exit_code,
                    stdout: outcome.stdout.clone(),
                    stderr: outcome.stderr.clone(),
                    stdout_json: outcome.stdout_json,
                    response_method: outcome.response_method,
                    response_id: outcome.response_id,
                },
            );
            if !outcome.success {
                let reason = format!("process command exited with code {:?}", outcome.exit_code,);
                return Err(BridgeExecutionFailure {
                    blocked: false,
                    reason,
                    runtime_evidence,
                });
            }

            Ok(BridgeExecutionSuccess {
                response_payload,
                runtime_evidence,
            })
        }
        Err(reason) => {
            let execution_tier = runtime_policy.process_stdio_execution_security_tier();
            let runtime_evidence = process_stdio_runtime_evidence(
                &protocol_context,
                execution_tier,
                &program,
                &args,
                timeout_ms,
                ProcessStdioRuntimeEvidenceKind::BaseOnly,
            );
            Err(BridgeExecutionFailure {
                blocked: false,
                reason,
                runtime_evidence,
            })
        }
    }
}

pub async fn run_process_stdio_json_line_exchange(
    program: &str,
    args: &[String],
    timeout_ms: u64,
    frame: OutboundFrame,
) -> Result<ProcessStdioExchangeOutcome, String> {
    let sanitized_env = loong_contracts::sanitized_child_process_env();
    let mut process = TokioCommand::new(program);

    process.env_clear();
    process.envs(sanitized_env);
    process.args(args);
    process.stdin(Stdio::piped());
    process.stdout(Stdio::piped());
    process.stderr(Stdio::piped());

    let mut child = process
        .spawn()
        .map_err(|error| format!("failed to spawn process command {program}: {error}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("process command {program} stdin is not piped"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("process command {program} stdout is not piped"))?;
    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut stderr_pipe) = stderr {
            loop {
                if bytes.len() >= MAX_STDERR_BYTES {
                    break;
                }

                let mut chunk = [0_u8; STDERR_READ_CHUNK_BYTES];
                let read_result = stderr_pipe.read(&mut chunk).await;
                let read = match read_result {
                    Ok(read) => read,
                    Err(_) => break,
                };
                if read == 0 {
                    break;
                }

                let remaining_capacity = MAX_STDERR_BYTES.saturating_sub(bytes.len());
                let bytes_to_take = remaining_capacity.min(read);
                let chunk_slice = chunk.get(..bytes_to_take);
                let Some(chunk_slice) = chunk_slice else {
                    break;
                };
                bytes.extend_from_slice(chunk_slice);
            }
        }
        bytes
    });

    let transport_info = TransportInfo {
        name: format!("process_stdio/{program}"),
        version: "0.1.0".to_owned(),
        secure: false,
    };
    let transport = JsonLineTransport::new(transport_info, stdout, stdin);

    let expected_method = frame.method.clone();
    let expected_id = frame.id.clone();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    let send_timeout =
        match remaining_phase_timeout(deadline, timeout_ms, "process_stdio transport send") {
            Ok(send_timeout) => send_timeout,
            Err(error) => {
                cleanup_process_stdio_child(&mut child, stderr_task).await;
                return Err(error);
            }
        };
    let send_result = timeout(send_timeout, transport.send(frame)).await;
    let send_result = match send_result {
        Ok(send_result) => send_result,
        Err(_error) => {
            cleanup_process_stdio_child(&mut child, stderr_task).await;
            return Err(format!(
                "process_stdio transport send timed out after {timeout_ms}ms",
            ));
        }
    };
    if let Err(error) = send_result {
        cleanup_process_stdio_child(&mut child, stderr_task).await;
        return Err(format!("process_stdio transport send failed: {error}"));
    }

    let close_timeout =
        match remaining_phase_timeout(deadline, timeout_ms, "process_stdio transport close") {
            Ok(close_timeout) => close_timeout,
            Err(error) => {
                cleanup_process_stdio_child(&mut child, stderr_task).await;
                return Err(error);
            }
        };
    let close_result = timeout(close_timeout, transport.close()).await;
    let close_result = match close_result {
        Ok(close_result) => close_result,
        Err(_error) => {
            cleanup_process_stdio_child(&mut child, stderr_task).await;
            return Err(format!(
                "process_stdio transport close timed out after {timeout_ms}ms",
            ));
        }
    };
    if let Err(error) = close_result {
        cleanup_process_stdio_child(&mut child, stderr_task).await;
        return Err(format!("process_stdio transport close failed: {error}"));
    }

    let recv_timeout =
        match remaining_phase_timeout(deadline, timeout_ms, "process_stdio transport recv") {
            Ok(recv_timeout) => recv_timeout,
            Err(error) => {
                cleanup_process_stdio_child(&mut child, stderr_task).await;
                return Err(error);
            }
        };
    let response = match timeout(recv_timeout, transport.recv()).await {
        Ok(Ok(Some(frame))) => frame,
        Ok(Ok(None)) => {
            drop(transport);
            cleanup_process_stdio_child(&mut child, stderr_task).await;
            return Err("process_stdio transport closed before response frame".to_owned());
        }
        Ok(Err(error)) => {
            cleanup_process_stdio_child(&mut child, stderr_task).await;
            return Err(format!("process_stdio transport recv failed: {error}"));
        }
        Err(_) => {
            cleanup_process_stdio_child(&mut child, stderr_task).await;
            return Err(format!(
                "process_stdio transport recv timed out after {timeout_ms}ms",
            ));
        }
    };

    let response_method_matches = response.method == expected_method;
    if !response_method_matches {
        cleanup_process_stdio_child(&mut child, stderr_task).await;
        return Err(format!(
            "process_stdio response method mismatch: expected `{expected_method}`, got `{}`",
            response.method,
        ));
    }

    let response_id_matches = response.id == expected_id;
    if !response_id_matches {
        cleanup_process_stdio_child(&mut child, stderr_task).await;
        return Err(format!(
            "process_stdio response id mismatch: expected `{:?}`, got `{:?}`",
            expected_id, response.id,
        ));
    }

    drop(transport);
    let wait_timeout = match remaining_phase_timeout(deadline, timeout_ms, "process command") {
        Ok(wait_timeout) => wait_timeout,
        Err(error) => {
            cleanup_process_stdio_child(&mut child, stderr_task).await;
            return Err(error);
        }
    };
    let status = timeout(wait_timeout, child.wait()).await;
    let status = match status {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            let _ = stderr_task.await;
            return Err(format!("failed to wait process output: {error}"));
        }
        Err(_) => {
            cleanup_process_stdio_child(&mut child, stderr_task).await;
            return Err(format!(
                "process command timed out after {timeout_ms}ms waiting for exit",
            ));
        }
    };

    let stderr_bytes = stderr_task
        .await
        .map_err(|error| format!("failed to collect process stderr: {error}"))?;
    let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_owned();
    let stdout_json = response.payload;
    let stdout = serde_json::to_string(&stdout_json).unwrap_or_else(|_| "null".to_owned());

    Ok(ProcessStdioExchangeOutcome {
        success: status.success(),
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_json,
        response_method: response.method,
        response_id: response.id,
    })
}

async fn cleanup_process_stdio_child(
    child: &mut tokio::process::Child,
    stderr_task: tokio::task::JoinHandle<Vec<u8>>,
) {
    let _ = child.start_kill();
    let _ = child.wait().await;
    let _ = stderr_task.await;
}

fn resolved_process_stdio_program(provider: &kernel::ProviderConfig) -> Option<String> {
    let command = non_empty_provider_metadata_value(provider, "command");
    if command.is_some() {
        return command.map(|command| resolve_relative_process_path(provider, command));
    }

    let entrypoint = non_empty_provider_metadata_value(provider, "entrypoint");
    if let Some(entrypoint) = entrypoint
        && entrypoint != DEFAULT_PROCESS_STDIO_ENTRYPOINT_HINT
    {
        return Some(resolve_relative_process_path(provider, entrypoint));
    }

    let entrypoint_hint = non_empty_provider_metadata_value(provider, "entrypoint_hint");
    if let Some(entrypoint_hint) = entrypoint_hint
        && entrypoint_hint != DEFAULT_PROCESS_STDIO_ENTRYPOINT_HINT
    {
        return Some(resolve_relative_process_path(provider, entrypoint_hint));
    }

    None
}

fn resolve_relative_process_path(provider: &kernel::ProviderConfig, raw: String) -> String {
    let package_root = provider
        .metadata
        .get("plugin_package_root")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(package_root) = package_root else {
        return raw;
    };

    let path = std::path::Path::new(raw.as_str());
    if path.is_absolute() {
        return raw;
    }
    if raw.starts_with('-') {
        return raw;
    }

    let resolved = std::path::Path::new(package_root).join(path);
    if !resolved.exists() {
        return raw;
    }

    resolved.display().to_string()
}

fn non_empty_provider_metadata_value(
    provider: &kernel::ProviderConfig,
    key: &str,
) -> Option<String> {
    let value = provider.metadata.get(key)?;
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return None;
    }

    Some(trimmed_value.to_owned())
}

fn remaining_phase_timeout(
    deadline: Instant,
    timeout_ms: u64,
    phase: &str,
) -> Result<Duration, String> {
    let remaining_timeout = deadline.saturating_duration_since(Instant::now());
    if remaining_timeout.is_zero() {
        return Err(format!("{phase} timed out after {timeout_ms}ms"));
    }

    Ok(remaining_timeout)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn execute_process_stdio_bridge_call_blocks_when_policy_disables_execution() {
        let provider = kernel::ProviderConfig {
            provider_id: "stdio-provider".to_owned(),
            connector_name: "stdio-provider".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([("command".to_owned(), "cat".to_owned())]),
        };
        let channel = kernel::ChannelConfig {
            channel_id: "primary".to_owned(),
            provider_id: "stdio-provider".to_owned(),
            endpoint: "local://stdio-provider".to_owned(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let command = kernel::ConnectorCommand {
            connector_name: "stdio-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([kernel::Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
        };
        let runtime_policy = BridgeExecutionPolicy {
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: BTreeSet::from(["cat".to_owned()]),
        };

        let failure =
            execute_process_stdio_bridge_call(&provider, &channel, &command, &runtime_policy)
                .await
                .expect_err("policy-disabled process bridge should be blocked");

        assert!(failure.blocked);
        assert_eq!(
            failure.reason,
            "process_stdio execution is disabled by runtime policy",
        );
    }
}
