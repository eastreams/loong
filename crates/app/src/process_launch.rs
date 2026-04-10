use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct ResolvedCommandInvocation {
    pub program: OsString,
    pub args: Vec<OsString>,
}

#[doc(hidden)]
pub fn should_retry_executable_file_busy(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::ExecutableFileBusy
}

#[doc(hidden)]
pub async fn retry_executable_file_busy_async<T, F>(
    mut operation: F,
    max_attempts: usize,
    delay: Duration,
) -> io::Result<T>
where
    F: FnMut() -> io::Result<T>,
{
    if max_attempts == 0 {
        let error = io::Error::new(io::ErrorKind::InvalidInput, "max_attempts must be > 0");
        return Err(error);
    }

    let mut attempt = 0;

    loop {
        attempt += 1;
        let result = operation();

        match result {
            Ok(value) => return Ok(value),
            Err(error) => {
                let retryable = should_retry_executable_file_busy(&error);
                let within_retry_budget = attempt < max_attempts;

                if retryable && within_retry_budget {
                    tokio::time::sleep(delay).await;
                    continue;
                }

                return Err(error);
            }
        }
    }
}

#[doc(hidden)]
pub fn retry_executable_file_busy_with_pause<T, F, P>(
    mut operation: F,
    max_attempts: usize,
    mut pause: P,
) -> io::Result<T>
where
    F: FnMut() -> io::Result<T>,
    P: FnMut() -> io::Result<()>,
{
    if max_attempts == 0 {
        let error = io::Error::new(io::ErrorKind::InvalidInput, "max_attempts must be > 0");
        return Err(error);
    }

    let mut attempt = 0;

    loop {
        attempt += 1;
        let result = operation();

        match result {
            Ok(value) => return Ok(value),
            Err(error) => {
                let retryable = should_retry_executable_file_busy(&error);
                let within_retry_budget = attempt < max_attempts;

                if retryable && within_retry_budget {
                    pause()?;
                    continue;
                }

                return Err(error);
            }
        }
    }
}

#[doc(hidden)]
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
pub fn retry_executable_file_busy_blocking<T, F>(
    operation: F,
    max_attempts: usize,
    delay: Duration,
) -> io::Result<T>
where
    F: FnMut() -> io::Result<T>,
{
    let pause = || {
        std::thread::sleep(delay);
        Ok(())
    };

    retry_executable_file_busy_with_pause(operation, max_attempts, pause)
}

#[doc(hidden)]
pub fn resolve_command_invocation<I, S>(command: &str, args: I) -> ResolvedCommandInvocation
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let collected_args = args
        .into_iter()
        .map(|value| OsString::from(value.as_ref()))
        .collect::<Vec<_>>();

    #[cfg(unix)]
    if let Some(invocation) = resolve_shebang_invocation(command, &collected_args) {
        return invocation;
    }

    ResolvedCommandInvocation {
        program: OsString::from(command),
        args: collected_args,
    }
}

#[cfg(unix)]
fn resolve_shebang_invocation(
    command: &str,
    collected_args: &[OsString],
) -> Option<ResolvedCommandInvocation> {
    let script_path = resolve_existing_command_path(command)?;
    let shebang = read_shebang(script_path.as_path())?;
    let trimmed_shebang = shebang.trim();
    let separator_index = trimmed_shebang.find(char::is_whitespace);
    let interpreter = match separator_index {
        Some(index) => trimmed_shebang.get(..index)?,
        None => trimmed_shebang,
    };
    let mut resolved_args = Vec::new();
    let remainder = separator_index.and_then(|index| trimmed_shebang.get(index..));
    let remainder = remainder.map(str::trim_start);
    if let Some(remainder) = remainder
        && !remainder.is_empty()
    {
        resolved_args.push(OsString::from(remainder));
    }
    let script_arg = script_path.into_os_string();
    resolved_args.push(script_arg);
    for argument in collected_args {
        let cloned_argument = argument.clone();
        resolved_args.push(cloned_argument);
    }

    Some(ResolvedCommandInvocation {
        program: OsString::from(interpreter),
        args: resolved_args,
    })
}

#[cfg(unix)]
fn resolve_existing_command_path(command: &str) -> Option<PathBuf> {
    let direct_path = Path::new(command);
    if direct_path.is_file() {
        return Some(direct_path.to_path_buf());
    }

    which::which(command).ok()
}

