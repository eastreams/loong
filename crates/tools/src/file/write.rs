use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use loong_contracts::{Capability, ToolInputError, ToolSpec};
use loong_core::{
    policy::context::ContextFactory,
    tool::{ToolFailureKind, ToolImpl},
};
use loong_kernel::{KernelAccess, access::fs::FsWriteOptions};
use serde_json::{Value, json};

use super::{FileToolError, required_trimmed_string_field};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    pub(super) tool_name: String,
    pub(super) path: String,
    pub(super) content: String,
    pub(super) create_dirs: bool,
    pub(super) overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutput {
    pub(super) tool_name: String,
    pub(super) path: PathBuf,
    pub(super) bytes_written: usize,
}

impl From<WriteOutput> for Value {
    fn from(output: WriteOutput) -> Self {
        json!({
            "adapter": "core-tools",
            "tool_name": output.tool_name,
            "path": output.path.display().to_string(),
            "bytes_written": output.bytes_written,
        })
    }
}

pub struct WriteTool {
    tool_name: &'static str,
}

impl WriteTool {
    /// Creates the write implementation for an app-facing tool name.
    ///
    /// Like `ReadTool`, this name is response metadata only. The app plane owns
    /// the registry path and will decide when this typed tool replaces legacy
    /// `write` / `file.write` dispatch.
    #[must_use]
    pub const fn new(tool_name: &'static str) -> Self {
        Self { tool_name }
    }

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
/// This is only the typed tool foundation: registration and legacy dispatch
/// migration stay in the app plane. The tool performs no filesystem I/O itself;
/// the write side effect is delegated to `ctx.access().fs().write_file(...)`.
#[async_trait]
impl<C> ToolImpl<C> for WriteTool
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + Sync,
{
    type Input = WriteRequest;
    type Output = WriteOutput;
    type Error = FileToolError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Write file contents in allowed roots.".to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([Capability::FilesystemWrite]),
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
        WriteRequest::parse_payload(self.tool_name.to_owned(), &payload)
            .map_err(ToolInputError::invalid_payload)
    }

    fn failure_kind(&self, error: &Self::Error) -> ToolFailureKind {
        error.failure_kind()
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
            tool_name: input.tool_name,
            path: output.path,
            bytes_written: output.bytes_written,
        })
    }
}

impl WriteRequest {
    pub(super) fn parse_payload(tool_name: String, payload: &Value) -> Result<Self, String> {
        let payload = payload
            .as_object()
            .ok_or_else(|| format!("{tool_name} payload must be an object"))?;
        let path = required_trimmed_string_field(payload, "path", tool_name.as_str())?.to_owned();
        let content = payload
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{tool_name} requires payload.content"))?
            .to_owned();
        let create_dirs = match payload.get("create_dirs") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| format!("{tool_name} payload.create_dirs must be a boolean"))?,
            None => true,
        };
        let overwrite = match payload.get("overwrite") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| format!("{tool_name} payload.overwrite must be a boolean"))?,
            None => false,
        };

        Ok(Self {
            tool_name,
            path,
            content,
            create_dirs,
            overwrite,
        })
    }
}
