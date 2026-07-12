use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use loong_contracts::{Capability, ToolExecutionError, ToolInputError, ToolSpec};
use loong_core::{policy::context::ContextFactory, tool::ToolImpl};
use loong_kernel::{KernelAccess, access::fs::FsWriteOptions};
use serde_json::{Value, json};

use super::{fs_access_error_reason, required_trimmed_string_field};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactTextEditBlock {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRequest {
    pub(super) tool_name: String,
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
    pub tool_name: String,
    pub path: PathBuf,
    pub before: String,
    pub after: String,
    pub replacements_made: usize,
    pub edit_blocks_applied: usize,
}

impl From<EditOutput> for Value {
    fn from(output: EditOutput) -> Self {
        json!({
            "adapter": "core-tools",
            "tool_name": output.tool_name,
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

pub struct EditTool {
    tool_name: &'static str,
}

impl EditTool {
    /// Creates the exact text edit implementation for an app-facing tool name.
    ///
    /// The app plane owns registration and preview-event observation. This tool
    /// only parses the edit payload, calls governed fs access, and returns a
    /// typed output that the app boundary can observe before JSON erasure.
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
    for<'a> C::Cx<'a>: KernelAccess<C> + Sync,
{
    type Input = EditRequest;
    type Output = EditOutput;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Apply exact text edits to a file in allowed roots.".to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
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
        EditRequest::parse_payload(self.tool_name.to_owned(), &payload)
            .map_err(ToolInputError::invalid_payload)
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError> {
        let read_output = ctx
            .access()
            .fs()
            .read_file(input.path.as_str())
            .await
            .map_err(fs_access_error_reason)
            .map_err(ToolExecutionError::execution)?;
        let before = String::from_utf8(read_output.bytes).map_err(|source| {
            ToolExecutionError::execution(format!(
                "failed to decode {} as UTF-8: {source}",
                read_output.path.display()
            ))
        })?;
        let applied = Self::apply_exact_edit_blocks(before.as_str(), input.blocks.as_slice())
            .map_err(ToolExecutionError::execution)?;

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
            .await
            .map_err(fs_access_error_reason)
            .map_err(ToolExecutionError::execution)?;

        Ok(EditOutput {
            tool_name: input.tool_name,
            path: write_output.path,
            before,
            after: applied.updated,
            replacements_made: applied.replacements_made,
            edit_blocks_applied: input.blocks.len(),
        })
    }
}

impl EditRequest {
    pub(super) fn parse_payload(tool_name: String, payload: &Value) -> Result<Self, String> {
        let payload = payload
            .as_object()
            .ok_or_else(|| format!("{tool_name} payload must be an object"))?;
        let path = required_trimmed_string_field(payload, "path", tool_name.as_str())?.to_owned();
        let blocks = parse_exact_edit_blocks(payload, tool_name.as_str())?;

        Ok(Self {
            tool_name,
            path,
            blocks,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocatedExactTextEditBlock<'a> {
    start: usize,
    end: usize,
    block: &'a ExactTextEditBlock,
}

fn exact_edit_block_field<'a>(
    block: &'a serde_json::Map<String, Value>,
    snake_case_field: &str,
    camel_case_field: &str,
) -> Option<&'a str> {
    block
        .get(snake_case_field)
        .and_then(Value::as_str)
        .or_else(|| block.get(camel_case_field).and_then(Value::as_str))
}

fn parse_exact_edit_blocks(
    payload: &serde_json::Map<String, Value>,
    tool_name: &str,
) -> Result<Vec<ExactTextEditBlock>, String> {
    let raw_blocks = payload
        .get("edits")
        .ok_or_else(|| format!("{tool_name} requires payload.edits"))?;
    let blocks = raw_blocks
        .as_array()
        .ok_or_else(|| format!("{tool_name} payload.edits must be an array"))?;
    if blocks.is_empty() {
        return Err(format!(
            "{tool_name} payload.edits must contain at least one edit block"
        ));
    }

    let mut parsed_blocks = Vec::with_capacity(blocks.len());
    for (index, raw_block) in blocks.iter().enumerate() {
        let block = raw_block
            .as_object()
            .ok_or_else(|| format!("{tool_name} payload.edits[{index}] must be an object"))?;
        let old_text = exact_edit_block_field(block, "old_text", "oldText").ok_or_else(|| {
            format!("{tool_name} payload.edits[{index}].old_text must be a string")
        })?;
        if old_text.is_empty() {
            return Err(format!(
                "edit_failed: edits[{index}].old_text must not be empty"
            ));
        }
        let new_text = exact_edit_block_field(block, "new_text", "newText").ok_or_else(|| {
            format!("{tool_name} payload.edits[{index}].new_text must be a string")
        })?;
        parsed_blocks.push(ExactTextEditBlock {
            old_text: old_text.to_owned(),
            new_text: new_text.to_owned(),
        });
    }

    Ok(parsed_blocks)
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
