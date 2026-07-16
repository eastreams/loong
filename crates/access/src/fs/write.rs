use std::{
    borrow::Cow,
    io::Write as _,
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
};

const FS_WRITE_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];

/// Policy-visible options shared by governed write operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsWriteOptions {
    pub create_dirs: bool,
    pub overwrite: bool,
}

/// Typed action for writing bytes to one governed filesystem path.
///
/// The action carries bytes because the access side-effect boundary needs them
/// to perform the write. Its audit payload records only byte count and flags,
/// not file content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWriteAction {
    path: GrantedPath,
    bytes: Vec<u8>,
    options: FsWriteOptions,
}

impl FsWriteAction {
    #[must_use]
    pub fn new(path: GrantedPath, bytes: Vec<u8>, options: FsWriteOptions) -> Self {
        Self {
            path,
            bytes,
            options,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    #[must_use]
    pub fn options(&self) -> FsWriteOptions {
        self.options
    }
}

impl ActionMeta for FsWriteAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.write",
            operation: Cow::Borrowed("write_file"),
            required_capabilities: Cow::Borrowed(&FS_WRITE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
            "byte_count": self.bytes.len(),
            "create_dirs": self.options.create_dirs,
            "overwrite": self.options.overwrite,
        }))
    }
}

/// Typed action for atomically replacing one governed filesystem path.
///
/// This is separate from `FsWriteAction` because migration manifests and
/// rollback records need a stronger failure mode: stage bytes next to the
/// target, then replace the target only after the staged file is complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsAtomicWriteAction {
    path: GrantedPath,
    bytes: Vec<u8>,
    options: FsWriteOptions,
}

impl FsAtomicWriteAction {
    #[must_use]
    pub fn new(path: GrantedPath, bytes: Vec<u8>, options: FsWriteOptions) -> Self {
        Self {
            path,
            bytes,
            options,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    #[must_use]
    pub fn options(&self) -> FsWriteOptions {
        self.options
    }
}

impl ActionMeta for FsAtomicWriteAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.atomic_write",
            operation: Cow::Borrowed("write_file_atomically"),
            required_capabilities: Cow::Borrowed(&FS_WRITE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
            "byte_count": self.bytes.len(),
            "create_dirs": self.options.create_dirs,
            "overwrite": self.options.overwrite,
        }))
    }
}

/// Terminal policy for ordinary governed writes.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsWriteAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsWriteAction> for FsWriteAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-write-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsWriteAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.write reached terminal allow policy".into()),
            reason: "filesystem write allowed after configured deny policies".into(),
        }
    }
}

