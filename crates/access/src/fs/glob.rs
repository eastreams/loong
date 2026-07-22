use std::{
    borrow::Cow,
    collections::VecDeque,
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

const FS_GLOB_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

#[derive(Debug, Error)]
pub enum FsGlobError {
    #[error(transparent)]
    Path(#[from] super::path::FsPathError),
    #[error(transparent)]
    Authorization(#[from] PolicyGrantError),
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
}

/// Typed action for listing filesystem paths by glob pattern.
///
/// Like `FsReadAction`, this is a data-leaking filesystem operation and
/// therefore consumes a `GrantedPath` root. Pattern matching happens inside the
/// access side-effect boundary so tools do not receive a broader directory
/// listing than the action payload describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsGlobAction {
    root: GrantedPath,
    pattern: String,
    include_directories: bool,
    max_results: usize,
}

impl FsGlobAction {
    #[must_use]
    pub fn new(
        root: GrantedPath,
        pattern: impl Into<String>,
        include_directories: bool,
        max_results: usize,
    ) -> Self {
        Self {
            root,
            pattern: pattern.into(),
            include_directories,
            max_results,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    #[must_use]
    pub fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    #[must_use]
    pub fn include_directories(&self) -> bool {
        self.include_directories
    }

    #[must_use]
    pub fn max_results(&self) -> usize {
        self.max_results
    }
}

impl ActionMeta for FsGlobAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.glob",
            operation: Cow::Borrowed("glob_paths"),
            required_capabilities: Cow::Borrowed(&FS_GLOB_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.root.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "root": self.root.as_path().display().to_string(),
            "pattern": self.pattern,
            "include_directories": self.include_directories,
            "max_results": self.max_results,
        }))
    }
}

/// Terminal glob policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsGlobAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsGlobAction> for FsGlobAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-glob-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsGlobAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.glob reached terminal allow policy".into()),
            reason: "filesystem glob allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
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
    ) -> Result<FsGlobOutput, FsGlobError> {
        let root = self.grant_target_path(root).await?;

        let action = FsGlobAction::new(root, pattern, include_directories, max_results);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
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
    type Error = FsGlobError;

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

        let matcher = GlobMatcher::new(action.pattern()).map_err(|source| {
            FsGlobError::InvalidGlobPattern {
                pattern: action.pattern().to_owned(),
                source,
            }
        })?;
        let mut matches = Vec::new();
        let mut queue = VecDeque::from([root.clone()]);

        while let Some(directory) = queue.pop_front() {
            let mut children = std::fs::read_dir(&directory)
                .map_err(|source| FsGlobError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| FsGlobError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?;
            children.sort_by_key(std::fs::DirEntry::path);

            for child in children {
                let child_path = child.path();
                let file_type = child
                    .file_type()
                    .map_err(|source| FsGlobError::InspectPath {
                        path: child_path.clone(),
                        source,
                    })?;
                let is_directory = file_type.is_dir();
                let relative_path = child_path.strip_prefix(&root).map_err(|source| {
                    FsGlobError::RenderRelativePath {
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

pub(in crate::fs) struct GlobMatcher {
    regexes: Vec<regex::Regex>,
}

impl GlobMatcher {
    pub(in crate::fs) fn new(pattern: &str) -> Result<Self, regex::Error> {
        let regexes = Self::split_patterns(pattern)
            .into_iter()
            .map(|candidate| {
                let regex_pattern = Self::pattern_to_regex(candidate.as_str());
                regex::Regex::new(regex_pattern.as_str())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { regexes })
    }

    pub(in crate::fs) fn is_match(&self, relative_path: &str) -> bool {
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
