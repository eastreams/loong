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
        FsPathError, FsPathPolicyContext, FsReadError, FsResolutionContext, FsWriteError,
        FsWriteOptions,
    },
};
use serde_json::{Value, json};
use thiserror::Error;

use super::required_trimmed_string_field;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactTextEditBlock {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    pub(super) path: String,
    pub(super) blocks: Vec<ExactTextEditBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedExactEdit {
    pub(super) updated: String,
    pub(super) replacements_made: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOutput {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
    pub replacements_made: usize,
    pub edit_blocks_applied: usize,
}

/// Failures owned by the read-transform-write edit operation.
#[derive(Debug, Error)]
pub enum EditToolError {
    #[error("{0}")]
    Read(
        #[from]
        #[source]
        FsReadError,
    ),
    #[error("{0}")]
    Write(
        #[from]
        #[source]
        FsWriteError,
    ),
    /// File editing currently requires UTF-8 text. Supporting other encodings
    /// may require an explicit decoding policy later; this path does not guess one yet.
    #[error("failed to decode {path} as UTF-8: {source}", path = .path.display())]
    InvalidUtf8 {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("{reason}")]
    ApplyEdit { reason: String },
}

impl From<EditOutput> for Value {
    fn from(output: EditOutput) -> Self {
        json!({
            "path": output.path.display().to_string(),
            "replacements_made": output.replacements_made,
            "bytes_written": output.after.len(),
            "edit_blocks_applied": output.edit_blocks_applied,
            "continuation": {
                "state": "verify_file_change",
                "is_terminal": false,
                "recommended_tool": "read",
                "recommended_payload": {
                    "path": output.path.display().to_string()
                },
                "note": "If the user still depends on the updated file contents, verify the file before finalizing."
            }
        })
    }
}

pub struct EditTool;

impl EditTool {
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Edit this workspace-relative or absolute file path."
                },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": { "type": "string" },
                            "oldText": { "type": "string" },
                            "new_text": { "type": "string" },
                            "newText": { "type": "string" }
                        },
                        "additionalProperties": false
                    }
                }
            },
            "required": ["path", "edits"],
            "additionalProperties": false
        })
    }

    pub(super) fn apply_exact_edit_blocks(
        content: &str,
        blocks: &[ExactTextEditBlock],
    ) -> Result<AppliedExactEdit, String> {
        let mut located_blocks = Vec::with_capacity(blocks.len());
        for (index, block) in blocks.iter().enumerate() {
            let located_block = locate_exact_edit_block(content, block, index)?;
            located_blocks.push(located_block);
        }
        located_blocks.sort_by_key(|left| left.start);

        for window in located_blocks.windows(2) {
            let [left, right] = window else {
                continue;
            };
            if left.end > right.start {
                return Err(
                    "edit_failed: edit blocks overlap in the original file; merge nested or overlapping edits into one block"
                        .to_owned(),
                );
            }
        }

        let mut updated = String::with_capacity(content.len());
        let mut cursor = 0usize;
        for located_block in &located_blocks {
            updated.push_str(&content[cursor..located_block.start]);
            updated.push_str(located_block.block.new_text.as_str());
            cursor = located_block.end;
        }
        updated.push_str(&content[cursor..]);

        Ok(AppliedExactEdit {
            updated,
            replacements_made: located_blocks.len(),
        })
    }
}

/// Concrete builtin implementation for exact text edits.
///
/// The filesystem read/write side effects stay behind `ctx.access().fs()`.
/// App-only preview events are produced by the registering app plane observer,
/// not by this concrete tool crate.
#[async_trait]
impl<C> ToolImpl<C> for EditTool
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + FsResolutionContext + FsPathPolicyContext + Sync,
{
    type Input = EditRequest;
    type Output = EditOutput;
    type Error = EditToolError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Apply exact text edits to a file in allowed roots.".to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
            scheduling: ToolSchedulingClass::SerialOnly,
            argument_hint: Some("path:string,edits:[{old_text:string,new_text:string}]".to_owned()),
            search_hint: Some(
                "replace one or more uniquely matching exact text blocks in a file".to_owned(),
            ),
            tags: ["surface", "edit", "file"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        EditRequest::parse_payload(&payload)
    }

    fn failure_kind(&self, error: &Self::Error) -> ToolFailureKind {
        match error {
            EditToolError::Read(
                FsReadError::Authorization(source)
                | FsReadError::Path(FsPathError::Authorization(source)),
            )
            | EditToolError::Write(
                FsWriteError::Authorization(source)
                | FsWriteError::Path(FsPathError::Authorization(source)),
            ) if matches!(
                source,
                PolicyGrantError::MissingCapability { .. }
                    | PolicyGrantError::Denied { .. }
                    | PolicyGrantError::PermissionDenied { .. }
            ) =>
            {
                ToolFailureKind::Denied
            }
            EditToolError::Read(_)
            | EditToolError::Write(_)
            | EditToolError::InvalidUtf8 { .. }
            | EditToolError::ApplyEdit { .. } => ToolFailureKind::Execution,
        }
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        let read_output = ctx.access().fs().read_file(input.path.as_str()).await?;
        let before =
            String::from_utf8(read_output.bytes).map_err(|source| EditToolError::InvalidUtf8 {
                path: read_output.path.clone(),
                source,
            })?;
        let applied = Self::apply_exact_edit_blocks(before.as_str(), input.blocks.as_slice())
            .map_err(|reason| EditToolError::ApplyEdit { reason })?;

        let write_output = ctx
            .access()
            .fs()
            .write_file(
                read_output.path.as_path(),
                applied.updated.as_bytes().to_vec(),
                FsWriteOptions {
                    create_dirs: false,
                    overwrite: true,
                },
            )
            .await?;

        Ok(EditOutput {
            path: write_output.path,
            before,
            after: applied.updated,
            replacements_made: applied.replacements_made,
            edit_blocks_applied: input.blocks.len(),
        })
    }
}

