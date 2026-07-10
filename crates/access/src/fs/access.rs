use std::path::{Path, PathBuf};

use async_trait::async_trait;
use loong_core::{
    error::AuthorizationError,
    policy::{
        action::Action,
        context::{ContextFactory, FsAccessContext},
        engine::PolicyEngine,
        grant::Granted,
    },
};
use thiserror::Error;

use super::{
    action::{FsReadAction, FsResolvePathAction},
    error::FsActionError,
    path::{CanonicalPath, GrantedPath},
};

/// Filesystem access facade.
///
/// This module is the side-effect boundary for fs reads. Callers provide a raw
/// path; `FsAccess` resolves it, builds the typed action, asks policy for a
/// grant, consumes that grant, and only then reads from disk.
pub struct FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
{
    policy_engine: &'a P,
    policy_context: &'a C::Cx<'ctx>,
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
{
    #[inline(always)]
    #[must_use]
    pub fn new(policy_engine: &'a P, policy_context: &'a C::Cx<'ctx>) -> Self {
        Self {
            policy_engine,
            policy_context,
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsAccessContext,
{
    /// Read a file after path resolution and typed policy grant.
    pub async fn read_file(self, path: impl AsRef<Path>) -> Result<FsReadOutput, FsAccessError> {
        let resolve_action = FsResolvePathAction::new(path);
        let resolve_grant = self
            .policy_engine
            .grant(self.policy_context, resolve_action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        let path = resolve_grant.granted.run(self.policy_context).await?;

        let action = FsReadAction::new(path);
        let grant = self
            .policy_engine
            .grant(self.policy_context, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.granted.run(self.policy_context).await
    }
}

/// Resolve an already-authorized raw fs path into a governed path.
///
/// This action may observe path metadata through canonicalization and symlink
/// resolution, but it does not read file contents. The returned `GrantedPath`
/// is the only public input accepted by concrete fs side-effect actions.
#[async_trait]
impl<Cx> Action<Cx> for FsResolvePathAction
where
    Cx: FsAccessContext + Sync,
{
    type Output = GrantedPath;
    type Error = FsActionError;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = CanonicalPath::resolve(
            action.raw_path(),
            ctx.fs_resolution_root(),
            ctx.fs_allowed_roots(),
        )?;
        Ok(GrantedPath::new(path.into_path_buf()))
    }
}

/// Execute an already-authorized fs read.
///
/// This is the concrete side-effect boundary for fs reads. It deliberately
/// consumes `Granted<FsReadAction>` so raw actions cannot reach the filesystem.
#[async_trait]
impl<Cx> Action<Cx> for FsReadAction
where
    Cx: Sync,
{
    type Output = FsReadOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|source| FsAccessError::ReadFile {
            path: path.clone(),
            source,
        })?;
        Ok(FsReadOutput { path, bytes })
    }
}

/// Bytes returned by a governed fs read.
///
/// `path` is the canonical path actually read, suitable for response metadata
/// and audit output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadOutput {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum FsAccessError {
    #[error(transparent)]
    Action(#[from] FsActionError),
    #[error(transparent)]
    Authorization(AuthorizationError),
    #[error("failed to read file {path}: {source}", path = .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
