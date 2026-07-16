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
};

const FS_CREATE_DIR_ALL_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];

/// Typed action for creating one governed directory tree.
///
/// Directory creation is a write side effect, so the action declares
/// `FilesystemWrite` and can only run after path policy has produced a
/// `GrantedPath`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCreateDirAllAction {
    path: GrantedPath,
}

impl FsCreateDirAllAction {
    #[must_use]
    pub fn new(path: GrantedPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsCreateDirAllAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.create_dir_all",
            operation: Cow::Borrowed("create_dir_all"),
            required_capabilities: Cow::Borrowed(&FS_CREATE_DIR_ALL_REQUIRED_CAPABILITIES),
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

/// Terminal directory-creation policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsCreateDirAllAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsCreateDirAllAction> for FsCreateDirAllAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-create-dir-all-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsCreateDirAllAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.create_dir_all reached terminal allow policy".into()),
            reason: "filesystem directory creation allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext,
{
    /// Create a directory tree through resolution, path, and write policy.
    pub async fn create_dir_all(
        self,
        path: impl AsRef<Path>,
    ) -> Result<FsCreateDirAllOutput, FsAccessError> {
        let path = self.grant_target_path(path).await?;

        let action = FsCreateDirAllAction::new(path);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized directory creation.
///
/// This is a filesystem write side effect and therefore consumes
/// `Granted<FsCreateDirAllAction>` instead of accepting a raw path.
#[async_trait]
impl<Cx> Action<Cx> for FsCreateDirAllAction
where
    Cx: Sync,
{
    type Output = FsCreateDirAllOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let already_exists = path
            .try_exists()
            .map_err(|source| FsAccessError::InspectPath {
                path: path.clone(),
                source,
            })?;

        std::fs::create_dir_all(&path).map_err(|source| FsAccessError::CreateDirectory {
            path: path.clone(),
            source,
        })?;

        Ok(FsCreateDirAllOutput {
            path,
            already_exists,
        })
    }
}

/// Result returned after a governed directory creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCreateDirAllOutput {
    pub path: PathBuf,
    pub already_exists: bool,
}
