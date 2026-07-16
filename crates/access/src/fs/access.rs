use std::path::{Path, PathBuf};

use loong_core::{
    error::AuthorizationError,
    policy::{context::ContextFactory, engine::PolicyEngine},
};
use thiserror::Error;

use super::{
    action::{
        FsContentSearchAction, FsContentSearchOptions, FsCreateDirAllAction, FsGlobAction,
        FsInspectPathAction, FsReadDirAction, FsRemoveDirAllAction, FsRemoveFileAction,
        FsRenameAction,
    },
    content_search::FsContentSearchOutput,
    directory::FsCreateDirAllOutput,
    error::FsActionError,
    glob::FsGlobOutput,
    inspect::FsInspectPathOutput,
    path::FsResolutionContext,
    read_dir::FsReadDirOutput,
    remove::FsRemoveFileOutput,
    remove_dir::FsRemoveDirAllOutput,
    rename::FsRenameOutput,
    write::FsWriteOptions,
};

/// Filesystem access facade.
///
/// This module is the side-effect boundary for governed filesystem operations.
/// Callers provide raw paths and operation inputs; `FsAccess` resolves paths,
/// builds typed actions, asks policy for grants, and only then touches disk.
pub struct FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext,
{
    pub(in crate::fs) policy_engine: &'a P,
    pub(in crate::fs) ctx: &'a C::Cx<'ctx>,
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

    /// Remove one file or symlink through remove-path policy and write policy.
    ///
    /// Unlike read/write/copy, removal does not use `GrantedPath`: the action
    /// must preserve final-component no-follow semantics so symlink deletion
    /// cannot accidentally become target deletion.
    pub async fn remove_file(
        self,
        path: impl AsRef<Path>,
    ) -> Result<FsRemoveFileOutput, FsAccessError> {
        let path = self.grant_entry_path(path).await?;

        let action = FsRemoveFileAction::new(path);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }

    /// Recursively remove one governed directory tree.
    ///
    /// This operation is intentionally distinct from `remove_file`: recursive
    /// deletion has a larger blast radius and refuses final-component symlinks.
    pub async fn remove_dir_all(
        self,
        path: impl AsRef<Path>,
    ) -> Result<FsRemoveDirAllOutput, FsAccessError> {
        let path = self.grant_entry_path(path).await?;

        let action = FsRemoveDirAllAction::new(path);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }

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
    ) -> Result<FsRenameOutput, FsAccessError> {
        let source = self.grant_entry_path(source).await?;
        let destination = self.grant_entry_path(destination).await?;

        let action = FsRenameAction::new(source, destination, options);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }

    /// Inspect one path through resolution, path, and inspect policy.
    ///
    /// This is for callers that need existence or file-kind observations before
    /// a later access-backed operation. It intentionally reports only metadata,
    /// not file contents.
    pub async fn inspect_path(
        self,
        path: impl AsRef<Path>,
    ) -> Result<FsInspectPathOutput, FsAccessError> {
        let path = self.grant_target_path(path).await?;

        let action = FsInspectPathAction::new(path);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
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
        let root = self.grant_target_path(root).await?;

        let action = FsGlobAction::new(root, pattern, include_directories, max_results);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }

    /// Read the immediate children of one governed directory.
    ///
    /// This is narrower than `glob_paths`: callers get one directory's direct
    /// entries, which is the shape migration discovery needs before it can move
    /// off direct `std::fs::read_dir`.
    pub async fn read_dir(
        self,
        root: impl AsRef<Path>,
        max_entries: usize,
    ) -> Result<FsReadDirOutput, FsAccessError> {
        let root = self.grant_target_path(root).await?;

        let action = FsReadDirAction::new(root, max_entries);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
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
        let root = self.grant_target_path(root).await?;

        let action = FsContentSearchAction::new(root, query, options);
        let grant = self
            .policy_engine
            .grant(self.ctx, action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?;
        grant.into_granted().run(self.ctx).await
    }
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
    #[error("failed to create parent directory {path}: {source}", path = .path.display())]
    CreateParentDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create directory {path}: {source}", path = .path.display())]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("path {path} is a directory, not a file", path = .path.display())]
    PathIsDirectory { path: PathBuf },
    #[error("path {path} is not a directory", path = .path.display())]
    PathIsNotDirectory { path: PathBuf },
    #[error("refusing to write through symlink {path}", path = .path.display())]
    RefuseSymlink { path: PathBuf },
    #[error("file {path} already exists; overwrite is required", path = .path.display())]
    FileExistsRequiresOverwrite { path: PathBuf },
    #[error("path {path} already exists; overwrite is required", path = .path.display())]
    PathExistsRequiresOverwrite { path: PathBuf },
    #[error("failed to open file {path} for writing: {source}", path = .path.display())]
    OpenWriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write file {path}: {source}", path = .path.display())]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "failed to copy file {source_path} to {destination_path}: {source}",
        source_path = .source_path.display(),
        destination_path = .destination_path.display()
    )]
    CopyFile {
        source_path: PathBuf,
        destination_path: PathBuf,
        #[source]
        source: std::io::Error,
    },
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
    #[error("failed to remove file {path}: {source}", path = .path.display())]
    RemoveFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to remove directory {path}: {source}", path = .path.display())]
    RemoveDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
