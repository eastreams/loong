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
        FsContentSearchError, FsContentSearchOptions, FsGlobError, FsPathError,
        FsPathPolicyContext, FsReadError, FsResolutionContext,
    },
};
use serde_json::{Value, json};
use thiserror::Error;

use super::{
    ContentSearchReadOutput, ContentSearchReadRequest, GlobReadOutput, GlobReadRequest,
    optional_positive_usize_field, optional_trimmed_string_field, required_trimmed_string_field,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadRequest {
    File(FileReadRequest),
    Glob(GlobReadRequest),
    Content(ContentSearchReadRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutput {
    File(ReadFileOutput),
    Glob(GlobReadOutput),
    Content(ContentSearchReadOutput),
}

/// Failures specific to the aggregate read tool.
///
/// The three access variants correspond to the three payload modes. Keeping
/// them distinct lets the tool classify policy denial before runtime erases
/// the concrete error, while response shaping remains owned by this module.
#[derive(Debug, Error)]
pub enum ReadToolError {
    #[error("{0}")]
    Read(
        #[from]
        #[source]
        FsReadError,
    ),
    #[error("{0}")]
    Glob(
        #[from]
        #[source]
        FsGlobError,
    ),
    #[error("{0}")]
    ContentSearch(
        #[from]
        #[source]
        FsContentSearchError,
    ),
    #[error("{reason}")]
    InvalidLineWindow { reason: String },
}

impl From<ReadOutput> for Value {
    fn from(output: ReadOutput) -> Self {
        match output {
            ReadOutput::File(output) => output.into(),
            ReadOutput::Glob(output) => output.into(),
            ReadOutput::Content(output) => output.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileReadSelection {
    content: String,
    truncated: bool,
    line_start: Option<usize>,
    line_end: Option<usize>,
    total_lines: Option<usize>,
    next_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFileOutput {
    path: PathBuf,
    bytes: usize,
    selection: FileReadSelection,
}

impl From<ReadFileOutput> for Value {
    fn from(output: ReadFileOutput) -> Self {
        let mut payload = json!({
            "path": output.path.display().to_string(),
            "bytes": output.bytes,
            "truncated": output.selection.truncated,
            "content": output.selection.content,
        });
        let Some(payload_object) = payload.as_object_mut() else {
            return payload;
        };
        if let Some(line_start) = output.selection.line_start {
            payload_object.insert("line_start".to_owned(), json!(line_start));
        }
        if let Some(line_end) = output.selection.line_end {
            payload_object.insert("line_end".to_owned(), json!(line_end));
        }
        if let Some(total_lines) = output.selection.total_lines {
            payload_object.insert("total_lines".to_owned(), json!(total_lines));
        }
        if let Some(next_offset) = output.selection.next_offset {
            payload_object.insert("next_offset".to_owned(), json!(next_offset));
        }
        payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReadRequest {
    pub(super) target: String,
    pub(super) max_bytes: usize,
    pub(super) offset: Option<usize>,
    pub(super) limit: Option<usize>,
}

pub struct ReadTool;

impl ReadTool {
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Read one file at this workspace-relative or absolute path."
                },
                "max_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 8_388_608,
                    "description": "Optional read limit in bytes when reading one file or file window."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional 1-indexed line number to start from when reading one file."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional maximum number of lines to return when reading one file."
                },
                "query": {
                    "type": "string",
                    "description": "Search workspace file contents for this text."
                },
                "pattern": {
                    "type": "string",
                    "description": "List workspace paths that match this glob pattern."
                },
                "root": {
                    "type": "string",
                    "description": "Optional search root path for query or pattern mode."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional file glob filter applied only in query mode."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "description": "Optional maximum result count for query or pattern mode."
                },
                "max_bytes_per_file": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1_048_576,
                    "description": "Optional per-file scan budget used only in query mode."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Use case-sensitive matching in query mode. Defaults to false."
                },
                "include_directories": {
                    "type": "boolean",
                    "description": "Include matching directories in pattern mode. Defaults to false."
                }
            },
            "anyOf": [
                { "required": ["path"] },
                { "required": ["query"] },
                { "required": ["pattern"] }
            ],
            "additionalProperties": false
        })
    }
}

/// Concrete builtin implementation for the aggregate file-reading facade.
///
/// `loong-tools` exports this value so the app plane can register it at a
/// runtime-owned path. The tool does not own that path, audit, or policy; it
/// only parses the already-selected payload, calls governed access, and shapes
/// the response.
#[async_trait]
impl<C> ToolImpl<C> for ReadTool
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + FsResolutionContext + FsPathPolicyContext + Sync,
{
    type Input = ReadRequest;
    type Output = ReadOutput;
    type Error = ReadToolError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Read files, list paths, or search file contents in allowed roots."
                .to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([Capability::FilesystemRead]),
            scheduling: ToolSchedulingClass::ParallelSafe,
            argument_hint: Some(
                "path?:string,offset?:integer,limit?:integer,max_bytes?:integer,query?:string,pattern?:string,root?:string,glob?:string,max_results?:integer,max_bytes_per_file?:integer,case_sensitive?:boolean,include_directories?:boolean"
                    .to_owned(),
            ),
            search_hint: Some(
                "read one file, page through a large file, search workspace content, or list matching paths through one direct tool"
                    .to_owned(),
            ),
            tags: ["surface", "read", "file", "search"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        ReadRequest::parse_payload(&payload)
    }

    fn failure_kind(&self, error: &Self::Error) -> ToolFailureKind {
        match error {
            ReadToolError::Read(
                FsReadError::Authorization(source)
                | FsReadError::Path(FsPathError::Authorization(source)),
            )
            | ReadToolError::Glob(
                FsGlobError::Authorization(source)
                | FsGlobError::Path(FsPathError::Authorization(source)),
            )
            | ReadToolError::ContentSearch(
                FsContentSearchError::Authorization(source)
                | FsContentSearchError::Path(FsPathError::Authorization(source)),
            ) if matches!(
                source,
                PolicyGrantError::MissingCapability { .. }
                    | PolicyGrantError::Denied { .. }
                    | PolicyGrantError::PermissionDenied { .. }
            ) =>
            {
                ToolFailureKind::Denied
            }
            ReadToolError::Read(_)
            | ReadToolError::Glob(_)
            | ReadToolError::ContentSearch(_)
            | ReadToolError::InvalidLineWindow { .. } => ToolFailureKind::Execution,
        }
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        // ReadTool owns direct-read mode selection and response shaping only.
        // Filesystem side effects stay behind loong_access::fs actions.
        match input {
            ReadRequest::File(input) => {
                let output = ctx.access().fs().read_file(input.target.as_str()).await?;
                Self::build_file_output(input, output.path, output.bytes).map(ReadOutput::File)
            }
            ReadRequest::Glob(input) => {
                let output = ctx
                    .access()
                    .fs()
                    .glob_paths(
                        input.root.as_str(),
                        input.pattern.clone(),
                        input.include_directories,
                        input.max_results,
                    )
                    .await?;
                Ok(ReadOutput::Glob(GlobReadOutput::from_access_output(
                    input, output,
                )))
            }
            ReadRequest::Content(input) => {
                let options = FsContentSearchOptions {
                    glob: input.glob.clone(),
                    max_results: input.max_results,
                    max_bytes_per_file: input.max_bytes_per_file,
                    case_sensitive: input.case_sensitive,
                };
                let output = ctx
                    .access()
                    .fs()
                    .search_content(input.root.as_str(), input.query.clone(), options)
                    .await?;
                Ok(ReadOutput::Content(
                    ContentSearchReadOutput::from_access_output(input, output),
                ))
            }
        }
    }
}

impl ReadTool {
    pub(super) fn build_file_output(
        request: FileReadRequest,
        resolved: PathBuf,
        bytes: Vec<u8>,
    ) -> Result<ReadFileOutput, ReadToolError> {
        let file_text = String::from_utf8_lossy(&bytes).to_string();
        let selection = select_file_read_content(
            file_text.as_str(),
            request.max_bytes,
            request.offset,
            request.limit,
        )
        .map_err(|reason| ReadToolError::InvalidLineWindow { reason })?;

        Ok(ReadFileOutput {
            path: resolved,
            bytes: bytes.len(),
            selection,
        })
    }
}

impl ReadRequest {
    pub(super) fn parse_payload(payload: &Value) -> Result<Self, ToolInputError> {
        let payload_object = payload
            .as_object()
            .ok_or(ToolInputError::PayloadMustBeObject)?;

        let has_path = optional_trimmed_string_field(payload_object.get("path")).is_some();
        let has_query = optional_trimmed_string_field(payload_object.get("query")).is_some();
        let has_pattern = optional_trimmed_string_field(payload_object.get("pattern")).is_some()
            || optional_trimmed_string_field(payload_object.get("glob")).is_some();

        if has_path {
            return FileReadRequest::parse_payload(payload).map(Self::File);
        }

        if has_query {
            return ContentSearchReadRequest::parse_payload(payload_object).map(Self::Content);
        }

        if has_pattern {
            return GlobReadRequest::parse_payload(payload_object).map(Self::Glob);
        }

        for field_name in ["path", "query", "pattern", "glob"] {
            if payload_object.contains_key(field_name) {
                required_trimmed_string_field(payload_object, field_name)?;
            }
        }

        Err(ToolInputError::missing_one_of(["path", "query", "pattern"]))
    }
}

impl FileReadRequest {
    pub(super) fn parse_payload(payload: &Value) -> Result<Self, ToolInputError> {
        let payload = payload
            .as_object()
            .ok_or(ToolInputError::PayloadMustBeObject)?;
        let target = required_trimmed_string_field(payload, "path")?.to_owned();

        let max_bytes = payload
            .get("max_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(1_048_576)
            .min(8 * 1_048_576) as usize;
        let offset = optional_positive_usize_field(payload, "offset")?;
        let limit = optional_positive_usize_field(payload, "limit")?;

        Ok(Self {
            target,
            max_bytes,
            offset,
            limit,
        })
    }
}

fn clip_file_read_content(content: &str, max_bytes: usize) -> FileReadSelection {
    let content_bytes = content.as_bytes();
    let truncated = content_bytes.len() > max_bytes;
    let visible_bytes = if truncated {
        content_bytes.get(..max_bytes).unwrap_or(content_bytes)
    } else {
        content_bytes
    };

    FileReadSelection {
        content: String::from_utf8_lossy(visible_bytes).to_string(),
        truncated,
        line_start: None,
        line_end: None,
        total_lines: None,
        next_offset: None,
    }
}

fn select_file_read_content(
    file_text: &str,
    max_bytes: usize,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<FileReadSelection, String> {
    let line_window_requested = offset.is_some() || limit.is_some();
    if !line_window_requested {
        return Ok(clip_file_read_content(file_text, max_bytes));
    }

    let all_lines = file_text.split('\n').collect::<Vec<_>>();
    let total_lines = all_lines.len();
    let line_start = offset.unwrap_or(1);
    if line_start > total_lines {
        return Err(format!(
            "offset {line_start} is beyond end of file ({total_lines} lines total)"
        ));
    }

    let start_index = line_start.saturating_sub(1);
    let requested_line_count = limit.unwrap_or(total_lines.saturating_sub(start_index));
    let end_index = start_index
        .saturating_add(requested_line_count)
        .min(total_lines);
    let selected_lines = all_lines
        .get(start_index..end_index)
        .ok_or_else(|| "internal line window is out of bounds".to_owned())?;
    let selected_content = selected_lines.join("\n");
    let mut selection = clip_file_read_content(selected_content.as_str(), max_bytes);
    selection.line_start = Some(line_start);
    selection.line_end = Some(end_index);
    selection.total_lines = Some(total_lines);
    selection.next_offset = (end_index < total_lines).then_some(end_index + 1);
    Ok(selection)
}
