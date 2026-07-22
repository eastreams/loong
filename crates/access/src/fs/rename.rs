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
    write::FsWriteOptions,
};

#[cfg(test)]
mod tests;

const FS_RENAME_PATH_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];

#[derive(Debug, Error)]
pub enum FsRenameError {
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
    #[error("failed to create parent directory {path}: {source}", path = .path.display())]
    CreateParentDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("path {path} already exists; overwrite is required", path = .path.display())]
    PathExistsRequiresOverwrite { path: PathBuf },
    #[error(
        "failed to rename {source_path} to {destination_path}: {source}",
        source_path = .source_path.display(),
        destination_path = .destination_path.display()
    )]
    RenamePath {
        source_path: PathBuf,
        destination_path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Typed action for renaming one governed filesystem path.
///
/// Rename uses final-component no-follow facts for both source and
/// destination. That lets access move a symlink or staged directory entry as
/// the entry itself instead of silently turning the final component into the
/// symlink target during path resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRenameAction {
    source: GrantedEntryPath,
    destination: GrantedEntryPath,
    options: FsWriteOptions,
}

impl FsRenameAction {
    #[must_use]
    pub fn new(
        source: GrantedEntryPath,
        destination: GrantedEntryPath,
        options: FsWriteOptions,
    ) -> Self {
        Self {
            source,
            destination,
            options,
        }
    }

    #[must_use]
    pub fn source_path(&self) -> &Path {
        self.source.as_path()
    }

    #[must_use]
    pub fn destination_path(&self) -> &Path {
        self.destination.as_path()
    }

    #[must_use]
    pub fn options(&self) -> FsWriteOptions {
        self.options
    }
}

impl ActionMeta for FsRenameAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.rename",
            operation: Cow::Borrowed("rename_path"),
            required_capabilities: Cow::Borrowed(&FS_RENAME_PATH_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(
            format!(
                "{} -> {}",
                self.source_path().display(),
                self.destination_path().display()
            )
            .into(),
        )
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "source": self.source_path().display().to_string(),
            "destination": self.destination_path().display().to_string(),
            "create_dirs": self.options.create_dirs,
            "overwrite": self.options.overwrite,
        }))
    }
}

/// Terminal rename policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsRenameAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsRenameAction> for FsRenameAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-rename-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsRenameAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.rename reached terminal allow policy".into()),
            reason: "filesystem rename allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Rename one governed filesystem entry through no-follow path policy.
    ///
    /// This is for staged install/update flows that need an entry move rather
    /// than file-byte copy. Both source and destination are resolved with
    /// final-component no-follow semantics before policy grants the rename.
    pub async fn rename_path(
        self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        options: FsWriteOptions,
    ) -> Result<FsRenameOutput, FsRenameError> {
        let source = self.grant_entry_path(source).await?;
        let destination = self.grant_entry_path(destination).await?;

        let action = FsRenameAction::new(source, destination, options);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized filesystem rename.
///
/// This is the primitive skills lifecycle migration needs for staged installs:
/// prepare a destination tree, then move the governed entry into place without
/// exposing a raw `std::fs::rename` call to app orchestration.
#[async_trait]
impl<Cx> Action<Cx> for FsRenameAction
where
    Cx: Sync,
{
    type Output = FsRenameOutput;
    type Error = FsRenameError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let source = action.source_path().to_path_buf();
        let destination = action.destination_path().to_path_buf();
        let options = action.options();

        if options.create_dirs
            && let Some(parent) = destination.parent()
        {
            std::fs::create_dir_all(parent).map_err(|source| {
                FsRenameError::CreateParentDirectory {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }

        let overwritten = path_entry_exists_no_follow(&destination)?;
        if overwritten && !options.overwrite {
            return Err(FsRenameError::PathExistsRequiresOverwrite { path: destination });
        }

        std::fs::rename(&source, &destination).map_err(|source_error| {
            FsRenameError::RenamePath {
                source_path: source.clone(),
                destination_path: destination.clone(),
                source: source_error,
            }
        })?;

        Ok(FsRenameOutput {
            source,
            destination,
            overwritten,
        })
    }
}

fn path_entry_exists_no_follow(path: &Path) -> Result<bool, FsRenameError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(FsRenameError::InspectPath {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Result returned after a governed filesystem rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRenameOutput {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub overwritten: bool,
}
