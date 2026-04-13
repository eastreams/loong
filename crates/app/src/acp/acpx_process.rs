use std::process::Stdio;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use super::*;
use crate::process_launch::retry_executable_file_busy_async;

#[cfg(test)]
use crate::process_launch::retry_executable_file_busy_blocking as retry_spawn_blocking;

#[derive(Debug, Clone)]
pub(super) struct AcpxCommandOutput {
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) exit_code: Option<i32>,
}

pub(super) async fn run_json_command(
    profile: &ResolvedAcpxProfile,
    args: Vec<String>,
    cwd: &str,
    timeout_ms: u64,
    stdin_payload: Option<&str>,
    ignore_no_session: bool,
) -> CliResult<Vec<Value>> {
    let output = run_process(
        profile.command.as_str(),
        &args,
        cwd,
        timeout_ms,
        stdin_payload,
    )
    .await?;
    let events = parse_json_lines(output.stdout.as_str());
    let error = event_error_message(events.as_slice(), ignore_no_session);
    if let Some(error) = error {
        return Err(error);
    }
    if output.exit_code.is_some_and(|code| code != 0) {
        return Err(format_exit_message(
            output.stderr.as_str(),
            output.exit_code,
        ));
    }
    Ok(events)
}

pub(super) async fn run_prompt_process(
    command: &str,
    args: &[String],
    cwd: &str,
    timeout_ms: u64,
    prompt: &str,
    mut abort: Option<AcpAbortSignal>,
    sink: Option<&dyn AcpTurnEventSink>,
) -> CliResult<AcpTurnResult> {
    if abort.as_ref().is_some_and(AcpAbortSignal::is_aborted) {
        let done = synthetic_done_event(Some(AcpTurnStopReason::Cancelled));
        emit_turn_event(sink, &done)?;
        return Ok(AcpTurnResult {
            output_text: String::new(),
            state: AcpSessionState::Ready,
            usage: None,
            events: vec![done],
            stop_reason: Some(AcpTurnStopReason::Cancelled),
        });
    }

    let abort_enabled = abort.is_some();
    let mut child = spawn_acpx_child(command, args, cwd, true).await?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ACPX prompt process stdout pipe was unavailable".to_owned())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ACPX prompt process stdin pipe was unavailable".to_owned())?;
    let stderr = child.stderr.take();
    let mut stderr_task = stderr.map(|stderr| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buffer = String::new();
            reader
                .read_to_string(&mut buffer)
                .await
                .map_err(|error| format!("read ACPX stderr failed: {error}"))?;
            Ok(buffer)
        })
    });
    let stdout_reader = BufReader::new(stdout);
    let mut lines = stdout_reader.lines();

    if let Err(error) = stdin.write_all(prompt.as_bytes()).await {
        terminate_child_process(&mut child).await;
        abort_stderr_task(&mut stderr_task);
        return Err(format!("write ACPX prompt stdin failed: {error}"));
    }
    if let Err(error) = stdin.write_all(b"\n").await {
        terminate_child_process(&mut child).await;
        abort_stderr_task(&mut stderr_task);
        return Err(format!("write ACPX prompt newline failed: {error}"));
    }
    if let Err(error) = stdin.shutdown().await {
        terminate_child_process(&mut child).await;
        abort_stderr_task(&mut stderr_task);
        return Err(format!("close ACPX prompt stdin failed: {error}"));
    }
    drop(stdin);

    let mut events = Vec::new();
    let read_loop = async {
        loop {
            let next_line = tokio::select! {
                line = lines.next_line() => line.map_err(|error| format!("read ACPX stdout failed: {error}"))?,
                _ = wait_for_abort(&mut abort), if abort_enabled => {
                    terminate_child_process(&mut child).await;
                    abort_stderr_task(&mut stderr_task);
                    let done = synthetic_done_event(Some(AcpTurnStopReason::Cancelled));
                    emit_turn_event(sink, &done)?;
                    return Ok(AcpTurnResult {
                        output_text: String::new(),
                        state: AcpSessionState::Ready,
                        usage: None,
                        events: vec![done],
                        stop_reason: Some(AcpTurnStopReason::Cancelled),
                    });
                }
            };

            let Some(line) = next_line else {
                break;
            };
            let Some(event) = parse_json_line(line.as_str()) else {
                continue;
            };
            emit_turn_event(sink, &event)?;
            events.push(event);
            if events.last().is_some_and(is_done_event) {
                break;
            }
        }

        let wait_result = tokio::select! {
            result = child.wait() => result.map_err(|error| format!("wait for ACPX prompt process failed: {error}"))?,
            _ = wait_for_abort(&mut abort), if abort_enabled => {
                terminate_child_process(&mut child).await;
                abort_stderr_task(&mut stderr_task);
                let done = synthetic_done_event(Some(AcpTurnStopReason::Cancelled));
                emit_turn_event(sink, &done)?;
                return Ok(AcpTurnResult {
                    output_text: collect_output_text(&events),
                    state: AcpSessionState::Ready,
                    usage: collect_usage_update(&events),
                    events,
                    stop_reason: Some(AcpTurnStopReason::Cancelled),
                });
            }
        };
        let stderr = collect_stderr_task(&mut stderr_task).await?;
        let exit_code = wait_result.code();
        if !wait_result.success() {
            return Err(format_exit_message(stderr.as_str(), exit_code));
        }
        if !events.last().is_some_and(is_done_event) {
            let done = synthetic_done_event(Some(AcpTurnStopReason::Completed));
            emit_turn_event(sink, &done)?;
            events.push(done);
        }
        Ok(AcpTurnResult {
            output_text: collect_output_text(&events),
            state: AcpSessionState::Ready,
            usage: collect_usage_update(&events),
            stop_reason: collect_stop_reason(&events).or(Some(AcpTurnStopReason::Completed)),
            events,
        })
    };

    match timeout(std::time::Duration::from_millis(timeout_ms), read_loop).await {
        Ok(result) => result,
        Err(_) => {
            terminate_child_process(&mut child).await;
            abort_stderr_task(&mut stderr_task);
            Err(format!(
                "ACPX prompt process timed out after {timeout_ms} ms"
            ))
        }
    }
}