/// Terminal policy for atomic governed writes.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsAtomicWriteAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsAtomicWriteAction> for FsAtomicWriteAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-atomic-write-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsAtomicWriteAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.atomic_write reached terminal allow policy".into()),
            reason: "filesystem atomic write allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext,
{
    /// Write bytes through resolution, path authorization, and write policy.
    ///
    /// This only prepares the access-backed primitive. App write/edit tools own
    /// payload parsing and response preview, never the filesystem side effect.
    pub async fn write_file(
        self,
        path: impl AsRef<Path>,
        bytes: impl Into<Vec<u8>>,
        options: FsWriteOptions,
    ) -> Result<FsWriteOutput, FsAccessError> {
        let path = self.grant_target_path(path).await?;

        let action = FsWriteAction::new(path, bytes.into(), options);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }

    /// Atomically write through resolution, path authorization, and write policy.
    ///
    /// This is for manifests and rollback records where a failed write must not
    /// leave the previous target truncated. The final replacement still happens
    /// inside access after policy grants the concrete action.
    pub async fn write_file_atomically(
        self,
        path: impl AsRef<Path>,
        bytes: impl Into<Vec<u8>>,
        options: FsWriteOptions,
    ) -> Result<FsWriteOutput, FsAccessError> {
        let path = self.grant_target_path(path).await?;

        let action = FsAtomicWriteAction::new(path, bytes.into(), options);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized fs write.
///
/// This consumes `Granted<FsWriteAction>` so write side effects cannot be
/// reached with a raw path or an ungranted action.
#[async_trait]
impl<Cx> Action<Cx> for FsWriteAction
where
    Cx: Sync,
{
    type Output = FsWriteOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let options = action.options();

        if symlink_metadata_is_symlink(&path)? {
            return Err(FsAccessError::RefuseSymlink { path });
        }
        if path.is_dir() {
            return Err(FsAccessError::PathIsDirectory { path });
        }
        if options.create_dirs
            && let Some(parent) = path.parent()
        {
            std::fs::create_dir_all(parent).map_err(|source| {
                FsAccessError::CreateParentDirectory {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }

        let overwritten = path
            .try_exists()
            .map_err(|source| FsAccessError::InspectPath {
                path: path.clone(),
                source,
            })?;
        if overwritten && !options.overwrite {
            return Err(FsAccessError::FileExistsRequiresOverwrite { path });
        }

        let mut file_options = std::fs::OpenOptions::new();
        file_options.write(true);
        if options.overwrite {
            file_options.create(true).truncate(true);
        } else {
            file_options.create_new(true);
        }
        let mut file = file_options.open(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists && !options.overwrite {
                return FsAccessError::FileExistsRequiresOverwrite { path: path.clone() };
            }

            FsAccessError::OpenWriteFile {
                path: path.clone(),
                source,
            }
        })?;
        file.write_all(action.bytes())
            .map_err(|source| FsAccessError::WriteFile {
                path: path.clone(),
                source,
            })?;

        Ok(FsWriteOutput {
            path,
            bytes_written: action.bytes().len(),
            overwritten,
        })
    }
}

/// Execute an already-authorized atomic fs write.
///
/// This consumes `Granted<FsAtomicWriteAction>` so manifest-style replacement
/// cannot be performed with an ungranted target path.
#[async_trait]
impl<Cx> Action<Cx> for FsAtomicWriteAction
where
    Cx: Sync,
{
    type Output = FsWriteOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let options = action.options();

        if symlink_metadata_is_symlink(&path)? {
            return Err(FsAccessError::RefuseSymlink { path });
        }
        if path.is_dir() {
            return Err(FsAccessError::PathIsDirectory { path });
        }
        if options.create_dirs
            && let Some(parent) = path.parent()
        {
            std::fs::create_dir_all(parent).map_err(|source| {
                FsAccessError::CreateParentDirectory {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }

        let overwritten = path
            .try_exists()
            .map_err(|source| FsAccessError::InspectPath {
                path: path.clone(),
                source,
            })?;
        if overwritten && !options.overwrite {
            return Err(FsAccessError::FileExistsRequiresOverwrite { path });
        }

        let parent = path.parent().unwrap_or(Path::new("."));
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|source| {
            FsAccessError::OpenWriteFile {
                path: path.clone(),
                source,
            }
        })?;
        staged
            .as_file_mut()
            .write_all(action.bytes())
            .map_err(|source| FsAccessError::WriteFile {
                path: path.clone(),
                source,
            })?;
        staged
            .as_file_mut()
            .sync_all()
            .map_err(|source| FsAccessError::WriteFile {
                path: path.clone(),
                source,
            })?;
        staged
            .persist(&path)
            .map_err(|error| FsAccessError::WriteFile {
                path: path.clone(),
                source: error.error,
            })?;

        Ok(FsWriteOutput {
            path,
            bytes_written: action.bytes().len(),
            overwritten,
        })
    }
}

// Both write modes must classify missing targets and symlinks identically
// before they choose different persistence strategies.
fn symlink_metadata_is_symlink(path: &Path) -> Result<bool, FsAccessError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(FsAccessError::InspectPath {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Result returned after a governed fs write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWriteOutput {
    pub path: PathBuf,
    pub bytes_written: usize,
    pub overwritten: bool,
}
