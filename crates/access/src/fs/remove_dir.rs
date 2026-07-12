use std::path::PathBuf;

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsRemoveDirAllAction};

/// Execute an already-authorized recursive directory removal.
///
/// This primitive is intentionally separate from [`FsRemoveFileAction`]:
/// recursive deletion is broader than unlinking one file or symlink, so tools
/// must request it explicitly and policy can report it as a distinct action.
#[async_trait]
impl<Cx> Action<Cx> for FsRemoveDirAllAction
where
    Cx: Sync,
{
    type Output = FsRemoveDirAllOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FsRemoveDirAllOutput {
                    path,
                    removed: false,
                });
            }
            Err(source) => {
                return Err(FsAccessError::InspectPath { path, source });
            }
        };

        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(FsAccessError::RefuseSymlink { path });
        }
        if !file_type.is_dir() {
            return Err(FsAccessError::PathIsNotDirectory { path });
        }

        std::fs::remove_dir_all(&path).map_err(|source| FsAccessError::RemoveDirectory {
            path: path.clone(),
            source,
        })?;

        Ok(FsRemoveDirAllOutput {
            path,
            removed: true,
        })
    }
}

/// Result returned after a governed recursive directory removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRemoveDirAllOutput {
    pub path: PathBuf,
    pub removed: bool,
}
