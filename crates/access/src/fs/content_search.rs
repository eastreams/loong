use std::{collections::VecDeque, path::PathBuf};

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};
use regex::RegexBuilder;

use super::{access::FsAccessError, action::FsContentSearchAction, glob::GlobMatcher};

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
    type Error = FsAccessError;

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
            Some(pattern) => Some(GlobMatcher::new(pattern)?),
            None => None,
        };
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

                if file_type.is_dir() {
                    queue.push_back(child_path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }

                let relative_path = child_path.strip_prefix(&root).map_err(|source| {
                    FsAccessError::RenderRelativePath {
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

                let file_bytes =
                    std::fs::read(&child_path).map_err(|source| FsAccessError::ReadFile {
                        path: child_path.clone(),
                        source,
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
                    .ok_or_else(|| FsAccessError::InvalidContentMatchRange {
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
) -> Result<Option<(usize, usize)>, FsAccessError> {
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
    let regex = regex_builder
        .build()
        .map_err(|source| FsAccessError::BuildContentSearchRegex {
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
) -> Result<String, FsAccessError> {
    let prefix =
        content
            .get(..byte_start)
            .ok_or_else(|| FsAccessError::InvalidContentMatchRange {
                path: path.to_path_buf(),
            })?;
    let suffix =
        content
            .get(byte_end..)
            .ok_or_else(|| FsAccessError::InvalidContentMatchRange {
                path: path.to_path_buf(),
            })?;
    let snippet_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let snippet_end = suffix
        .find('\n')
        .map_or(content.len(), |index| byte_end + index);
    let snippet = content.get(snippet_start..snippet_end).ok_or_else(|| {
        FsAccessError::InvalidContentMatchRange {
            path: path.to_path_buf(),
        }
    })?;
    Ok(snippet.trim().to_owned())
}

fn compute_line_info(
    content: &str,
    byte_start: usize,
    path: &std::path::Path,
) -> Result<LineInfo, FsAccessError> {
    let prefix =
        content
            .get(..byte_start)
            .ok_or_else(|| FsAccessError::InvalidContentMatchRange {
                path: path.to_path_buf(),
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
