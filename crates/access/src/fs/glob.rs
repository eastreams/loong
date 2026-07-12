use std::{collections::VecDeque, path::PathBuf};

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsGlobAction};

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
    pub(in crate::fs) fn from_file_type(file_type: std::fs::FileType) -> Option<Self> {
        if file_type.is_dir() {
            return Some(Self::Directory);
        }
        if file_type.is_file() || file_type.is_symlink() {
            return Some(Self::File);
        }
        None
    }
}

pub(in crate::fs) struct GlobMatcher {
    regexes: Vec<regex::Regex>,
}

impl GlobMatcher {
    pub(in crate::fs) fn new(pattern: &str) -> Result<Self, FsAccessError> {
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
