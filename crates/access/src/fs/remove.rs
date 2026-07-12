use std::path::PathBuf;

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsRemoveFileAction};

/// Execute an already-authorized file or symlink removal.
///
/// The deletion path was prepared with final-component no-follow semantics, so
/// `remove_file` deletes a symlink itself while ancestor symlinks are reflected
/// in the policy-visible path facts.
#[async_trait]
impl<Cx> Action<Cx> for FsRemoveFileAction
where
    Cx: Sync,
{
    type Output = FsRemoveFileOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.deletion_path().to_path_buf();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FsRemoveFileOutput {
                    path,
                    removed: false,
                    kind: None,
                });
            }
            Err(source) => {
                return Err(FsAccessError::InspectPath {
                    path: path.clone(),
                    source,
                });
            }
        };

        let file_type = metadata.file_type();
        if file_type.is_dir() {
            return Err(FsAccessError::PathIsDirectory { path });
        }
        let kind = if file_type.is_symlink() {
            FsRemoveFileKind::Symlink
        } else {
            FsRemoveFileKind::File
        };

        match std::fs::remove_file(&path) {
            Ok(()) => Ok(FsRemoveFileOutput {
                path,
                removed: true,
                kind: Some(kind),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FsRemoveFileOutput {
                path,
                removed: false,
                kind: None,
            }),
            Err(source) => Err(FsAccessError::RemoveFile { path, source }),
        }
    }
}

/// Result returned after a governed file removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRemoveFileOutput {
    pub path: PathBuf,
    pub removed: bool,
    pub kind: Option<FsRemoveFileKind>,
}

/// Kind of filesystem entry removed by [`FsRemoveFileAction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsRemoveFileKind {
    File,
    Symlink,
}
