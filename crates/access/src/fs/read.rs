use std::{
    borrow::Cow,
    io,
    path::{Path, PathBuf},
};

use loong_contracts::capability::{Capabilities, Capability};
use loong_core::{
    action::{Action, ActionMeta, Granted},
    policy::PolicyEngine,
};
use serde_json::Value;
use thiserror::Error;

use crate::fs::FsAccess;

/// Failure while executing an authorized filesystem read.
#[derive(Debug, Error)]
#[error("failed to read `{}`: {source}", .path.display())]
pub struct FsReadError {
    path: PathBuf,
    #[source]
    source: io::Error,
}

impl FsReadError {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl<'a, Cx: Sync, P> FsAccess<'a, Cx, P>
where
    P: PolicyEngine<Cx>,
{
    // TODO
    // pub async fn read(&self, path: impl Into<PathBuf>) -> Result<Vec<u8>, P::Error> {
    //     let action = FsReadAction { path: path.into() };
    //     self.policy_engine.grant(self.ctx, action).await?;
    // }
}

/// Internal scaffold for the read execution boundary.
///
/// A raw `PathBuf` is not a resolved-path proof. Do not expose or construct
/// this action from an access method until that field uses the resolved type.
#[allow(
    dead_code,
    reason = "read actions stay unreachable until the resolved-path boundary exists"
)]
pub struct FsReadAction {
    pub path: PathBuf,
}

#[allow(
    dead_code,
    reason = "read action metadata is unreachable with the action scaffold"
)]
impl FsReadAction {
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
            .map_err(|source| FsReadError { path, source })
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error as _, io, path::Path};

    use super::FsReadError;

    #[test]
    fn read_error_keeps_path_and_source() {
        let error = FsReadError {
            path: "/workspace/missing.txt".into(),
            source: io::Error::new(io::ErrorKind::NotFound, "missing"),
        };

        assert_eq!(error.path(), Path::new("/workspace/missing.txt"));
        assert!(error.source().is_some());
        assert_eq!(
            error.to_string(),
            "failed to read `/workspace/missing.txt`: missing"
        );
    }
}
