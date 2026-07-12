use std::path::{Path, PathBuf};

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsRenameAction};

/// Execute an already-authorized filesystem rename.
///
/// This is the primitive skills lifecycle migration needs for staged installs:
/// prepare a destination tree, then move the governed entry into place without
/// exposing a raw `std::fs::rename` call to app orchestration.
#[async_trait]
impl<Cx> Action<Cx> for FsRenameAction
where
    Cx: Sync,
{
    type Output = FsRenameOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let source = action.source_path().to_path_buf();
        let destination = action.destination_path().to_path_buf();
        let options = action.options();

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

        let overwritten = path_entry_exists_no_follow(&destination)?;
        if overwritten && !options.overwrite {
            return Err(FsAccessError::PathExistsRequiresOverwrite { path: destination });
        }

        std::fs::rename(&source, &destination).map_err(|source_error| {
            FsAccessError::RenamePath {
                source_path: source.clone(),
                destination_path: destination.clone(),
                source: source_error,
            }
        })?;

        Ok(FsRenameOutput {
            source,
            destination,
            overwritten,
        })
    }
}

fn path_entry_exists_no_follow(path: &Path) -> Result<bool, FsAccessError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(FsAccessError::InspectPath {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Result returned after a governed filesystem rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRenameOutput {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub overwritten: bool,
}
