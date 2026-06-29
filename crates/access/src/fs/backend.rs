use async_trait::async_trait;

use loong_core::action::ActionExecutor;
use loong_core::error::{CapabilityError, ExecutionError};
use loong_core::policy::Granted;

use super::action::{CanonicalPath, FsReadAction, FsWriteAction};

#[async_trait]
pub trait FsBackend: Send + Sync {
    async fn read_canonical(&self, path: &CanonicalPath) -> Result<String, CapabilityError>;
    async fn write_canonical(
        &self,
        path: &CanonicalPath,
        content: &str,
    ) -> Result<(), CapabilityError>;
}

pub trait HasFsBackend: Send + Sync {
    type FsBackend: FsBackend + ?Sized;

    fn fs_backend(&self) -> &Self::FsBackend;
}

#[async_trait]
impl<T> FsBackend for T
where
    T: HasFsBackend,
{
    async fn read_canonical(&self, path: &CanonicalPath) -> Result<String, CapabilityError> {
        self.fs_backend().read_canonical(path).await
    }

    async fn write_canonical(
        &self,
        path: &CanonicalPath,
        content: &str,
    ) -> Result<(), CapabilityError> {
        self.fs_backend().write_canonical(path, content).await
    }
}

#[derive(Default)]
pub struct StdFsBackend;

impl StdFsBackend {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FsBackend for StdFsBackend {
    async fn read_canonical(&self, path: &CanonicalPath) -> Result<String, CapabilityError> {
        tokio::fs::read_to_string(path.as_path())
            .await
            .map_err(CapabilityError::Io)
    }

    async fn write_canonical(
        &self,
        path: &CanonicalPath,
        content: &str,
    ) -> Result<(), CapabilityError> {
        tokio::fs::write(path.as_path(), content)
            .await
            .map_err(CapabilityError::Io)
    }
}

// TODO: implement Fs{Read,Write}Action Executor
