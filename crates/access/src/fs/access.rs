use std::path::Path;

use loong_core::{
    error::AuthorizationError,
    kernel::Kernel,
    policy::{context::WorkspacePolicyContext, engine::PolicyEngine, grant::ActionGrant},
};
use thiserror::Error;

use super::{action::FsReadAction, error::FsActionError, path::CanonicalPath};

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
    K::Cx<'a>: WorkspacePolicyContext,
{
    pub async fn read_file(
        self,
        path: impl AsRef<Path>,
    ) -> Result<ActionGrant<FsReadAction>, FsAccessError> {
        let path = CanonicalPath::resolve(path, self.policy_context.workspace_root())?;
        let action = FsReadAction::new(path);
        self.kernel
            .policy_engine()
            .grant(&self.policy_context, action)
            .await
            .map_err(FsAccessError::Authorization)
    }
}

#[derive(Debug, Error)]
pub enum FsAccessError {
    #[error(transparent)]
    Action(#[from] FsActionError),
    #[error(transparent)]
    Authorization(AuthorizationError),
}
