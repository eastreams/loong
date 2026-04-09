#[cfg(feature = "tool-shell")]
use std::ffi::OsStr;
#[cfg(feature = "tool-shell")]
use std::future::Future;
#[cfg(feature = "tool-shell")]
use std::path::Path;
#[cfg(feature = "tool-shell")]
use std::process::{Output, Stdio};
#[cfg(feature = "tool-shell")]
use std::thread;
#[cfg(feature = "tool-shell")]
use std::time::Duration;
#[cfg(feature = "tool-shell")]
use tokio::io::AsyncReadExt;
#[cfg(feature = "tool-shell")]
use tokio::process::Command;

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
async fn read_capped<R>(mut reader: R, cap: usize, stream_name: &str) -> Result<Vec<u8>, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8_192];

    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("{stream_name} read failed: {error}"))?;
        if read == 0 {
            break;
        }

        let remaining = cap.saturating_sub(output.len());
        if remaining > 0 {
            let to_copy = remaining.min(read);
            output.extend(buffer.iter().take(to_copy).copied());
        }
    }

    Ok(output)
}

#[cfg(feature = "tool-shell")]
pub(super) async fn run_process_with_timeout<P, S>(
    program: P,
    args: &[S],
    cwd: &Path,
    timeout_ms: u64,
    error_prefix: &str,
) -> Result<Output, String>
where
    P: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
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

    let stdout_task =
        tokio::spawn(async move { read_capped(stdout, OUTPUT_CAP_BYTES, "stdout").await });
    let stderr_task =
        tokio::spawn(async move { read_capped(stderr, OUTPUT_CAP_BYTES, "stderr").await });

    match tokio::time::timeout(duration, child.wait()).await {
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
    }
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
