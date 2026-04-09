#[cfg(feature = "tool-shell")]
use serde_json::Value;
#[cfg(feature = "tool-shell")]
use std::ffi::OsStr;
#[cfg(feature = "tool-shell")]
use std::future::Future;
#[cfg(feature = "tool-shell")]
use std::path::{Path, PathBuf};
#[cfg(feature = "tool-shell")]
use std::process::{Output, Stdio};
#[cfg(feature = "tool-shell")]
use std::sync::Arc;
#[cfg(feature = "tool-shell")]
use std::thread;
#[cfg(feature = "tool-shell")]
use std::time::{Duration, Instant};
#[cfg(feature = "tool-shell")]
use tokio::io::AsyncReadExt;
#[cfg(feature = "tool-shell")]
use tokio::process::Command;

#[cfg(feature = "tool-shell")]
use super::runtime_events::{
    ToolCommandMetrics, ToolOutputDelta, ToolRuntimeEvent, ToolRuntimeEventSink, ToolRuntimeStream,
};
#[cfg(feature = "tool-shell")]
use crate::process_launch::retry_executable_file_busy_async;

#[cfg(feature = "tool-shell")]
pub(super) const DEFAULT_TIMEOUT_MS: u64 = 120_000;
#[cfg(feature = "tool-shell")]
pub(super) const MAX_TIMEOUT_MS: u64 = 600_000;
#[cfg(feature = "tool-shell")]
const OUTPUT_CAP_BYTES: usize = 1_048_576;
#[cfg(feature = "tool-shell")]
const SPAWN_RETRY_ATTEMPTS: usize = 5;
#[cfg(feature = "tool-shell")]
const SPAWN_RETRY_DELAY: Duration = Duration::from_millis(25);

#[cfg(feature = "tool-shell")]
pub(super) fn resolve_process_cwd_with_config(
    payload: &serde_json::Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
    tool_name: &str,
) -> Result<PathBuf, String> {
    let raw_cwd = parse_optional_process_cwd(payload, tool_name)?;

    let resolved_cwd = match raw_cwd {
        Some(raw_cwd) => resolve_process_cwd_override(raw_cwd, config, tool_name)?,
        None => default_process_cwd_with_config(config),
    };

    if !resolved_cwd.is_dir() {
        let display_path = resolved_cwd.display();
        let error = format!("{tool_name} cwd `{display_path}` is not a directory");
        return Err(error);
    }

    Ok(resolved_cwd)
}

#[cfg(feature = "tool-shell")]
fn parse_optional_process_cwd<'a>(
    payload: &'a serde_json::Map<String, Value>,
    tool_name: &str,
) -> Result<Option<&'a str>, String> {
    let Some(raw_value) = payload.get("cwd") else {
        return Ok(None);
    };

    let raw_cwd = raw_value
        .as_str()
        .ok_or_else(|| format!("{tool_name} payload.cwd must be a string"))?;
    let trimmed_cwd = raw_cwd.trim();
    if trimmed_cwd.is_empty() {
        return Ok(None);
    }

    Ok(Some(trimmed_cwd))
}

#[cfg(feature = "tool-shell")]
fn default_process_cwd_with_config(config: &super::runtime_config::ToolRuntimeConfig) -> PathBuf {
    let configured_root = config.file_root.clone();
    if let Some(configured_root) = configured_root {
        return configured_root;
    }

    let current_dir_result = std::env::current_dir();
    match current_dir_result {
        Ok(current_dir) => current_dir,
        Err(_) => PathBuf::from("."),
    }
}

#[cfg(feature = "tool-shell")]
fn resolve_process_cwd_override(
    raw_cwd: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
    tool_name: &str,
) -> Result<PathBuf, String> {
    if config.file_root.is_some() {
        return super::file::resolve_safe_directory_path_with_config(raw_cwd, config);
    }

    let requested_path = PathBuf::from(raw_cwd);
    let base_path = if requested_path.is_absolute() {
        requested_path
    } else {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        current_dir.join(requested_path)
    };

    canonicalize_existing_directory(base_path.as_path(), tool_name)
}

