use std::{borrow::Cow, io, path::PathBuf};

use crate::policy::action::{Action, ActionMeta, Granted};
use contracts::capability::{Capabilities, Capability};
use serde_json::Value;
use thiserror::Error;

use crate::GrantSendError;

use super::{FsAccess, FsPathError};

/// Failure while authorizing or executing a filesystem read.
#[derive(Debug, Error)]
pub enum FsReadError {
    #[error("fs.read: {0}")]
    Denied(#[from] GrantSendError),
    #[error("fs.read path: {0}")]
    Path(#[from] FsPathError),
    #[error("failed to read `{}`: {source}", .path.display())]
    Io { path: PathBuf, source: io::Error },
}

impl<'a> FsAccess<'a> {
    #[inline]
    pub async fn read(&self, path: impl AsRef<std::path::Path>) -> Result<Vec<u8>, FsReadError> {
        let path = super::resolve_existing(&self.workspace_root, path.as_ref()).await?;
        let action = FsReadAction::new(path);
        let granted = self.ctx.grant(action).await?;
        granted.run(self.ctx).await
    }
}

/// The authorized filesystem read action.
pub struct FsReadAction {
    path: PathBuf,
}

impl FsReadAction {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    const NAME: &str = "fs.read";
    const CAPABILITIES: Capabilities = Capabilities::singleton(Capability::FsRead);
}

impl ActionMeta for FsReadAction {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(Self::NAME)
    }
    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(Value::String(self.path.display().to_string()))
    }
    fn required_capabilities(&self) -> Capabilities {
        Self::CAPABILITIES
    }
}

impl<Cx: Sync> Action<Cx> for FsReadAction {
    type Output = Vec<u8>;
    type Error = FsReadError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let (_, action) = granted.into_parts();
        let path = action.path;

        tokio::fs::read(&path)
            .await
            .map_err(|source| FsReadError::Io { path, source })
    }
}