impl EditRequest {
    pub(super) fn parse_payload(payload: &Value) -> Result<Self, ToolInputError> {
        let payload = payload
            .as_object()
            .ok_or(ToolInputError::PayloadMustBeObject)?;
        let path = required_trimmed_string_field(payload, "path")?.to_owned();
        let raw_blocks = payload
            .get("edits")
            .ok_or_else(|| ToolInputError::missing_field("edits"))?;
        let blocks = raw_blocks
            .as_array()
            .ok_or_else(|| ToolInputError::invalid_field("edits", "must be an array"))?;
        if blocks.is_empty() {
            return Err(ToolInputError::invalid_field(
                "edits",
                "must contain at least one edit block",
            ));
        }

        let mut parsed_blocks = Vec::with_capacity(blocks.len());
        for (index, raw_block) in blocks.iter().enumerate() {
            let field_prefix = format!("edits[{index}]");
            let block = raw_block
                .as_object()
                .ok_or_else(|| ToolInputError::invalid_field(field_prefix, "must be an object"))?;
            let old_text =
                required_exact_edit_block_string_field(block, "old_text", "oldText", index)?;
            if old_text.is_empty() {
                return Err(ToolInputError::invalid_field(
                    format!("edits[{index}].old_text"),
                    "must not be empty",
                ));
            }
            let new_text =
                required_exact_edit_block_string_field(block, "new_text", "newText", index)?;
            parsed_blocks.push(ExactTextEditBlock {
                old_text: old_text.to_owned(),
                new_text: new_text.to_owned(),
            });
        }

        Ok(Self {
            path,
            blocks: parsed_blocks,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocatedExactTextEditBlock<'a> {
    start: usize,
    end: usize,
    block: &'a ExactTextEditBlock,
}

// Both edit text fields accept snake_case and camelCase aliases. Keeping that
// precedence here also gives every nested failure the same canonical field path.
fn required_exact_edit_block_string_field<'a>(
    block: &'a serde_json::Map<String, Value>,
    snake_case_field: &str,
    camel_case_field: &str,
    index: usize,
) -> Result<&'a str, ToolInputError> {
    if let Some(value) = block.get(snake_case_field).and_then(Value::as_str) {
        return Ok(value);
    }
    if let Some(value) = block.get(camel_case_field).and_then(Value::as_str) {
        return Ok(value);
    }

    let field = format!("edits[{index}].{snake_case_field}");
    if block.contains_key(snake_case_field) || block.contains_key(camel_case_field) {
        Err(ToolInputError::invalid_field(field, "must be a string"))
    } else {
        Err(ToolInputError::missing_field(field))
    }
}

fn locate_exact_edit_block<'a>(
    content: &str,
    block: &'a ExactTextEditBlock,
    index: usize,
) -> Result<LocatedExactTextEditBlock<'a>, String> {
    let match_offsets = content
        .match_indices(block.old_text.as_str())
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    if match_offsets.is_empty() {
        return Err(format!(
            "edit_failed: edits[{index}].old_text not found in file"
        ));
    }
    if match_offsets.len() > 1 {
        return Err(format!(
            "edit_failed: edits[{index}].old_text matches {} locations; each edit block must match uniquely in the original file",
            match_offsets.len()
        ));
    }

    let start = match_offsets
        .first()
        .copied()
        .ok_or_else(|| format!("edit_failed: edits[{index}].old_text not found in file"))?;
    let end = start.saturating_add(block.old_text.len());
    Ok(LocatedExactTextEditBlock { start, end, block })
}
