use std::path::PathBuf;

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsCreateDirAllAction};

/// Execute an already-authorized directory creation.
///
/// This is a filesystem write side effect and therefore consumes
/// `Granted<FsCreateDirAllAction>` instead of accepting a raw path.
#[async_trait]
impl<Cx> Action<Cx> for FsCreateDirAllAction
where
    Cx: Sync,
{
    type Output = FsCreateDirAllOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let already_exists = path
            .try_exists()
            .map_err(|source| FsAccessError::InspectPath {
                path: path.clone(),
                source,
            })?;

        std::fs::create_dir_all(&path).map_err(|source| FsAccessError::CreateDirectory {
            path: path.clone(),
            source,
        })?;

        Ok(FsCreateDirAllOutput {
            path,
            already_exists,
        })
    }
}

/// Result returned after a governed directory creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCreateDirAllOutput {
    pub path: PathBuf,
    pub already_exists: bool,
}
