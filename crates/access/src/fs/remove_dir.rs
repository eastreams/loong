use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyDecision, PolicyGrant};
use loong_core::{
    error::PolicyGrantError,
    policy::{
        action::{Action, ActionMeta, ActionMetadata},
        context::ContextFactory,
        engine::PolicyEngine,
        grant::Granted,
        policy::Policy,
    },
};
use serde_json::{Value, json};
use thiserror::Error;

use super::{
    access::FsAccess,
    path::{FsPathPolicyContext, FsResolutionContext, GrantedEntryPath},
};

#[cfg(test)]
mod tests;

const FS_REMOVE_DIR_ALL_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];

#[derive(Debug, Error)]
pub enum FsRemoveDirAllError {
    #[error(transparent)]
    Path(#[from] super::path::FsPathError),
    #[error(transparent)]
    Authorization(#[from] PolicyGrantError),
    #[error("failed to inspect path {path}: {source}", path = .path.display())]
    InspectPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("refusing to recursively remove symlink {path}", path = .path.display())]
    RefuseSymlink { path: PathBuf },
    #[error("path {path} is not a directory", path = .path.display())]
    PathIsNotDirectory { path: PathBuf },
    #[error("failed to remove directory {path}: {source}", path = .path.display())]
    RemoveDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Typed action for removing one governed directory tree.
///
/// Like file removal, this uses final-component no-follow path facts. The run
/// boundary refuses symlinks so recursive deletion cannot leave the governed
/// tree through a final symlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRemoveDirAllAction {
    path: GrantedEntryPath,
}

impl FsRemoveDirAllAction {
    #[must_use]
    pub fn new(path: GrantedEntryPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsRemoveDirAllAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.remove_dir_all",
            operation: Cow::Borrowed("remove_dir_all"),
            required_capabilities: Cow::Borrowed(&FS_REMOVE_DIR_ALL_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
        }))
    }
}

/// Terminal recursive-directory-removal policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsRemoveDirAllAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsRemoveDirAllAction> for FsRemoveDirAllAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-remove-dir-all-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsRemoveDirAllAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.remove_dir_all reached terminal allow policy".into()),
            reason: "filesystem directory removal allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Recursively remove one governed directory tree.
    ///
    /// This operation is intentionally distinct from `remove_file`: recursive
    /// deletion has a larger blast radius and refuses final-component symlinks.
    pub async fn remove_dir_all(
        self,
        path: impl AsRef<Path>,
    ) -> Result<FsRemoveDirAllOutput, FsRemoveDirAllError> {
        let path = self.grant_entry_path(path).await?;

        let action = FsRemoveDirAllAction::new(path);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized recursive directory removal.
///
/// This primitive is intentionally separate from [`crate::fs::FsRemoveFileAction`]:
/// recursive deletion is broader than unlinking one file or symlink, so tools
/// must request it explicitly and policy can report it as a distinct action.
#[async_trait]
impl<Cx> Action<Cx> for FsRemoveDirAllAction
where
    Cx: Sync,
{
    type Output = FsRemoveDirAllOutput;
    type Error = FsRemoveDirAllError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FsRemoveDirAllOutput {
                    path,
                    removed: false,
                });
            }
            Err(source) => {
                return Err(FsRemoveDirAllError::InspectPath { path, source });
            }
        };

        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(FsRemoveDirAllError::RefuseSymlink { path });
        }
        if !file_type.is_dir() {
            return Err(FsRemoveDirAllError::PathIsNotDirectory { path });
        }

        std::fs::remove_dir_all(&path).map_err(|source| FsRemoveDirAllError::RemoveDirectory {
            path: path.clone(),
            source,
        })?;

        Ok(FsRemoveDirAllOutput {
            path,
            removed: true,
        })
    }
}

/// Result returned after a governed recursive directory removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRemoveDirAllOutput {
    pub path: PathBuf,
    pub removed: bool,
}
