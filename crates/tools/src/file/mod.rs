use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use loong_contracts::{Capability, ToolExecutionError, ToolInputError, ToolSpec};
use loong_core::{policy::context::ContextFactory, tool::ToolImpl};
use loong_kernel::{
    KernelAccess,
    access::fs::{FsAccessError, FsContentSearchOptions},
};
use serde_json::{Value, json};

mod search;
mod write;

pub use search::{
    ContentSearchReadOutput, ContentSearchReadRequest, ContentSearchTool, GlobReadOutput,
    GlobReadRequest, GlobSearchTool,
};
pub use write::{WriteOutput, WriteRequest, WriteTool};

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
    tool_name: String,
    path: PathBuf,
    bytes: usize,
    selection: FileReadSelection,
}

impl From<ReadFileOutput> for Value {
    fn from(output: ReadFileOutput) -> Self {
        let mut payload = json!({
            "adapter": "core-tools",
            "tool_name": output.tool_name,
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
    tool_name: String,
    target: String,
    max_bytes: usize,
    offset: Option<usize>,
    limit: Option<usize>,
}

pub struct ReadTool {
    tool_name: &'static str,
}

impl ReadTool {
    /// Creates the aggregate read implementation for an app-facing tool name.
    ///
    /// The name is used in legacy-compatible responses and continuation
    /// payloads only. The actual registry path remains owned by the app plane.
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
    for<'a> C::Cx<'a>: KernelAccess<C> + Sync,
{
    type Input = ReadRequest;
    type Output = ReadOutput;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Read files, list paths, or search file contents in allowed roots."
                .to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([Capability::FilesystemRead]),
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
        ReadRequest::parse_payload(self.tool_name.to_owned(), &payload)
            .map_err(ToolInputError::invalid_payload)
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError> {
        // ReadTool owns direct-read mode selection and response shaping only.
        // Filesystem side effects stay behind loong_access::fs actions.
        match input {
            ReadRequest::File(input) => {
                let output = ctx
                    .access()
                    .fs()
                    .read_file(input.target.as_str())
                    .await
                    .map_err(fs_access_error_reason)
                    .map_err(ToolExecutionError::execution)?;
                Self::build_file_output(input, output.path, output.bytes)
                    .map(ReadOutput::File)
                    .map_err(ToolExecutionError::execution)
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
                    .await
                    .map_err(fs_access_error_reason)
                    .map_err(ToolExecutionError::execution)?;
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
                    .await
                    .map_err(fs_access_error_reason)
                    .map_err(ToolExecutionError::execution)?;
                Ok(ReadOutput::Content(
                    ContentSearchReadOutput::from_access_output(input, output),
                ))
            }
        }
    }
}

impl ReadTool {
    fn build_file_output(
        request: FileReadRequest,
        resolved: PathBuf,
        bytes: Vec<u8>,
    ) -> Result<ReadFileOutput, String> {
        let file_text = String::from_utf8_lossy(&bytes).to_string();
        let selection = select_file_read_content(
            file_text.as_str(),
            request.max_bytes,
            request.offset,
            request.limit,
            request.tool_name.as_str(),
        )?;

        Ok(ReadFileOutput {
            tool_name: request.tool_name,
            path: resolved,
            bytes: bytes.len(),
            selection,
        })
    }
}

impl ReadRequest {
    fn parse_payload(tool_name: String, payload: &Value) -> Result<Self, String> {
        let payload_object = payload
            .as_object()
            .ok_or_else(|| format!("{tool_name} payload must be an object"))?;

        let has_path = optional_trimmed_string_field(payload_object.get("path")).is_some();
        let has_query = optional_trimmed_string_field(payload_object.get("query")).is_some();
        let has_pattern = optional_trimmed_string_field(payload_object.get("pattern")).is_some()
            || optional_trimmed_string_field(payload_object.get("glob")).is_some();

        if !has_path && !has_query && !has_pattern {
            return Err(
                "direct_read_requires_one_of: expected exactly one of `path`, `query`, or `pattern`"
                    .to_owned(),
            );
        }

        if has_path {
            return FileReadRequest::parse_payload(tool_name, payload).map(Self::File);
        }

        if has_query {
            return ContentSearchReadRequest::parse_payload(tool_name, payload_object)
                .map(Self::Content);
        }

        GlobReadRequest::parse_payload(tool_name, payload_object).map(Self::Glob)
    }
}

impl FileReadRequest {
    fn parse_payload(tool_name: String, payload: &Value) -> Result<Self, String> {
        let payload = payload
            .as_object()
            .ok_or_else(|| format!("{tool_name} payload must be an object"))?;
        let target = payload
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{tool_name} requires payload.path"))?
            .to_owned();

        let max_bytes = payload
            .get("max_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(1_048_576)
            .min(8 * 1_048_576) as usize;
        let offset = optional_positive_usize_field(payload, "offset", &tool_name)?;
        let limit = optional_positive_usize_field(payload, "limit", &tool_name)?;

        Ok(Self {
            tool_name,
            target,
            max_bytes,
            offset,
            limit,
        })
    }
}

// Boundary conversion: access keeps typed errors, while the legacy app-facing
// tool result still carries string reasons. Keep policy denials recognizable
// until the outer error envelope becomes typed end to end.
fn fs_access_error_reason(error: FsAccessError) -> String {
    let rendered = error.to_string();
    if matches!(error, FsAccessError::Authorization(_)) {
        format!("policy_denied: {rendered}")
    } else {
        rendered
    }
}

fn required_trimmed_string_field<'a>(
    payload: &'a serde_json::Map<String, Value>,
    field_name: &str,
    tool_name: &str,
) -> Result<&'a str, String> {
    optional_trimmed_string_field(payload.get(field_name))
        .ok_or_else(|| format!("{tool_name} requires payload.{field_name}"))
}

// `offset` and `limit` intentionally share one parser: both fields use the same
// positive-integer contract, and a separate type would not add a stronger boundary.
fn optional_positive_usize_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    tool_name: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = payload.get(field_name) else {
        return Ok(None);
    };

    let raw_value = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.{field_name} must be a positive integer"))?;
    if raw_value == 0 {
        return Err(format!(
            "{tool_name} payload.{field_name} must be a positive integer"
        ));
    }

    usize::try_from(raw_value)
        .map(Some)
        .map_err(|conversion_error| {
            format!("{tool_name} payload.{field_name} is too large: {conversion_error}")
        })
}

fn optional_trimmed_string_field(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn optional_bounded_usize_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
    default_value: usize,
    minimum: usize,
    maximum: usize,
    tool_name: &str,
) -> Result<usize, String> {
    let Some(value) = payload.get(field_name) else {
        return Ok(default_value);
    };
    let parsed_value_u64 = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.{field_name} must be an integer"))?;
    let parsed_value = usize::try_from(parsed_value_u64).map_err(|conversion_error| {
        format!("{tool_name} payload.{field_name} is out of range: {conversion_error}")
    })?;
    if parsed_value < minimum || parsed_value > maximum {
        return Err(format!(
            "{tool_name} payload.{field_name} must be between {minimum} and {maximum}"
        ));
    }
    Ok(parsed_value)
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
    tool_name: &str,
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
        .ok_or_else(|| format!("{tool_name} internal line window is out of bounds"))?;
    let selected_content = selected_lines.join("\n");
    let mut selection = clip_file_read_content(selected_content.as_str(), max_bytes);
    selection.line_start = Some(line_start);
    selection.line_end = Some(end_index);
    selection.total_lines = Some(total_lines);
    selection.next_offset = (end_index < total_lines).then_some(end_index + 1);
    Ok(selection)
}

#[cfg(test)]
mod tests;
