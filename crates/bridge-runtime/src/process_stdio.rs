use std::process::Stdio;
use std::time::Duration;

use loongclaw_kernel as kernel;
use loongclaw_protocol::{JsonLineTransport, OutboundFrame, Transport, TransportInfo};
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
    let program = provider.metadata.get("command").cloned();
    let Some(program) = program else {
        return Err(BridgeExecutionFailure {
            blocked: true,
            reason: "process_stdio execution requires provider metadata.command".to_owned(),
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
    let sanitized_env = loongclaw_contracts::sanitized_child_process_env();
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
            let _ = stderr_pipe.read_to_end(&mut bytes).await;
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

    let send_result = timeout(Duration::from_millis(timeout_ms), transport.send(frame)).await;
    let send_result = send_result
        .map_err(|_err| format!("process_stdio transport send timed out after {timeout_ms}ms"))?;
    if let Err(error) = send_result {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!("process_stdio transport send failed: {error}"));
    }

    let close_result = timeout(Duration::from_millis(timeout_ms), transport.close()).await;
    let close_result = close_result
        .map_err(|_err| format!("process_stdio transport close timed out after {timeout_ms}ms"))?;
    if let Err(error) = close_result {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!("process_stdio transport close failed: {error}"));
    }

    let response = match timeout(Duration::from_millis(timeout_ms), transport.recv()).await {
        Ok(Ok(Some(frame))) => frame,
        Ok(Ok(None)) => {
            drop(transport);
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err("process_stdio transport closed before response frame".to_owned());
        }
        Ok(Err(error)) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err(format!("process_stdio transport recv failed: {error}"));
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err(format!(
                "process_stdio transport recv timed out after {timeout_ms}ms",
            ));
        }
    };

    let response_method_matches = response.method == expected_method;
    if !response_method_matches {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!(
            "process_stdio response method mismatch: expected `{expected_method}`, got `{}`",
            response.method,
        ));
    }

    let response_id_matches = response.id == expected_id;
    if !response_id_matches {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!(
            "process_stdio response id mismatch: expected `{:?}`, got `{:?}`",
            expected_id, response.id,
        ));
    }

    drop(transport);
    let status = timeout(Duration::from_millis(timeout_ms), child.wait()).await;
    let status = match status {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            let _ = stderr_task.await;
            return Err(format!("failed to wait process output: {error}"));
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stderr_task.await;
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
