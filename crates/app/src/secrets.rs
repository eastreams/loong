use std::fs;
use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use loong_contracts::{SecretRef, SecretResolutionError, SecretResolver, SecretValue};
use wait_timeout::ChildExt;

const DEFAULT_SECRET_EXEC_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct DefaultSecretResolver {
    exec_timeout_ms: u64,
}

impl Default for DefaultSecretResolver {
    fn default() -> Self {
        Self {
            exec_timeout_ms: DEFAULT_SECRET_EXEC_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SecretLookup {
    Absent,
    Missing,
    Value(String),
}

impl SecretResolver for DefaultSecretResolver {
    fn resolve(
        &self,
        secret_ref: &SecretRef,
    ) -> Result<Option<SecretValue>, SecretResolutionError> {
        let explicit_env_name = secret_ref.explicit_env_name();
        if let Some(explicit_env_name) = explicit_env_name {
            return self.resolve_env_secret(explicit_env_name.as_str());
        }

        match secret_ref {
            SecretRef::Inline(value) => {
                let resolved = normalize_inline_secret_value(value.as_str());
                Ok(resolved.map(SecretValue::new))
            }
            SecretRef::Env { env } => self.resolve_env_secret(env.as_str()),
            SecretRef::File { file } => self.resolve_file_secret(file),
            SecretRef::Exec { exec } => self.resolve_exec_secret(exec.as_slice()),
        }
    }
}

impl DefaultSecretResolver {
    fn resolve_env_secret(
        &self,
        env_name: &str,
    ) -> Result<Option<SecretValue>, SecretResolutionError> {
        let trimmed_env_name = env_name.trim();
        if trimmed_env_name.is_empty() {
            return Err(SecretResolutionError::InvalidEnvName {
                env: env_name.to_owned(),
            });
        }

        let env_value = std::env::var(trimmed_env_name).ok();
        let Some(env_value) = env_value else {
            return Ok(None);
        };

        let trimmed_value = env_value.trim();
        if trimmed_value.is_empty() {
            return Ok(None);
        }

        let secret_value = SecretValue::new(trimmed_value.to_owned());
        Ok(Some(secret_value))
    }

    fn resolve_file_secret(
        &self,
        file: &std::path::Path,
    ) -> Result<Option<SecretValue>, SecretResolutionError> {
        let path_string = file.display().to_string();
        let contents =
            fs::read_to_string(file).map_err(|error| SecretResolutionError::FileRead {
                path: path_string.clone(),
                message: error.to_string(),
            })?;

        let trimmed_contents = trim_trailing_newlines(contents.as_str());
        if trimmed_contents.is_empty() {
            return Ok(None);
        }

        let secret_value = SecretValue::new(trimmed_contents.to_owned());
        Ok(Some(secret_value))
    }

    fn resolve_exec_secret(
        &self,
        exec: &[String],
    ) -> Result<Option<SecretValue>, SecretResolutionError> {
        let Some(program) = exec.first() else {
            return Err(SecretResolutionError::EmptyExec);
        };

        let trimmed_program = program.trim();
        if trimmed_program.is_empty() {
            return Err(SecretResolutionError::EmptyExec);
        }

        let mut command = Command::new(trimmed_program);
        let args = exec.get(1..).unwrap_or(&[]);
        command.args(args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|error| SecretResolutionError::ExecSpawn {
                program: trimmed_program.to_owned(),
                message: error.to_string(),
            })?;

        let stdout_pipe = child.stdout.take().ok_or(SecretResolutionError::ExecWait {
            program: trimmed_program.to_owned(),
            message: "stdout pipe unavailable".to_owned(),
        })?;
        let stderr_pipe = child.stderr.take().ok_or(SecretResolutionError::ExecWait {
            program: trimmed_program.to_owned(),
            message: "stderr pipe unavailable".to_owned(),
        })?;

        let stdout_reader = spawn_exec_output_reader(stdout_pipe);
        let stderr_reader = spawn_exec_output_reader(stderr_pipe);

        let timeout = Duration::from_millis(self.exec_timeout_ms);
        let status =
            child
                .wait_timeout(timeout)
                .map_err(|error| SecretResolutionError::ExecWait {
                    program: trimmed_program.to_owned(),
                    message: error.to_string(),
                })?;

        if status.is_none() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_exec_output_reader(stdout_reader, trimmed_program, "stdout");
            let _ = join_exec_output_reader(stderr_reader, trimmed_program, "stderr");
            return Err(SecretResolutionError::ExecTimeout {
                program: trimmed_program.to_owned(),
                timeout_ms: self.exec_timeout_ms,
            });
        }

        let Some(status) = status else {
            return Err(SecretResolutionError::ExecTimeout {
                program: trimmed_program.to_owned(),
                timeout_ms: self.exec_timeout_ms,
            });
        };
        let stdout = join_exec_output_reader(stdout_reader, trimmed_program, "stdout")?;
        let stderr = join_exec_output_reader(stderr_reader, trimmed_program, "stderr")?;

        if !status.success() {
            let status_string = status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned());
            let stderr = String::from_utf8_lossy(stderr.as_slice());
            let trimmed_stderr = stderr.trim().to_owned();
            return Err(SecretResolutionError::ExecFailed {
                program: trimmed_program.to_owned(),
                status: status_string,
                message: trimmed_stderr,
            });
        }

        let stdout = String::from_utf8(stdout).map_err(|_utf8_error| {
            SecretResolutionError::ExecInvalidUtf8 {
                program: trimmed_program.to_owned(),
            }
        })?;
        let trimmed_stdout = trim_trailing_newlines(stdout.as_str());
        if trimmed_stdout.is_empty() {
            return Ok(None);
        }

        let secret_value = SecretValue::new(trimmed_stdout.to_owned());
        Ok(Some(secret_value))
    }
}

pub(crate) fn resolve_secret_lookup(secret_ref: Option<&SecretRef>) -> SecretLookup {
    let Some(secret_ref) = secret_ref else {
        return SecretLookup::Absent;
    };

    if inline_secret_ref_is_blank(secret_ref) {
        return SecretLookup::Absent;
    }

    let resolver = DefaultSecretResolver::default();
    let resolved = resolver.resolve(secret_ref);

    match resolved {
        Ok(Some(secret_value)) => {
            let value = secret_value.into_inner();
            SecretLookup::Value(value)
        }
        Ok(None) | Err(_) => SecretLookup::Missing,
    }
}

pub(crate) fn resolve_secret_with_legacy_env(
    secret_ref: Option<&SecretRef>,
    env_name: Option<&str>,
) -> Option<String> {
    let secret_lookup = resolve_secret_lookup(secret_ref);
    match secret_lookup {
        SecretLookup::Value(value) => Some(value),
        SecretLookup::Missing => None,
        SecretLookup::Absent => read_non_empty_env_value(env_name),
    }
}

pub(crate) fn has_configured_secret_ref(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };
    secret_ref.is_configured()
}

