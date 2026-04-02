#[cfg(feature = "tool-shell")]
use std::ffi::OsStr;
#[cfg(feature = "tool-shell")]
use std::future::Future;
#[cfg(feature = "tool-shell")]
use std::io::ErrorKind;
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
pub(super) const DEFAULT_TIMEOUT_MS: u64 = 120_000;
#[cfg(feature = "tool-shell")]
pub(super) const MAX_TIMEOUT_MS: u64 = 600_000;
#[cfg(feature = "tool-shell")]
const OUTPUT_CAP_BYTES: usize = 1_048_576;
#[cfg(feature = "tool-shell")]
const EXECUTABLE_FILE_BUSY_SPAWN_RETRY_ATTEMPTS: usize = 20;
#[cfg(feature = "tool-shell")]
const EXECUTABLE_FILE_BUSY_SPAWN_RETRY_DELAY: Duration = Duration::from_millis(50);

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
    let mut child = spawn_process_with_retry(program.as_ref(), args, cwd)
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

#[cfg(feature = "tool-shell")]
async fn spawn_process_with_retry<S>(
    program: &OsStr,
    args: &[S],
    cwd: &Path,
) -> std::io::Result<tokio::process::Child>
where
    S: AsRef<OsStr>,
{
    retry_executable_file_busy(|| {
        let mut command = Command::new(program);
        command.args(args);
        command.current_dir(cwd);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.stdin(Stdio::null());
        command.kill_on_drop(true);
        command.spawn()
    })
    .await
}

#[cfg(feature = "tool-shell")]
async fn retry_executable_file_busy<T, F>(operation: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    retry_executable_file_busy_with_pause(operation, pause_before_spawn_retry).await
}

#[cfg(feature = "tool-shell")]
async fn retry_executable_file_busy_with_pause<T, F, P, Fut>(
    mut operation: F,
    mut pause: P,
) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
    P: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut attempt_count = 0;

    loop {
        attempt_count += 1;

        match operation() {
            Ok(value) => return Ok(value),
            Err(error)
                if should_retry_spawn_error(&error)
                    && attempt_count < EXECUTABLE_FILE_BUSY_SPAWN_RETRY_ATTEMPTS =>
            {
                pause().await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(feature = "tool-shell")]
async fn pause_before_spawn_retry() {
    tokio::time::sleep(EXECUTABLE_FILE_BUSY_SPAWN_RETRY_DELAY).await;
}

#[cfg(feature = "tool-shell")]
fn should_retry_spawn_error(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::ExecutableFileBusy
}

#[cfg(all(test, feature = "tool-shell"))]
mod tests {
    use super::*;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn retry_executable_file_busy_retries_until_success() {
        let attempts = AtomicUsize::new(0);

        let result = retry_executable_file_busy(|| {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed);

            if attempt < 2 {
                return Err(io::Error::from(std::io::ErrorKind::ExecutableFileBusy));
            }

            Ok("spawned")
        })
        .await
        .expect("retry should recover once the executable is no longer busy");

        assert_eq!(result, "spawned");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn retry_executable_file_busy_surfaces_non_retryable_error_immediately() {
        let attempts = AtomicUsize::new(0);

        let error = retry_executable_file_busy::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(io::Error::other("boom"))
        })
        .await
        .expect_err("non-retryable errors should surface immediately");

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn retry_executable_file_busy_stops_after_retry_budget() {
        let attempts = AtomicUsize::new(0);

        let error = retry_executable_file_busy::<(), _>(|| {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err(io::Error::from(std::io::ErrorKind::ExecutableFileBusy))
        })
        .await
        .expect_err("retry should stop after exhausting the executable-busy budget");

        assert_eq!(error.kind(), std::io::ErrorKind::ExecutableFileBusy);
        assert_eq!(
            attempts.load(Ordering::Relaxed),
            EXECUTABLE_FILE_BUSY_SPAWN_RETRY_ATTEMPTS
        );
    }

    #[tokio::test]
    async fn retry_executable_file_busy_pauses_between_retryable_failures() {
        let attempts = AtomicUsize::new(0);
        let pauses = AtomicUsize::new(0);

        let result = retry_executable_file_busy_with_pause(
            || {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed);

                if attempt < 2 {
                    return Err(io::Error::from(std::io::ErrorKind::ExecutableFileBusy));
                }

                Ok("spawned")
            },
            || async {
                pauses.fetch_add(1, Ordering::Relaxed);
            },
        )
        .await
        .expect("retry should pause between retryable executable-busy failures");

        assert_eq!(result, "spawned");
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        assert_eq!(pauses.load(Ordering::Relaxed), 2);
    }
}
