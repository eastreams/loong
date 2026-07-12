use std::path::PathBuf;

use async_trait::async_trait;
use loong_core::policy::{action::Action, grant::Granted};

use super::{access::FsAccessError, action::FsInspectPathAction, glob::FsPathKind};

/// Execute an already-authorized path inspection.
///
/// This observes filesystem metadata but does not read file contents. It still
/// consumes `Granted<FsInspectPathAction>` because existence and type metadata
/// are policy-governed read-family information.
#[async_trait]
impl<Cx> Action<Cx> for FsInspectPathAction
where
    Cx: Sync,
{
    type Output = FsInspectPathOutput;
    type Error = FsAccessError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let kind = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => FsPathKind::from_file_type(metadata.file_type()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(FsAccessError::InspectPath { path, source });
            }
        };

        Ok(FsInspectPathOutput { path, kind })
    }
}

/// Result returned after a governed filesystem path inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInspectPathOutput {
    pub path: PathBuf,
    pub kind: Option<FsPathKind>,
}
