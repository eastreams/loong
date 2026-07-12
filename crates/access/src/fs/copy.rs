use std::path::PathBuf;

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsCopyFileAction};

/// Execute an already-authorized file copy.
///
/// Backup/restore flows should use this boundary instead of reading bytes into
/// app orchestration and then writing them back out.
#[async_trait]
impl<Cx> Action<Cx> for FsCopyFileAction
where
    Cx: Sync,
{
    type Output = FsCopyFileOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let source = action.source_path().to_path_buf();
        let destination = action.destination_path().to_path_buf();
        let options = action.options();

        if destination.is_dir() {
            return Err(FsAccessError::PathIsDirectory { path: destination });
        }
        if options.create_dirs
            && let Some(parent) = destination.parent()
        {
            std::fs::create_dir_all(parent).map_err(|source| {
                FsAccessError::CreateParentDirectory {
                    path: parent.to_path_buf(),
                    source,
                }
            })?;
        }

        let overwritten =
            destination
                .try_exists()
                .map_err(|source| FsAccessError::InspectPath {
                    path: destination.clone(),
                    source,
                })?;
        if overwritten && !options.overwrite {
            return Err(FsAccessError::FileExistsRequiresOverwrite { path: destination });
        }

        let bytes_copied = std::fs::copy(&source, &destination).map_err(|source_error| {
            FsAccessError::CopyFile {
                source_path: source.clone(),
                destination_path: destination.clone(),
                source: source_error,
            }
        })?;

        Ok(FsCopyFileOutput {
            source,
            destination,
            bytes_copied,
            overwritten,
        })
    }
}

/// Result returned after a governed file copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCopyFileOutput {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes_copied: u64,
    pub overwritten: bool,
}
