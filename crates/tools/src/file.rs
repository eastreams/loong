use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use loong_contracts::{
    Capability, ToolExecutionError, ToolInputError, ToolOutcome, ToolPath, ToolSpec,
};
use loong_core::{
    policy::context::{ContextFactory, FsAccessContext},
    tool::ToolImpl,
};
use loong_kernel::KernelAccess;
use serde_json::{Value, json};

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
pub struct FileReadRequest {
    tool_name: String,
    target: String,
    max_bytes: usize,
    offset: Option<usize>,
    limit: Option<usize>,
}

pub struct ReadFileTool;

/// Concrete builtin implementation for the file-read branch of `read`.
///
/// `loong-tools` exports this value so the app plane can register it at a
/// runtime-owned path. The tool does not own that path, audit, or policy; it
/// only parses the already-selected payload, calls governed access, and shapes
/// the response.
#[async_trait]
impl<C> ToolImpl<C> for ReadFileTool
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + FsAccessContext + Sync,
{
    type Input = FileReadRequest;
    type Output = ToolOutcome;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            path: ToolPath::from("read"),
            description: "Read a file from the allowed filesystem roots.".to_owned(),
            required_capabilities: BTreeSet::from([Capability::FilesystemRead]),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        parse_file_read_payload("read".to_owned(), &payload)
            .map_err(ToolInputError::invalid_payload)
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError> {
        // ReadFileTool owns input parsing and response shaping only. The
        // filesystem side effect must stay behind loong_access::fs, where
        // path resolution and policy-granted execution are enforced.
        let output = ctx
            .access()
            .fs()
            .read_file(input.target.as_str())
            .await
            .map_err(|error| {
                render_fs_read_error(
                    error.to_string(),
                    loong_kernel::access::fs_read_error_is_policy_denial(&error),
                )
            })
            .map_err(ToolExecutionError::execution)?;

        file_read_tool_outcome(input, output.path, output.bytes)
            .map_err(ToolExecutionError::execution)
    }
}

pub async fn execute_file_read_payload_with_context<C>(
    raw_tool_name: String,
    payload: Value,
    ctx: &C::Cx<'_>,
) -> Result<ToolOutcome, String>
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + FsAccessContext + Sync,
{
    // Compatibility entry for the legacy direct-read bridge. Keep the bridge
    // payload-level so core request/outcome envelopes stay owned by the app
    // dispatch layer, not by this concrete tool crate.
    let parsed = parse_file_read_payload(raw_tool_name, &payload)?;
    let output = ctx
        .access()
        .fs()
        .read_file(parsed.target.as_str())
        .await
        .map_err(|error| {
            render_fs_read_error(
                error.to_string(),
                loong_kernel::access::fs_read_error_is_policy_denial(&error),
            )
        })?;

    file_read_tool_outcome(parsed, output.path, output.bytes)
}

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

fn parse_file_read_payload(tool_name: String, payload: &Value) -> Result<FileReadRequest, String> {
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

    Ok(FileReadRequest {
        tool_name,
        target,
        max_bytes,
        offset,
        limit,
    })
}

fn render_fs_read_error(rendered: String, policy_denied: bool) -> String {
    if policy_denied {
        format!("policy_denied: {rendered}")
    } else {
        rendered
    }
}

fn file_read_tool_outcome(
    request: FileReadRequest,
    resolved: PathBuf,
    bytes: Vec<u8>,
) -> Result<ToolOutcome, String> {
    let file_text = String::from_utf8_lossy(&bytes).to_string();
    let selection = select_file_read_content(
        file_text.as_str(),
        request.max_bytes,
        request.offset,
        request.limit,
        request.tool_name.as_str(),
    )?;

    let mut response_payload = json!({
        "adapter": "core-tools",
        "tool_name": request.tool_name.as_str(),
        "path": resolved.display().to_string(),
        "bytes": bytes.len(),
        "truncated": selection.truncated,
        "content": selection.content,
    });
    let Some(response_object) = response_payload.as_object_mut() else {
        return Err(format!(
            "{} internal response payload must be an object",
            request.tool_name
        ));
    };
    if let Some(line_start) = selection.line_start {
        response_object.insert("line_start".to_owned(), json!(line_start));
    }
    if let Some(line_end) = selection.line_end {
        response_object.insert("line_end".to_owned(), json!(line_end));
    }
    if let Some(total_lines) = selection.total_lines {
        response_object.insert("total_lines".to_owned(), json!(total_lines));
    }
    if let Some(next_offset) = selection.next_offset {
        response_object.insert("next_offset".to_owned(), json!(next_offset));
    }

    Ok(ToolOutcome {
        status: "ok".to_owned(),
        payload: response_payload,
    })
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
mod tests {
    use super::*;

    #[test]
    fn parse_read_payload_requires_path() {
        let error = parse_file_read_payload("read".to_owned(), &json!({}))
            .expect_err("missing path should fail");

        assert_eq!(error, "read requires payload.path");
    }

    #[test]
    fn parse_read_payload_keeps_window_fields() {
        let parsed = parse_file_read_payload(
            "read".to_owned(),
            &json!({
                "path": "notes.txt",
                "offset": 2,
                "limit": 3,
                "max_bytes": 4,
            }),
        )
        .expect("payload should parse");

        assert_eq!(parsed.target, "notes.txt");
        assert_eq!(parsed.offset, Some(2));
        assert_eq!(parsed.limit, Some(3));
        assert_eq!(parsed.max_bytes, 4);
    }
}