pub(crate) fn secret_ref_env_name(secret_ref: Option<&SecretRef>) -> Option<String> {
    let secret_ref = secret_ref?;
    secret_ref.explicit_env_name()
}

pub(crate) fn canonicalize_env_secret_reference(
    secret_ref: &mut Option<SecretRef>,
    env_name: &mut Option<String>,
) {
    let explicit_env_name = secret_ref_env_name(secret_ref.as_ref());
    let Some(explicit_env_name) = explicit_env_name else {
        return;
    };

    let configured_env_name = env_name.as_deref();
    let configured_env_name = configured_env_name.map(str::trim);
    let configured_env_name = configured_env_name.filter(|value| !value.is_empty());

    match configured_env_name {
        None => {
            *env_name = Some(explicit_env_name);
            *secret_ref = None;
        }
        Some(configured_env_name) if configured_env_name == explicit_env_name => {
            *env_name = Some(explicit_env_name);
            *secret_ref = None;
        }
        Some(_) => {}
    }
}

fn normalize_inline_secret_value(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn inline_secret_ref_is_blank(secret_ref: &SecretRef) -> bool {
    let Some(inline_value) = secret_ref.inline_value() else {
        return false;
    };
    inline_value.trim().is_empty()
}

fn read_non_empty_env_value(env_name: Option<&str>) -> Option<String> {
    let env_name = env_name?;
    let trimmed_env_name = env_name.trim();
    if trimmed_env_name.is_empty() {
        return None;
    }

    let env_value = std::env::var(trimmed_env_name).ok()?;
    let trimmed_value = env_value.trim();
    if trimmed_value.is_empty() {
        return None;
    }

    Some(trimmed_value.to_owned())
}

fn trim_trailing_newlines(raw: &str) -> &str {
    raw.trim_end_matches(['\r', '\n'])
}

fn spawn_exec_output_reader<R>(mut reader: R) -> thread::JoinHandle<io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn join_exec_output_reader(
    reader: thread::JoinHandle<io::Result<Vec<u8>>>,
    program: &str,
    stream_name: &str,
) -> Result<Vec<u8>, SecretResolutionError> {
    let bytes = reader
        .join()
        .map_err(|_panic_payload| SecretResolutionError::ExecWait {
            program: program.to_owned(),
            message: format!("reading {stream_name} output panicked"),
        })?;
    bytes.map_err(|error| SecretResolutionError::ExecWait {
        program: program.to_owned(),
        message: format!("reading {stream_name} output failed: {error}"),
    })
}
