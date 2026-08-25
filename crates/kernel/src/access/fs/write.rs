use std::{borrow::Cow, io, path::PathBuf};

use crate::policy::action::{Action, ActionMeta, Granted};
use contracts::capability::{Capabilities, Capability};
use serde_json::Value;
use thiserror::Error;

use crate::GrantSendError;

use super::{FsAccess, FsPathError};

/// Failure while authorizing or executing a filesystem write.
#[derive(Debug, Error)]
pub enum FsWriteError {
    #[error("fs.write: {0}")]
    Denied(#[from] GrantSendError),
    #[error("fs.write path: {0}")]
    Path(#[from] FsPathError),
    #[error("failed to write `{}`: {source}", .path.display())]
    Io { path: PathBuf, source: io::Error },
}

impl<'a> FsAccess<'a> {
    #[inline]
    pub async fn write(
        &self,
        path: impl AsRef<std::path::Path>,
        content: Vec<u8>,
    ) -> Result<(), FsWriteError> {
        let path = super::resolve_for_write(&self.workspace_root, path.as_ref()).await?;
        let action = FsWriteAction::new(path, content);
        let granted = self.ctx.grant(action).await?;
        granted.run(self.ctx).await
    }
}

/// The authorized filesystem write action.
pub struct FsWriteAction {
    path: PathBuf,
    content: Vec<u8>,
}

impl FsWriteAction {
    pub(super) fn new(path: PathBuf, content: Vec<u8>) -> Self {
        Self { path, content }
    }

    const NAME: &str = "fs.write";
    const CAPABILITIES: Capabilities = Capabilities::singleton(Capability::FsWrite);
}

impl ActionMeta for FsWriteAction {
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

impl<Cx: Sync> Action<Cx> for FsWriteAction {
    type Output = ();
    type Error = FsWriteError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let (_, action) = granted.into_parts();
        let path = action.path;
        let content = action.content;

        tokio::fs::write(&path, content)
            .await
            .map_err(|source| FsWriteError::Io { path, source })
    }
}
