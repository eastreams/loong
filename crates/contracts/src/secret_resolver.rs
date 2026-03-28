use thiserror::Error;

use crate::{SecretRef, SecretValue};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SecretResolutionError {
    #[error("secret env reference `{env}` is invalid")]
    InvalidEnvName { env: String },
    #[error("failed to read secret file `{path}`: {message}")]
    FileRead { path: String, message: String },
    #[error("secret exec command must not be empty")]
    EmptyExec,
    #[error("failed to start secret exec `{program}`: {message}")]
    ExecSpawn { program: String, message: String },
    #[error("failed while waiting for secret exec `{program}`: {message}")]
    ExecWait { program: String, message: String },
    #[error("secret exec `{program}` timed out after {timeout_ms}ms")]
    ExecTimeout { program: String, timeout_ms: u64 },
    #[error("secret exec `{program}` exited with status {status}: {message}")]
    ExecFailed {
        program: String,
        status: String,
        message: String,
    },
    #[error("secret exec `{program}` output was not valid UTF-8")]
    ExecInvalidUtf8 { program: String },
}

pub trait SecretResolver {
    fn resolve(&self, secret_ref: &SecretRef)
    -> Result<Option<SecretValue>, SecretResolutionError>;
}