#[cfg(feature = "tool-shell")]
fn canonicalize_existing_directory(path: &Path, tool_name: &str) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        let display_path = path.display();
        format!("failed to inspect {tool_name} cwd `{display_path}`: {error}")
    })?;

    if !metadata.is_dir() {
        let display_path = path.display();
        let error = format!("{tool_name} cwd `{display_path}` is not a directory");
        return Err(error);
    }

    std::fs::canonicalize(path).map_err(|error| {
        let display_path = path.display();
        format!("failed to canonicalize {tool_name} cwd `{display_path}`: {error}")
    })
}

#[cfg(feature = "tool-shell")]
pub(super) fn run_tool_async<F>(future: F, tool_label: &str) -> Result<F::Output, String>
where
    F: Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            Ok(tokio::task::block_in_place(|| handle.block_on(future)))
        }
        Ok(_) => thread::scope(|scope| {
            scope
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create tokio runtime for {tool_label}: {error}")
                        })?;
                    Ok(rt.block_on(future))
                })
                .join()
                .map_err(|_panic| format!("{tool_label} async worker thread panicked"))?
        }),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    format!("failed to create tokio runtime for {tool_label}: {error}")
                })?;
            Ok(rt.block_on(future))
        }
    }
}

#[cfg(feature = "tool-shell")]
async fn read_capped_with_runtime_events<R>(
    mut reader: R,
    cap: usize,
    stream_name: &str,
    stream: ToolRuntimeStream,
    runtime_event_sink: Option<Arc<dyn ToolRuntimeEventSink>>,
) -> Result<Vec<u8>, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8_192];
    let mut total_bytes = 0_usize;
    let mut newline_count = 0_usize;

    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("{stream_name} read failed: {error}"))?;
        if read == 0 {
            break;
        }

        total_bytes = total_bytes.saturating_add(read);
        let chunk_bytes = buffer.get(..read).unwrap_or(buffer.as_slice());
        let chunk_newlines = chunk_bytes.iter().filter(|byte| **byte == b'\n').count();
        newline_count = newline_count.saturating_add(chunk_newlines);
        let last_byte_was_newline = chunk_bytes.last().copied() == Some(b'\n');

        let remaining = cap.saturating_sub(output.len());
        if remaining > 0 {
            let to_copy = remaining.min(read);
            let retained_chunk = chunk_bytes.get(..to_copy).unwrap_or(chunk_bytes);
            output.extend(retained_chunk.iter().copied());
        }

        let has_partial_final_line = total_bytes > 0 && !last_byte_was_newline;
        let total_lines = newline_count + usize::from(has_partial_final_line);
        let truncated = total_bytes > cap;

        if let Some(sink) = runtime_event_sink.as_ref() {
            let chunk_text = String::from_utf8_lossy(chunk_bytes).into_owned();
            let event = ToolRuntimeEvent::OutputDelta(ToolOutputDelta {
                stream,
                chunk: chunk_text,
                total_bytes,
                total_lines,
                truncated,
            });
            sink.emit(event);
        }
    }

    Ok(output)
}

