use serde_json::{Value, json};

use super::ToolDescriptor;

// Legacy file descriptor builders stay isolated behind `tool-file` so disabling
// the feature removes the whole file-tool surface from provider/catalog views.
pub(super) fn direct_read_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
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
                    {
                        "required": ["path"]
                    },
                    {
                        "required": ["query"]
                    },
                    {
                        "required": ["pattern"]
                    }
                ],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn direct_write_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Target file path."
                    },
                    "content": {
                        "type": "string",
                        "description": "Whole-file content used for create or replace mode."
                    },
                    "create_dirs": {
                        "type": "boolean",
                        "description": "Create parent directories when missing. Defaults to true."
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "Allow replacing an existing file. Defaults to false."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn direct_edit_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Target file path."
                    },
                    "edits": {
                        "type": "array",
                        "description": "One or more exact text replacement blocks matched against the original file. Merge nearby edits instead of sending overlapping blocks.",
                        "items": exact_edit_block_definition(),
                        "minItems": 1
                    }
                },
                "required": ["path", "edits"],
                "additionalProperties": false
            }
        }
    })
}

fn exact_edit_block_definition() -> Value {
    json!({
        "type": "object",
        "properties": {
            "old_text": {
                "type": "string",
                "minLength": 1,
                "description": "Exact text for one targeted replacement. It must match uniquely in the original file and must not overlap any other edit block."
            },
            "new_text": {
                "type": "string",
                "description": "Replacement text for this targeted edit block."
            }
        },
        "required": ["old_text", "new_text"],
        "additionalProperties": false
    })
}
