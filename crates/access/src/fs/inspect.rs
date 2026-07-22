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

const FS_INSPECT_PATH_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

#[derive(Debug, Error)]
pub enum FsInspectPathError {
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
}

/// Typed action for observing one governed filesystem path.
///
/// Existence and file-kind metadata leak filesystem state, so inspect is a
/// read-family action even though it does not read file contents. Write actions
/// keep their own write-authorized checks for overwrite safety.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInspectPathAction {
    path: GrantedPath,
}

impl FsInspectPathAction {
    #[must_use]
    pub fn new(path: GrantedPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsInspectPathAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.inspect_path",
            operation: Cow::Borrowed("inspect_path"),
            required_capabilities: Cow::Borrowed(&FS_INSPECT_PATH_REQUIRED_CAPABILITIES),
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

/// Terminal path-inspection policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsInspectPathAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsInspectPathAction> for FsInspectPathAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-inspect-path-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsInspectPathAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.inspect_path reached terminal allow policy".into()),
            reason: "filesystem path inspection allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Inspect one path through resolution, path, and inspect policy.
    ///
    /// This is for callers that need existence or file-kind observations before
    /// a later access-backed operation. It intentionally reports only metadata,
    /// not file contents.
    pub async fn inspect_path(
        self,
        path: impl AsRef<Path>,
    ) -> Result<FsInspectPathOutput, FsInspectPathError> {
        let path = self.grant_target_path(path).await?;

        let action = FsInspectPathAction::new(path);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized path inspection.
///
/// This observes filesystem metadata but does not read file contents. It still
/// consumes `Granted<FsInspectPathAction>` because existence and type metadata
/// are policy-governed read-family information.
#[async_trait]
impl<Cx> Action<Cx> for FsInspectPathAction
where
    Cx: Sync,
{
    type Output = FsInspectPathOutput;
    type Error = FsInspectPathError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let kind = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => FsPathKind::from_file_type(metadata.file_type()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(FsInspectPathError::InspectPath { path, source });
            }
        };

        Ok(FsInspectPathOutput { path, kind })
    }
}

/// Result returned after a governed filesystem path inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInspectPathOutput {
    pub path: PathBuf,
    pub kind: Option<FsPathKind>,
}
