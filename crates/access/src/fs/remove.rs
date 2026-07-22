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

const FS_REMOVE_FILE_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];

#[derive(Debug, Error)]
pub enum FsRemoveFileError {
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
    #[error("path {path} is a directory, not a file", path = .path.display())]
    PathIsDirectory { path: PathBuf },
    #[error("failed to remove file {path}: {source}", path = .path.display())]
    RemoveFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Typed action for removing one governed file or symlink.
///
/// Deletion needs final-component no-follow semantics, so this action consumes
/// `GrantedEntryPath` rather than the canonical-target `GrantedPath` used by
/// read and write operations. Path policy therefore authorizes the directory
/// entry that the side-effect boundary will actually remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRemoveFileAction {
    path: GrantedEntryPath,
}

impl FsRemoveFileAction {
    #[must_use]
    pub fn new(path: GrantedEntryPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsRemoveFileAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.remove_file",
            operation: Cow::Borrowed("remove_file"),
            required_capabilities: Cow::Borrowed(&FS_REMOVE_FILE_REQUIRED_CAPABILITIES),
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

/// Terminal file-removal policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsRemoveFileAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsRemoveFileAction> for FsRemoveFileAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-remove-file-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsRemoveFileAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.remove_file reached terminal allow policy".into()),
            reason: "filesystem file removal allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Remove one file or symlink through entry-path and write policy.
    ///
    /// The final component is deliberately not followed, so removing a symlink
    /// deletes the link itself rather than its target.
    pub async fn remove_file(
        self,
        path: impl AsRef<Path>,
    ) -> Result<FsRemoveFileOutput, FsRemoveFileError> {
        let path = self.grant_entry_path(path).await?;

        let action = FsRemoveFileAction::new(path);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized file or symlink removal.
///
/// The deletion path was prepared with final-component no-follow semantics, so
/// `remove_file` deletes a symlink itself while ancestor symlinks are reflected
/// in the policy-visible path facts.
#[async_trait]
impl<Cx> Action<Cx> for FsRemoveFileAction
where
    Cx: Sync,
{
    type Output = FsRemoveFileOutput;
    type Error = FsRemoveFileError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FsRemoveFileOutput {
                    path,
                    removed: false,
                    kind: None,
                });
            }
            Err(source) => {
                return Err(FsRemoveFileError::InspectPath { path, source });
            }
        };

        let file_type = metadata.file_type();
        if file_type.is_dir() {
            return Err(FsRemoveFileError::PathIsDirectory { path });
        }
        let kind = if file_type.is_symlink() {
            FsRemoveFileKind::Symlink
        } else {
            FsRemoveFileKind::File
        };

        match std::fs::remove_file(&path) {
            Ok(()) => Ok(FsRemoveFileOutput {
                path,
                removed: true,
                kind: Some(kind),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FsRemoveFileOutput {
                path,
                removed: false,
                kind: None,
            }),
            Err(source) => Err(FsRemoveFileError::RemoveFile { path, source }),
        }
    }
}

/// Result returned after a governed file removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRemoveFileOutput {
    pub path: PathBuf,
    pub removed: bool,
    pub kind: Option<FsRemoveFileKind>,
}

/// Kind of filesystem entry removed by [`FsRemoveFileAction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsRemoveFileKind {
    File,
    Symlink,
}