#[cfg(unix)]
fn read_shebang(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = [0_u8; 256];
    let count = file.read(&mut buffer).ok()?;
    let prefix = buffer.get(..2)?;
    if count < 2 || prefix != b"#!" {
        return None;
    }

    let header = buffer.get(..count)?;
    let line_end = header
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(count);
    let line = std::str::from_utf8(buffer.get(2..line_end)?).ok()?;
    let trimmed = line.trim().trim_end_matches('\r');
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{
        resolve_command_invocation, retry_executable_file_busy_async,
        retry_executable_file_busy_blocking, retry_executable_file_busy_with_pause,
        should_retry_executable_file_busy,
    };

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_wraps_shebang_script_with_interpreter() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-sh");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let script_path = root.join("script.sh");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write script");

        let resolved =
            resolve_command_invocation(script_path.to_string_lossy().as_ref(), ["--flag", "value"]);

        assert_eq!(resolved.program, std::ffi::OsString::from("/bin/sh"));
        assert_eq!(
            resolved.args,
            vec![
                script_path.into_os_string(),
                std::ffi::OsString::from("--flag"),
                std::ffi::OsString::from("value"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_preserves_non_script_program() {
        let resolved = resolve_command_invocation("/bin/echo", ["hello"]);

        assert_eq!(resolved.program, std::ffi::OsString::from("/bin/echo"));
        assert_eq!(resolved.args, vec![std::ffi::OsString::from("hello")]);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_supports_env_shebang_arguments() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-env");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let script_path = root.join("script.py");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/usr/bin/env python3\nprint('ok')\n",
        )
        .expect("write script");

        let resolved = resolve_command_invocation(
            script_path.to_string_lossy().as_ref(),
            Vec::<String>::new(),
        );

        assert_eq!(resolved.program, std::ffi::OsString::from("/usr/bin/env"));
        assert_eq!(
            resolved.args,
            vec![
                std::ffi::OsString::from("python3"),
                PathBuf::from(&script_path).into_os_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_preserves_env_split_arguments_as_one_argument() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-env-s");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let script_path = root.join("script.py");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/usr/bin/env -S python3 -u\nprint('ok')\n",
        )
        .expect("write script");

        let resolved = resolve_command_invocation(
            script_path.to_string_lossy().as_ref(),
            Vec::<String>::new(),
        );

        assert_eq!(resolved.program, std::ffi::OsString::from("/usr/bin/env"));
        assert_eq!(
            resolved.args,
            vec![
                std::ffi::OsString::from("-S python3 -u"),
                PathBuf::from(&script_path).into_os_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_invocation_uses_resolved_path_for_path_discovered_scripts() {
        let root = crate::test_support::unique_temp_dir("loongclaw-process-launch-path");
        let bin_dir = root.join("bin");
        let script_path = bin_dir.join("path-script");
        std::fs::create_dir_all(&bin_dir).expect("create bin dir");
        crate::test_support::write_executable_script_atomically(
            &script_path,
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write path-discovered script");

        let mut env = crate::test_support::ScopedEnv::new();
        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let mut path_entries = vec![PathBuf::from(&bin_dir)];
        path_entries.extend(std::env::split_paths(Path::new(&original_path)).collect::<Vec<_>>());
        let joined_path = std::env::join_paths(path_entries).expect("join PATH");
        env.set("PATH", joined_path);

        let resolved = resolve_command_invocation("path-script", ["--flag"]);

        assert_eq!(resolved.program, std::ffi::OsString::from("/bin/sh"));
        assert_eq!(
            resolved.args,
            vec![
                script_path.into_os_string(),
                std::ffi::OsString::from("--flag"),
            ]
        );
    }

    #[test]
    fn should_retry_executable_file_busy_matches_executable_file_busy() {
        let busy_error = Error::from(ErrorKind::ExecutableFileBusy);
        let missing_error = Error::from(ErrorKind::NotFound);

        assert!(should_retry_executable_file_busy(&busy_error));
        assert!(!should_retry_executable_file_busy(&missing_error));
    }

    #[tokio::test]
    async fn retry_executable_file_busy_async_retries_until_success() {
        let attempts = AtomicUsize::new(0);

        let result = retry_executable_file_busy_async(
            || {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed);

                if attempt < 2 {
                    return Err(Error::from(ErrorKind::ExecutableFileBusy));
                }

                Ok("spawned")
            },
            5,
            Duration::ZERO,
        )
        .await
        .expect("executable-file-busy errors should retry");

        let total_attempts = attempts.load(Ordering::Relaxed);

        assert_eq!(result, "spawned");
        assert_eq!(total_attempts, 3);
    }

    #[tokio::test]
    async fn retry_executable_file_busy_async_rejects_zero_attempt_budget() {
        let result =
            retry_executable_file_busy_async(|| Ok::<_, Error>("spawned"), 0, Duration::ZERO).await;

        let error = result.expect_err("zero-attempt budget should be rejected");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn retry_executable_file_busy_with_pause_records_retry_boundaries() {
        let attempts = AtomicUsize::new(0);
        let pauses = AtomicUsize::new(0);

        let result = retry_executable_file_busy_with_pause(
            || {
                let attempt = attempts.fetch_add(1, Ordering::Relaxed);

                if attempt < 2 {
                    return Err(Error::from(ErrorKind::ExecutableFileBusy));
                }

                Ok("spawned")
            },
            5,
            || {
                pauses.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        )
        .expect("retry helper should recover after retryable failures");

        let total_attempts = attempts.load(Ordering::Relaxed);
        let total_pauses = pauses.load(Ordering::Relaxed);

        assert_eq!(result, "spawned");
        assert_eq!(total_attempts, 3);
        assert_eq!(total_pauses, 2);
    }

    #[test]
    fn retry_executable_file_busy_with_pause_rejects_zero_attempt_budget() {
        let result =
            retry_executable_file_busy_with_pause(|| Ok::<_, Error>("spawned"), 0, || Ok(()));

        let error = result.expect_err("zero-attempt budget should be rejected");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn retry_executable_file_busy_blocking_stops_after_retry_budget() {
        let attempts = AtomicUsize::new(0);

        let error = retry_executable_file_busy_blocking(
            || {
                attempts.fetch_add(1, Ordering::Relaxed);
                Err::<(), Error>(Error::from(ErrorKind::ExecutableFileBusy))
            },
            4,
            Duration::ZERO,
        )
        .expect_err("retry helper should stop after exhausting the retry budget");

        let total_attempts = attempts.load(Ordering::Relaxed);

        assert_eq!(error.kind(), ErrorKind::ExecutableFileBusy);
        assert_eq!(total_attempts, 4);
    }
}
