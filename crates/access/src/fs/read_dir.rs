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
    path::{FsPathKind, FsPathPolicyContext, FsResolutionContext, GrantedPath},
};

#[cfg(test)]
mod tests;

const FS_READ_DIR_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

#[derive(Debug, Error)]
pub enum FsReadDirError {
    #[error(transparent)]
    Path(#[from] super::path::FsPathError),
    #[error(transparent)]
    Authorization(#[from] PolicyGrantError),
    #[error("failed to read directory {path}: {source}", path = .path.display())]
    ReadDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to inspect path {path}: {source}", path = .path.display())]
    InspectPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Typed action for reading one immediate filesystem directory listing.
///
/// This is separate from `FsGlobAction`: discovery/import flows often need the
/// direct children of one directory, not a recursive pattern search. Keeping
/// that distinction in the action prevents callers from broadening the read
/// surface and filtering the result outside access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadDirAction {
    root: GrantedPath,
    max_entries: usize,
}

impl FsReadDirAction {
    #[must_use]
    pub fn new(root: GrantedPath, max_entries: usize) -> Self {
        Self { root, max_entries }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    #[must_use]
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }
}

impl ActionMeta for FsReadDirAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.read_dir",
            operation: Cow::Borrowed("read_dir"),
            required_capabilities: Cow::Borrowed(&FS_READ_DIR_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.root.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "root": self.root.as_path().display().to_string(),
            "max_entries": self.max_entries,
        }))
    }
}

/// Terminal directory-listing policy installed after path policy.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsReadDirAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsReadDirAction> for FsReadDirAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-read-dir-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsReadDirAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.read_dir reached terminal allow policy".into()),
            reason: "filesystem directory listing allowed after path policy".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Read the immediate children of one governed directory.
    ///
    /// This is narrower than `glob_paths`: callers get one directory's direct
    /// entries, which is the shape migration discovery needs before it can move
    /// off direct `std::fs::read_dir`.
    pub async fn read_dir(
        self,
        root: impl AsRef<Path>,
        max_entries: usize,
    ) -> Result<FsReadDirOutput, FsReadDirError> {
        let root = self.grant_target_path(root).await?;

        let action = FsReadDirAction::new(root, max_entries);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized immediate directory listing.
///
/// This intentionally does not recurse. Callers that need recursive matching
/// should use `FsGlobAction`; import/discovery code can use this narrower
/// primitive when it only needs direct children.
#[async_trait]
impl<Cx> Action<Cx> for FsReadDirAction
where
    Cx: Sync,
{
    type Output = FsReadDirOutput;
    type Error = FsReadDirError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let root = action.root().to_path_buf();
        if action.max_entries() == 0 {
            return Ok(FsReadDirOutput {
                root,
                entries: Vec::new(),
                truncated: true,
            });
        }

        let mut children = std::fs::read_dir(&root)
            .map_err(|source| FsReadDirError::ReadDirectory {
                path: root.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| FsReadDirError::ReadDirectory {
                path: root.clone(),
                source,
            })?;
        children.sort_by_key(std::fs::DirEntry::path);

        let mut entries = Vec::new();
        for child in children {
            let path = child.path();
            let file_type = child
                .file_type()
                .map_err(|source| FsReadDirError::InspectPath {
                    path: path.clone(),
                    source,
                })?;
            let Some(kind) = FsPathKind::from_file_type(file_type) else {
                continue;
            };
            let name = child.file_name().to_string_lossy().to_string();
            entries.push(FsReadDirEntry { path, name, kind });
            if entries.len() >= action.max_entries() {
                return Ok(FsReadDirOutput {
                    root,
                    entries,
                    truncated: true,
                });
            }
        }

        Ok(FsReadDirOutput {
            root,
            entries,
            truncated: false,
        })
    }
}

/// Result returned after a governed immediate directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadDirOutput {
    pub root: PathBuf,
    pub entries: Vec<FsReadDirEntry>,
    pub truncated: bool,
}

/// One entry returned by [`FsReadDirAction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadDirEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: FsPathKind,
}
