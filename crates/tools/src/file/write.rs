use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use loong_contracts::{Capability, ToolInputError, ToolSchedulingClass, ToolSpec};
use loong_core::{
    PolicyGrantError,
    policy::context::ContextFactory,
    tool::{ToolFailureKind, ToolImpl},
};
use loong_kernel::{
    KernelAccess,
    access::fs::{
        FsPathError, FsPathPolicyContext, FsResolutionContext, FsWriteError, FsWriteOptions,
    },
};
use serde_json::{Value, json};

use super::required_trimmed_string_field;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    pub(super) path: String,
    pub(super) content: String,
    pub(super) create_dirs: bool,
    pub(super) overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutput {
    pub(super) path: PathBuf,
    pub(super) bytes_written: usize,
}

impl From<WriteOutput> for Value {
    fn from(output: WriteOutput) -> Self {
        json!({
            "path": output.path.display().to_string(),
            "bytes_written": output.bytes_written,
        })
    }
}

pub struct WriteTool;

impl WriteTool {
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Write file contents to this workspace-relative or absolute path."
                },
                "content": {
                    "type": "string",
                    "description": "Exact file contents to write."
                },
                "create_dirs": {
                    "type": "boolean",
                    "description": "Create missing parent directories before writing. Defaults to true."
                },
                "overwrite": {
                    "type": "boolean",
                    "description": "Allow replacing an existing file. Defaults to false."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }
}

/// Concrete builtin implementation for writing file contents.
///
/// Registration and tool identity stay in the app plane. The tool performs no
/// filesystem I/O itself; the write side effect is delegated to
/// `ctx.access().fs().write_file(...)`.
#[async_trait]
impl<C> ToolImpl<C> for WriteTool
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + FsResolutionContext + FsPathPolicyContext + Sync,
{
    type Input = WriteRequest;
    type Output = WriteOutput;
    type Error = FsWriteError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Write file contents in allowed roots.".to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([Capability::FilesystemWrite]),
            scheduling: ToolSchedulingClass::SerialOnly,
            argument_hint: Some(
                "path:string,content:string,create_dirs?:boolean,overwrite?:boolean".to_owned(),
            ),
            search_hint: Some(
                "write exact file contents, optionally creating parent directories or overwriting an existing file"
                    .to_owned(),
            ),
            tags: ["surface", "write", "file"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        WriteRequest::parse_payload(&payload)
    }

    fn failure_kind(&self, error: &Self::Error) -> ToolFailureKind {
        match error {
            FsWriteError::Authorization(source)
            | FsWriteError::Path(FsPathError::Authorization(source))
                if matches!(
                    source,
                    PolicyGrantError::MissingCapability { .. }
                        | PolicyGrantError::Denied { .. }
                        | PolicyGrantError::PermissionDenied { .. }
                ) =>
            {
                ToolFailureKind::Denied
            }
            FsWriteError::Path(_)
            | FsWriteError::Authorization(_)
            | FsWriteError::InspectPath { .. }
            | FsWriteError::CreateParentDirectory { .. }
            | FsWriteError::PathIsDirectory { .. }
            | FsWriteError::RefuseSymlink { .. }
            | FsWriteError::FileExistsRequiresOverwrite { .. }
            | FsWriteError::OpenWriteFile { .. }
            | FsWriteError::WriteFile { .. } => ToolFailureKind::Execution,
        }
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        let options = FsWriteOptions {
            create_dirs: input.create_dirs,
            overwrite: input.overwrite,
        };
        let output = ctx
            .access()
            .fs()
            .write_file(input.path.as_str(), input.content.into_bytes(), options)
            .await?;

        Ok(WriteOutput {
            path: output.path,
            bytes_written: output.bytes_written,
        })
    }
}

impl WriteRequest {
    pub(super) fn parse_payload(payload: &Value) -> Result<Self, ToolInputError> {
        let payload = payload
            .as_object()
            .ok_or(ToolInputError::PayloadMustBeObject)?;
        let path = required_trimmed_string_field(payload, "path")?.to_owned();
        let content = payload
            .get("content")
            .ok_or_else(|| ToolInputError::missing_field("content"))?
            .as_str()
            .ok_or_else(|| ToolInputError::invalid_field("content", "must be a string"))?
            .to_owned();
        let create_dirs = match payload.get("create_dirs") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| ToolInputError::invalid_field("create_dirs", "must be a boolean"))?,
            None => true,
        };
        let overwrite = match payload.get("overwrite") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| ToolInputError::invalid_field("overwrite", "must be a boolean"))?,
            None => false,
        };

        Ok(Self {
            path,
            content,
            create_dirs,
            overwrite,
        })
    }
}
