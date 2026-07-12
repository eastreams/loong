use std::path::PathBuf;

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsReadDirAction, glob::FsPathKind};

/// Execute an already-authorized immediate directory listing.
///
/// This intentionally does not recurse. Callers that need recursive matching
/// should use `FsGlobAction`; import/discovery code can use this narrower
/// primitive when it only needs direct children.
#[async_trait]
impl<Cx> Action<Cx> for FsReadDirAction
where
    Cx: Sync,
{
    type Output = FsReadDirOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let root = action.root().to_path_buf();
        if action.max_entries() == 0 {
            return Ok(FsReadDirOutput {
                root,
                entries: Vec::new(),
                truncated: true,
            });
        }

        let mut children = std::fs::read_dir(&root)
            .map_err(|source| FsAccessError::ReadDirectory {
                path: root.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| FsAccessError::ReadDirectory {
                path: root.clone(),
                source,
            })?;
        children.sort_by_key(std::fs::DirEntry::path);

        let mut entries = Vec::new();
        for child in children {
            let path = child.path();
            let file_type = child
                .file_type()
                .map_err(|source| FsAccessError::InspectPath {
                    path: path.clone(),
                    source,
                })?;
            let Some(kind) = FsPathKind::from_file_type(file_type) else {
                continue;
            };
            let name = child.file_name().to_string_lossy().to_string();
            entries.push(FsReadDirEntry { path, name, kind });
            if entries.len() >= action.max_entries() {
                return Ok(FsReadDirOutput {
                    root,
                    entries,
                    truncated: true,
                });
            }
        }

        Ok(FsReadDirOutput {
            root,
            entries,
            truncated: false,
        })
    }
}

/// Result returned after a governed immediate directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadDirOutput {
    pub root: PathBuf,
    pub entries: Vec<FsReadDirEntry>,
    pub truncated: bool,
}

/// One entry returned by [`FsReadDirAction`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadDirEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: FsPathKind,
}