#[cfg(feature = "tool-shell")]
pub(super) async fn run_process_with_timeout_with_sink<P, S>(
    program: P,
    args: &[S],
    cwd: &Path,
    timeout_ms: u64,
    error_prefix: &str,
    runtime_event_sink: Option<Arc<dyn ToolRuntimeEventSink>>,
) -> Result<Output, String>
where
    P: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    let started_at = Instant::now();
    let mut command = Command::new(program);
    let sanitized_env = loongclaw_contracts::sanitized_child_process_env();

    command.env_clear();
    command.envs(sanitized_env);
    command.args(args);
    command.current_dir(cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(Stdio::null());
    command.kill_on_drop(true);

    let mut child = retry_executable_file_busy_async(
        || command.spawn(),
        SPAWN_RETRY_ATTEMPTS,
        SPAWN_RETRY_DELAY,
    )
    .await
    .map_err(|error| format!("{error_prefix} spawn failed: {error}"))?;

    let duration = Duration::from_millis(timeout_ms.max(1));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{error_prefix} stdout pipe missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{error_prefix} stderr pipe missing"))?;

    let metrics_runtime_event_sink = runtime_event_sink.clone();
    let stdout_runtime_event_sink = runtime_event_sink.clone();
    let stdout_task = tokio::spawn(async move {
        read_capped_with_runtime_events(
            stdout,
            OUTPUT_CAP_BYTES,
            "stdout",
            ToolRuntimeStream::Stdout,
            stdout_runtime_event_sink,
        )
        .await
    });
    let stderr_runtime_event_sink = runtime_event_sink;
    let stderr_task = tokio::spawn(async move {
        read_capped_with_runtime_events(
            stderr,
            OUTPUT_CAP_BYTES,
            "stderr",
            ToolRuntimeStream::Stderr,
            stderr_runtime_event_sink,
        )
        .await
    });

    let result = match tokio::time::timeout(duration, child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout_result, stderr_result) = tokio::join!(stdout_task, stderr_task);
            let stdout = stdout_result
                .map_err(|join_error| {
                    format!("{error_prefix} stdout reader panicked: {join_error}")
                })?
                .map_err(|error| format!("{error_prefix} {error}"))?;
            let stderr = stderr_result
                .map_err(|join_error| {
                    format!("{error_prefix} stderr reader panicked: {join_error}")
                })?
                .map_err(|error| format!("{error_prefix} {error}"))?;

            Ok(Output {
                status,
                stdout,
                stderr,
            })
        }
        Ok(Err(error)) => {
            stdout_task.abort();
            stderr_task.abort();
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = tokio::join!(stdout_task, stderr_task);
            Err(format!("{error_prefix} wait failed: {error}"))
        }
        Err(_) => {
            stdout_task.abort();
            stderr_task.abort();
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = tokio::join!(stdout_task, stderr_task);
            Err(format!("{error_prefix} timed out after {timeout_ms}ms"))
        }
    };

    if let Some(sink) = metrics_runtime_event_sink.as_ref() {
        let duration_ms = started_at.elapsed().as_millis();
        let duration_ms = u64::try_from(duration_ms).unwrap_or(u64::MAX);
        let exit_code = result.as_ref().ok().and_then(|output| output.status.code());
        let metrics = ToolCommandMetrics {
            exit_code,
            duration_ms,
        };
        let event = ToolRuntimeEvent::CommandMetrics(metrics);
        sink.emit(event);
    }

    result
}

#[cfg(all(test, feature = "tool-shell"))]
mod tests {
    use std::io::{Error, ErrorKind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::process_launch::{
        retry_executable_file_busy_async, should_retry_executable_file_busy,
    };

    #[test]
    fn should_retry_spawn_error_matches_executable_file_busy() {
        let busy_error = Error::from(ErrorKind::ExecutableFileBusy);
        let missing_error = Error::from(ErrorKind::NotFound);

        assert!(should_retry_executable_file_busy(&busy_error));
        assert!(!should_retry_executable_file_busy(&missing_error));
    }

    #[tokio::test]
    async fn retry_executable_file_busy_retries_until_success() {
        let attempts = AtomicUsize::new(0);

        let result = retry_executable_file_busy_async(
            || {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed);

                if attempt < 2 {
                    return Err(Error::from(ErrorKind::ExecutableFileBusy));
                }

                Ok("spawned")
            },
            super::SPAWN_RETRY_ATTEMPTS,
            super::SPAWN_RETRY_DELAY,
        )
        .await
        .expect("executable-file-busy errors should retry");

        let total_attempts = attempts.load(Ordering::Relaxed);

        assert_eq!(result, "spawned");
        assert_eq!(total_attempts, 3);
    }
}
