use std::path::{Path, PathBuf};

use async_trait::async_trait;
use loong_core::{
    error::AuthorizationError,
    policy::{action::Action, context::ContextFactory, engine::PolicyEngine, grant::Granted},
};
use thiserror::Error;

use super::{
    FsResolutionContext,
    action::{
        FsContentSearchAction, FsContentSearchOptions, FsGlobAction, FsReadAction,
        FsResolvePathAction,
    },
    content_search::FsContentSearchOutput,
    error::FsActionError,
    glob::FsGlobOutput,
    path::GrantedPath,
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
    C::Cx<'ctx>: FsResolutionContext,
{
    policy_engine: &'a P,
    ctx: &'a C::Cx<'ctx>,
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext,
{
    #[inline(always)]
    #[must_use]
    pub fn new(policy_engine: &'a P, ctx: &'a C::Cx<'ctx>) -> Self {
        Self { policy_engine, ctx }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext,
{
    /// Read a file through path-resolution policy and read policy.
    ///
    /// Access prepares the resolved-path facts because that requires
    /// filesystem observation. Kernel policy decides whether those facts are
    /// allowed, and only the granted resolve action can mint `GrantedPath`.
    pub async fn read_file(self, path: impl AsRef<Path>) -> Result<FsReadOutput, FsAccessError> {
        let resolve_action = FsResolvePathAction::resolve(path, self.ctx.fs_resolution_root())?;
        let resolve_grant = self
            .policy_engine
            .grant(self.ctx, resolve_action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        let path = resolve_grant.granted.run(self.ctx).await?;

        let action = FsReadAction::new(path);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.granted.run(self.ctx).await
    }

    /// List paths under a governed root by glob pattern.
    ///
    /// This is the access-backed primitive for the path-listing branch of
    /// `read`. Tools should not call `std::fs::read_dir` directly; they should
    /// ask access for the exact listing operation they need.
    pub async fn glob_paths(
        self,
        root: impl AsRef<Path>,
        pattern: impl Into<String>,
        include_directories: bool,
        max_results: usize,
    ) -> Result<FsGlobOutput, FsAccessError> {
        let resolve_action = FsResolvePathAction::resolve(root, self.ctx.fs_resolution_root())?;
        let resolve_grant = self
            .policy_engine
            .grant(self.ctx, resolve_action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        let root = resolve_grant.granted.run(self.ctx).await?;

        let action = FsGlobAction::new(root, pattern, include_directories, max_results);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.granted.run(self.ctx).await
    }

    /// Search file contents under a governed root.
    ///
    /// Access performs the candidate traversal and file reads. The caller only
    /// receives match metadata, so content search cannot bypass fs read policy
    /// by moving bulk file reads into a concrete tool implementation.
    pub async fn search_content(
        self,
        root: impl AsRef<Path>,
        query: impl Into<String>,
        options: FsContentSearchOptions,
    ) -> Result<FsContentSearchOutput, FsAccessError> {
        let resolve_action = FsResolvePathAction::resolve(root, self.ctx.fs_resolution_root())?;
        let resolve_grant = self
            .policy_engine
            .grant(self.ctx, resolve_action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        let root = resolve_grant.granted.run(self.ctx).await?;

        let action = FsContentSearchAction::new(root, query, options);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.granted.run(self.ctx).await
    }
}

/// Mint a governed path from an already-authorized resolution action.
///
/// Canonicalization already happened when access prepared the action facts.
/// `run` deliberately just consumes the grant and turns accepted facts into
/// `GrantedPath`, the only public input accepted by concrete fs side-effect
/// actions.
#[async_trait]
impl<Cx> Action<Cx> for FsResolvePathAction
where
    Cx: Sync,
{
    type Output = GrantedPath;
    type Error = FsActionError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        Ok(action.into_granted_path())
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
    #[error("invalid glob pattern {pattern}: {source}")]
    InvalidGlobPattern {
        pattern: String,
        #[source]
        source: regex::Error,
    },
    #[error("failed to build content search matcher for {query}: {source}")]
    BuildContentSearchRegex {
        query: String,
        #[source]
        source: regex::Error,
    },
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
    #[error(
        "failed to render relative path for {path} from {root}: {source}",
        path = .path.display(),
        root = .root.display()
    )]
    RenderRelativePath {
        root: PathBuf,
        path: PathBuf,
        #[source]
        source: std::path::StripPrefixError,
    },
    #[error("content search produced an invalid match range in {path}", path = .path.display())]
    InvalidContentMatchRange { path: PathBuf },
    #[error("failed to read file {path}: {source}", path = .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
