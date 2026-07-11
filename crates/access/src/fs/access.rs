use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use loong_core::{
    error::AuthorizationError,
    policy::{action::Action, context::ContextFactory, engine::PolicyEngine, grant::Granted},
};
use thiserror::Error;

use super::{
    FsResolutionContext,
    action::{FsGlobAction, FsReadAction, FsResolvePathAction},
    error::FsActionError,
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

/// Execute an already-authorized path glob.
///
/// Directory traversal, file-type inspection, and glob matching all live here
/// so the concrete tool receives only the granted result set.
#[async_trait]
impl<Cx> Action<Cx> for FsGlobAction
where
    Cx: Sync,
{
    type Output = FsGlobOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let root = action.root().to_path_buf();
        if action.max_results() == 0 {
            return Ok(FsGlobOutput {
                root,
                matches: Vec::new(),
                truncated: true,
            });
        }

        let matcher = GlobMatcher::new(action.pattern())?;
        let mut matches = Vec::new();
        let mut queue = VecDeque::from([root.clone()]);

        while let Some(directory) = queue.pop_front() {
            let mut children = std::fs::read_dir(&directory)
                .map_err(|source| FsAccessError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| FsAccessError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?;
            children.sort_by_key(std::fs::DirEntry::path);

            for child in children {
                let child_path = child.path();
                let file_type = child
                    .file_type()
                    .map_err(|source| FsAccessError::InspectPath {
                        path: child_path.clone(),
                        source,
                    })?;
                let is_directory = file_type.is_dir();
                let relative_path = child_path.strip_prefix(&root).map_err(|source| {
                    FsAccessError::RenderRelativePath {
                        root: root.clone(),
                        path: child_path.clone(),
                        source,
                    }
                })?;
                let relative_path = relative_path.to_string_lossy().replace('\\', "/");

                if let Some(kind) = FsPathKind::from_file_type(file_type)
                    && matcher.is_match(relative_path.as_str())
                    && (!is_directory || action.include_directories())
                {
                    matches.push(FsPathMatch {
                        path: child_path.clone(),
                        relative_path,
                        kind,
                    });

                    if matches.len() >= action.max_results() {
                        return Ok(FsGlobOutput {
                            root,
                            matches,
                            truncated: true,
                        });
                    }
                }

                if is_directory {
                    queue.push_back(child_path);
                }
            }
        }

        Ok(FsGlobOutput {
            root,
            matches,
            truncated: false,
        })
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

/// Result returned by a governed path glob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsGlobOutput {
    pub root: PathBuf,
    pub matches: Vec<FsPathMatch>,
    pub truncated: bool,
}

/// One path returned by a governed path glob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsPathMatch {
    pub path: PathBuf,
    pub relative_path: String,
    pub kind: FsPathKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsPathKind {
    File,
    Directory,
}

impl FsPathKind {
    fn from_file_type(file_type: std::fs::FileType) -> Option<Self> {
        if file_type.is_dir() {
            return Some(Self::Directory);
        }
        if file_type.is_file() || file_type.is_symlink() {
            return Some(Self::File);
        }
        None
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
    #[error("failed to read file {path}: {source}", path = .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

struct GlobMatcher {
    regexes: Vec<regex::Regex>,
}

impl GlobMatcher {
    fn new(pattern: &str) -> Result<Self, FsAccessError> {
        let regexes = Self::split_patterns(pattern)
            .into_iter()
            .map(|candidate| {
                let regex_pattern = Self::pattern_to_regex(candidate.as_str());
                regex::Regex::new(regex_pattern.as_str()).map_err(|source| {
                    FsAccessError::InvalidGlobPattern {
                        pattern: pattern.to_owned(),
                        source,
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { regexes })
    }

    fn is_match(&self, relative_path: &str) -> bool {
        self.regexes
            .iter()
            .any(|regex| regex.is_match(relative_path))
    }

    fn split_patterns(pattern: &str) -> Vec<String> {
        let trimmed = pattern.trim();
        if let Some(inner) = trimmed
            .strip_prefix('{')
            .and_then(|candidate| candidate.strip_suffix('}'))
        {
            let parts = inner
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return parts;
            }
        }

        let pipe_parts = trimmed
            .split('|')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if pipe_parts.len() > 1 {
            return pipe_parts;
        }

        vec![trimmed.to_owned()]
    }

    fn pattern_to_regex(pattern: &str) -> String {
        let mut regex_pattern = String::from("^");
        let mut chars = pattern.chars().peekable();

        while let Some(character) = chars.next() {
            match character {
                '*' => {
                    let next_is_star = chars.peek() == Some(&'*');
                    if next_is_star {
                        let _ = chars.next();
                        if chars.peek() == Some(&'/') {
                            let _ = chars.next();
                            regex_pattern.push_str("(?:.*/)?");
                        } else {
                            regex_pattern.push_str(".*");
                        }
                    } else {
                        regex_pattern.push_str("[^/]*");
                    }
                }
                '?' => regex_pattern.push_str("[^/]"),
                '\\' => regex_pattern.push('/'),
                value => {
                    if ".+()^$|{}[]\\".contains(value) {
                        regex_pattern.push('\\');
                    }
                    regex_pattern.push(value);
                }
            }
        }

        regex_pattern.push('$');
        regex_pattern
    }
}
