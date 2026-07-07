use std::path::{Path, PathBuf};

use loong_core::{error::AuthorizationError, kernel::Kernel, policy::engine::PolicyEngine};
use thiserror::Error;

use super::{action::FsReadAction, error::FsActionError, path::CanonicalPath};

pub trait FsAccessContext {
    fn fs_resolution_root(&self) -> &Path;

    fn fs_allowed_roots(&self) -> &[PathBuf];
}

pub trait HasFsAccess<'a, K>: Sized
where
    K: Kernel,
{
    fn fs(self) -> FsAccess<'a, K>;
}

pub struct FsAccess<'a, K>
where
    K: Kernel,
{
    kernel: &'a K,
    policy_context: K::Cx<'a>,
}

impl<'a, K> FsAccess<'a, K>
where
    K: Kernel,
{
    #[inline(always)]
    #[must_use]
    pub fn new(kernel: &'a K, policy_context: K::Cx<'a>) -> Self {
        Self {
            kernel,
            policy_context,
        }
    }
}

impl<'a, K> FsAccess<'a, K>
where
    K: Kernel,
    K::Cx<'a>: FsAccessContext,
{
    pub async fn read_file(self, path: impl AsRef<Path>) -> Result<FsReadOutput, FsAccessError> {
        let path = CanonicalPath::resolve(
            path,
            self.policy_context.fs_resolution_root(),
            self.policy_context.fs_allowed_roots(),
        )?;
        let action = FsReadAction::new(path);
        let grant = self
            .kernel
            .policy_engine()
            .grant(&self.policy_context, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        let action = grant.granted.into_action();
        let path = action.path().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|source| FsAccessError::ReadFile {
            path: path.clone(),
            source,
        })?;
        Ok(FsReadOutput { path, bytes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadOutput {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum FsAccessError {
    #[error(transparent)]
    Action(#[from] FsActionError),
    #[error(transparent)]
    Authorization(AuthorizationError),
    #[error("failed to read file {path}: {source}", path = .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
