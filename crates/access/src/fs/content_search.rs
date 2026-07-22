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
use regex::RegexBuilder;
use serde_json::{Value, json};
use thiserror::Error;

use super::{
    access::FsAccess,
    glob::GlobMatcher,
    path::{FsPathPolicyContext, FsResolutionContext, GrantedPath},
};

#[cfg(test)]
mod tests;

const FS_CONTENT_SEARCH_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

#[derive(Debug, Error)]
pub enum FsContentSearchError {
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
    #[error("failed to read file {path}: {source}", path = .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("content search produced an invalid match range in {path}", path = .path.display())]
    InvalidContentMatchRange { path: PathBuf },
}

/// Policy-visible options for one governed content search.
///
/// Defaults and bounds belong to the caller/tool parser; access receives the
/// already-selected values and records them in the action payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsContentSearchOptions {
    pub glob: Option<String>,
    pub max_results: usize,
    pub max_bytes_per_file: usize,
    pub case_sensitive: bool,
}

/// Typed action for searching text inside files under one governed root.
///
/// Content search reads many candidate files, so the query and optional glob
/// filter belong to the action payload. Keeping matching inside access avoids
/// returning broad file contents to a tool just so it can filter them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsContentSearchAction {
    root: GrantedPath,
    query: String,
    options: FsContentSearchOptions,
}

impl FsContentSearchAction {
    #[must_use]
    pub fn new(
        root: GrantedPath,
        query: impl Into<String>,
        options: FsContentSearchOptions,
    ) -> Self {
        Self {
            root,
            query: query.into(),
            options,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    #[must_use]
    pub fn query(&self) -> &str {
        self.query.as_str()
    }

    #[must_use]
    pub fn options(&self) -> &FsContentSearchOptions {
        &self.options
    }
}

impl ActionMeta for FsContentSearchAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.content_search",
            operation: Cow::Borrowed("search_content"),
            required_capabilities: Cow::Borrowed(&FS_CONTENT_SEARCH_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.root.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "root": self.root.as_path().display().to_string(),
            "query": self.query,
            "glob": self.options.glob.as_deref(),
            "max_results": self.options.max_results,
            "max_bytes_per_file": self.options.max_bytes_per_file,
            "case_sensitive": self.options.case_sensitive,
        }))
    }
}

