use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyDecision, PolicyGrant};
use loong_core::{
    error::AuthorizationError,
    policy::{
        action::{Action, ActionMeta, ActionMetadata},
        context::ContextFactory,
        engine::PolicyEngine,
        grant::Granted,
        policy::Policy,
    },
};
use serde_json::{Value, json};

use super::{
    access::{FsAccess, FsAccessError},
    path::{FsResolutionContext, GrantedPath},
    write::FsWriteOptions,
};

const FS_COPY_FILE_REQUIRED_CAPABILITIES: [Capability; 2] =
    [Capability::FilesystemRead, Capability::FilesystemWrite];

/// Typed action for copying bytes between two governed filesystem paths.
///
/// Copy is one action instead of a read followed by an app-side write, so file
/// bytes never leave the access side-effect boundary during backup or restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCopyFileAction {
    source: GrantedPath,
    destination: GrantedPath,
    options: FsWriteOptions,
}

impl FsCopyFileAction {
    #[must_use]
    pub fn new(source: GrantedPath, destination: GrantedPath, options: FsWriteOptions) -> Self {
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

impl ActionMeta for FsCopyFileAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.copy_file",
            operation: Cow::Borrowed("copy_file"),
            required_capabilities: Cow::Borrowed(&FS_COPY_FILE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(
            format!(
                "{} -> {}",
                self.source.as_path().display(),
                self.destination.as_path().display()
            )
            .into(),
        )
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "source": self.source.as_path().display().to_string(),
            "destination": self.destination.as_path().display().to_string(),
            "create_dirs": self.options.create_dirs,
            "overwrite": self.options.overwrite,
        }))
    }
}

/// Terminal copy policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsCopyFileAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsCopyFileAction> for FsCopyFileAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-copy-file-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsCopyFileAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.copy_file reached terminal allow policy".into()),
            reason: "filesystem file copy allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext,
{
    /// Copy one file through source/destination path policy and copy policy.
    pub async fn copy_file(
        self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        options: FsWriteOptions,
    ) -> Result<FsCopyFileOutput, FsAccessError> {
        let source = self.grant_target_path(source).await?;
        let destination = self.grant_target_path(destination).await?;

        let action = FsCopyFileAction::new(source, destination, options);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized file copy.
///
/// Backup/restore flows should use this boundary instead of reading bytes into
/// app orchestration and then writing them back out.
#[async_trait]
impl<Cx> Action<Cx> for FsCopyFileAction
where
    Cx: Sync,
{
    type Output = FsCopyFileOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let source = action.source_path().to_path_buf();
        let destination = action.destination_path().to_path_buf();
        let options = action.options();

        if destination.is_dir() {
            return Err(FsAccessError::PathIsDirectory { path: destination });
        }
        if options.create_dirs
            && let Some(parent) = destination.parent()
        {
            std::fs::create_dir_all(parent).map_err(|source| {
                FsAccessError::CreateParentDirectory {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }

        let overwritten =
            destination
                .try_exists()
                .map_err(|source| FsAccessError::InspectPath {
                    path: destination.clone(),
                    source,
                })?;
        if overwritten && !options.overwrite {
            return Err(FsAccessError::FileExistsRequiresOverwrite { path: destination });
        }

        let bytes_copied = std::fs::copy(&source, &destination).map_err(|source_error| {
            FsAccessError::CopyFile {
                source_path: source.clone(),
                destination_path: destination.clone(),
                source: source_error,
            }
        })?;

        Ok(FsCopyFileOutput {
            source,
            destination,
            bytes_copied,
            overwritten,
        })
    }
}

/// Result returned after a governed file copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCopyFileOutput {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes_copied: u64,
    pub overwritten: bool,
}