pub(super) async fn run_process(
    command: &str,
    args: &[String],
    cwd: &str,
    timeout_ms: u64,
    stdin_payload: Option<&str>,
) -> CliResult<AcpxCommandOutput> {
    let mut child = spawn_acpx_child(command, args, cwd, stdin_payload.is_some()).await?;
    let mut stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ACPX process stdout pipe was unavailable".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "ACPX process stderr pipe was unavailable".to_owned())?;

    if let Some(payload) = stdin_payload {
        let mut stdin = stdin
            .take()
            .ok_or_else(|| "ACPX process stdin pipe was unavailable".to_owned())?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|error| format!("write ACPX stdin failed: {error}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|error| format!("close ACPX stdin failed: {error}"))?;
    }

    let stdout_task: tokio::task::JoinHandle<Result<String, String>> = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buffer = String::new();
        reader
            .read_to_string(&mut buffer)
            .await
            .map_err(|error| format!("read ACPX stdout failed: {error}"))?;
        Ok(buffer)
    });
    let stderr_task: tokio::task::JoinHandle<Result<String, String>> = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buffer = String::new();
        reader
            .read_to_string(&mut buffer)
            .await
            .map_err(|error| format!("read ACPX stderr failed: {error}"))?;
        Ok(buffer)
    });

    let wait_result = timeout(std::time::Duration::from_millis(timeout_ms), child.wait())
        .await
        .map_err(|error| format!("ACPX process timed out after {timeout_ms} ms: {error}"))?
        .map_err(|error| format!("wait for ACPX process failed: {error}"))?;
    let stdout = stdout_task
        .await
        .map_err(|error| format!("join ACPX stdout task failed: {error}"))??;
    let stderr = stderr_task
        .await
        .map_err(|error| format!("join ACPX stderr task failed: {error}"))??;

    Ok(AcpxCommandOutput {
        stdout,
        stderr,
        exit_code: wait_result.code(),
    })
}

pub(super) async fn collect_stderr_task(
    task: &mut Option<tokio::task::JoinHandle<Result<String, String>>>,
) -> CliResult<String> {
    let Some(handle) = task.take() else {
        return Ok(String::new());
    };
    handle
        .await
        .map_err(|error| format!("join ACPX stderr task failed: {error}"))?
}

pub(super) fn abort_stderr_task(
    task: &mut Option<tokio::task::JoinHandle<Result<String, String>>>,
) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

pub(super) async fn terminate_child_process(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

pub(super) async fn wait_for_abort(abort: &mut Option<AcpAbortSignal>) {
    if let Some(signal) = abort {
        signal.cancelled().await;
    }
}

pub(super) fn synthetic_done_event(stop_reason: Option<AcpTurnStopReason>) -> Value {
    json!({
        "type": "done",
        "stopReason": stop_reason.map(|reason| match reason {
            AcpTurnStopReason::Completed => "completed",
            AcpTurnStopReason::Cancelled => "cancelled",
        }),
    })
}

pub(super) fn emit_turn_event(sink: Option<&dyn AcpTurnEventSink>, event: &Value) -> CliResult<()> {
    if let Some(sink) = sink {
        sink.on_event(event)?;
    }
    Ok(())
}

pub(super) fn map_spawn_error(command: &str, cwd: &str, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("ACPX command `{command}` was not found (cwd `{cwd}`)")
    } else {
        format!("spawn ACPX command `{command}` failed in `{cwd}`: {error}")
    }
}

pub(super) async fn spawn_acpx_child(
    command: &str,
    args: &[String],
    cwd: &str,
    stdin_piped: bool,
) -> CliResult<tokio::process::Child> {
    retry_executable_file_busy_async(
        || {
            let mut command_process = Command::new(command);
            command_process
                .args(args)
                .current_dir(cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if stdin_piped {
                command_process.stdin(Stdio::piped());
            } else {
                command_process.stdin(Stdio::null());
            }
            command_process.spawn()
        },
        ACPX_SPAWN_RETRY_ATTEMPTS,
        ACPX_SPAWN_RETRY_DELAY,
    )
    .await
    .map_err(|error| map_spawn_error(command, cwd, error))
}

#[allow(dead_code)]
pub(super) async fn retry_executable_file_busy<T, F>(operation: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    retry_executable_file_busy_async(operation, ACPX_SPAWN_RETRY_ATTEMPTS, ACPX_SPAWN_RETRY_DELAY)
        .await
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, dead_code)]
pub(super) fn retry_executable_file_busy_blocking<T, F>(operation: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    retry_spawn_blocking(operation, ACPX_SPAWN_RETRY_ATTEMPTS, ACPX_SPAWN_RETRY_DELAY)
}