/// Terminal content-search policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsContentSearchAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsContentSearchAction> for FsContentSearchAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-content-search-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsContentSearchAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.content_search reached terminal allow policy".into()),
            reason: "filesystem content search allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
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
    ) -> Result<FsContentSearchOutput, FsContentSearchError> {
        let root = self.grant_target_path(root).await?;

        let action = FsContentSearchAction::new(root, query, options);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized content search.
///
/// The search reads candidate files and filters their contents inside access;
/// concrete tools receive only match metadata and snippets, not arbitrary file
/// bytes from every file under the root.
#[async_trait]
impl<Cx> Action<Cx> for FsContentSearchAction
where
    Cx: Sync,
{
    type Output = FsContentSearchOutput;
    type Error = FsContentSearchError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let root = action.root().to_path_buf();
        let query = action.query().to_owned();
        let options = action.options().clone();
        if options.max_results == 0 {
            return Ok(FsContentSearchOutput {
                root,
                query,
                matches: Vec::new(),
                truncated: true,
            });
        }

        let glob_matcher = match options.glob.as_deref() {
            Some(pattern) => Some(GlobMatcher::new(pattern).map_err(|source| {
                FsContentSearchError::InvalidGlobPattern {
                    pattern: pattern.to_owned(),
                    source,
                }
            })?),
            None => None,
        };
        let mut matches = Vec::new();
        let mut queue = VecDeque::from([root.clone()]);

        while let Some(directory) = queue.pop_front() {
            let mut children = std::fs::read_dir(&directory)
                .map_err(|source| FsContentSearchError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| FsContentSearchError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?;
            children.sort_by_key(std::fs::DirEntry::path);

            for child in children {
                let child_path = child.path();
                let file_type =
                    child
                        .file_type()
                        .map_err(|source| FsContentSearchError::InspectPath {
                            path: child_path.clone(),
                            source,
                        })?;

                if file_type.is_dir() {
                    queue.push_back(child_path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }

                let relative_path = child_path.strip_prefix(&root).map_err(|source| {
                    FsContentSearchError::RenderRelativePath {
                        root: root.clone(),
                        path: child_path.clone(),
                        source,
                    }
                })?;
                let relative_path = relative_path.to_string_lossy().replace('\\', "/");
                if let Some(matcher) = glob_matcher.as_ref()
                    && !matcher.is_match(relative_path.as_str())
                {
                    continue;
                }

                let file_bytes = std::fs::read(&child_path).map_err(|source| {
                    FsContentSearchError::ReadFile {
                        path: child_path.clone(),
                        source,
                    }
                })?;
                let truncated_file = file_bytes.len() > options.max_bytes_per_file;
                let limited_bytes = file_bytes
                    .get(..options.max_bytes_per_file)
                    .unwrap_or(file_bytes.as_slice());
                let file_text = String::from_utf8_lossy(limited_bytes).to_string();
                let Some((byte_start, byte_end)) =
                    find_content_match(file_text.as_str(), query.as_str(), options.case_sensitive)?
                else {
                    continue;
                };

                let match_text = file_text
                    .get(byte_start..byte_end)
                    .ok_or_else(|| FsContentSearchError::InvalidContentMatchRange {
                        path: child_path.clone(),
                    })?
                    .to_owned();
                let line_info = compute_line_info(file_text.as_str(), byte_start, &child_path)?;
                let snippet = build_snippet(file_text.as_str(), byte_start, byte_end, &child_path)?;
                matches.push(FsContentSearchMatch {
                    path: child_path,
                    relative_path,
                    line: line_info.line,
                    column: line_info.column,
                    match_text,
                    snippet,
                    truncated_file,
                });

                if matches.len() >= options.max_results {
                    return Ok(FsContentSearchOutput {
                        root,
                        query,
                        matches,
                        truncated: true,
                    });
                }
            }
        }

        Ok(FsContentSearchOutput {
            root,
            query,
            matches,
            truncated: false,
        })
    }
}

/// Result returned by a governed content search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsContentSearchOutput {
    pub root: PathBuf,
    pub query: String,
    pub matches: Vec<FsContentSearchMatch>,
    pub truncated: bool,
}

/// One text match returned by a governed content search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsContentSearchMatch {
    pub path: PathBuf,
    pub relative_path: String,
    pub line: usize,
    pub column: usize,
    pub match_text: String,
    pub snippet: String,
    pub truncated_file: bool,
}

fn find_content_match(
    content: &str,
    query: &str,
    case_sensitive: bool,
) -> Result<Option<(usize, usize)>, FsContentSearchError> {
    if query.is_empty() {
        return Ok(None);
    }

    if case_sensitive {
        let Some(byte_start) = content.find(query) else {
            return Ok(None);
        };
        return Ok(Some((byte_start, byte_start + query.len())));
    }

    let escaped_query = regex::escape(query);
    let mut regex_builder = RegexBuilder::new(escaped_query.as_str());
    regex_builder.case_insensitive(true);
    let regex =
        regex_builder
            .build()
            .map_err(|source| FsContentSearchError::BuildContentSearchRegex {
                query: query.to_owned(),
                source,
            })?;
    Ok(regex
        .find(content)
        .map(|matched| (matched.start(), matched.end())))
}

fn build_snippet(
    content: &str,
    byte_start: usize,
    byte_end: usize,
    path: &std::path::Path,
) -> Result<String, FsContentSearchError> {
    let prefix = content.get(..byte_start).ok_or_else(|| {
        FsContentSearchError::InvalidContentMatchRange {
            path: path.to_path_buf(),
        }
    })?;
    let suffix =
        content
            .get(byte_end..)
            .ok_or_else(|| FsContentSearchError::InvalidContentMatchRange {
                path: path.to_path_buf(),
            })?;
    let snippet_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let snippet_end = suffix
        .find('\n')
        .map_or(content.len(), |index| byte_end + index);
    let snippet = content.get(snippet_start..snippet_end).ok_or_else(|| {
        FsContentSearchError::InvalidContentMatchRange {
            path: path.to_path_buf(),
        }
    })?;
    Ok(snippet.trim().to_owned())
}

fn compute_line_info(
    content: &str,
    byte_start: usize,
    path: &std::path::Path,
) -> Result<LineInfo, FsContentSearchError> {
    let prefix = content.get(..byte_start).ok_or_else(|| {
        FsContentSearchError::InvalidContentMatchRange {
            path: path.to_path_buf(),
        }
    })?;
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map(|segment| segment.chars().count() + 1)
        .unwrap_or(1);
    Ok(LineInfo { line, column })
}

struct LineInfo {
    line: usize,
    column: usize,
}
