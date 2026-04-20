use serde_json::{Value, json};

use super::ToolDescriptor;

pub(super) fn file_read_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Path to read (absolute or relative to configured file root)."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional 1-indexed line number to start from."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional maximum number of lines to return."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 8_388_608,
                        "description": "Optional read limit in bytes. Defaults to 1048576."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn file_write_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Path to write (absolute or relative to configured file root)."
                    },
                    "content": {
                        "type": "string",
                        "description": "File content to write."
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

pub(super) fn glob_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match against workspace-relative paths."
                    },
                    "root": {
                        "type": "string",
                        "description": "Optional search root path. Defaults to the configured file root."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Optional maximum number of matches to return. Defaults to 50."
                    },
                    "include_directories": {
                        "type": "boolean",
                        "description": "Include matching directories in addition to files. Defaults to false."
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn content_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Text to search for inside workspace files."
                    },
                    "root": {
                        "type": "string",
                        "description": "Optional search root path. Defaults to the configured file root."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Optional glob filter applied to workspace-relative file paths before content scanning."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Optional maximum number of matches to return. Defaults to 20."
                    },
                    "max_bytes_per_file": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 1_048_576,
                        "description": "Optional per-file scan budget in bytes. Defaults to 262144."
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "Use case-sensitive matching. Defaults to false."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn memory_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural-language lookup query for durable workspace memory and canonical cross-session recall."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 8,
                        "description": "Optional maximum number of memory hits to return. Defaults to 5."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn memory_get_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Relative or absolute durable memory file path within the configured safe file root."
                    },
                    "from": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional 1-based starting line number. Defaults to 1."
                    },
                    "lines": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Optional number of lines to read. Defaults to 40."
                    }
                },
                "required": ["path"],
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

pub(super) fn file_edit_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Path to the file (absolute or relative to configured file root)."
                    },
                    "edits": {
                        "type": "array",
                        "description": "One or more exact text replacement blocks matched against the original file. Each block must match uniquely and must not overlap another block.",
                        "items": exact_edit_block_definition(),
                        "minItems": 1
                    },
                    "old_string": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Legacy single-block exact edit field. Prefer `edits` for new requests."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Legacy replacement text paired with `old_string`. Prefer `edits` for new requests."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Legacy single-block mode only. Replace all occurrences instead of requiring a unique match. Zero-match still fails regardless of this flag. Defaults to false."
                    }
                },
                "required": ["path"],
                "anyOf": [
                    {
                        "required": ["edits"]
                    },
                    {
                        "required": ["old_string", "new_string"]
                    }
                ],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn http_request_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to request."
                    },
                    "method": {
                        "type": "string",
                        "description": "HTTP method to send. Defaults to GET."
                    },
                    "headers": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "string"
                        },
                        "description": "Optional request headers."
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional request body."
                    },
                    "content_type": {
                        "type": "string",
                        "description": "Optional Content-Type header for the request body."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_WEB_FETCH_MAX_BYTES,
                        "description": "Optional maximum response bytes to return. Cannot exceed the configured runtime max."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn web_fetch_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to fetch."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["readable_text", "raw_text"],
                        "description": "How to render the response body. Defaults to `readable_text`."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_WEB_FETCH_MAX_BYTES,
                        "description": "Optional per-call read limit in bytes. Cannot exceed the configured runtime max."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }
    })
}

#[cfg(feature = "tool-websearch")]
pub(super) fn web_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query string."
                    },
                    "provider": {
                        "type": "string",
                        "enum": crate::config::WEB_SEARCH_PROVIDER_SCHEMA_VALUES,
                        "description": crate::config::web_search_provider_parameter_description()
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 10,
                        "description": "Maximum results to return. Defaults to 5."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn shell_exec_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Executable command name. Must be allowlisted."
                    },
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional command arguments."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1000,
                        "maximum": 600000,
                        "description": "Optional command timeout in milliseconds. Defaults to 120000 and is clamped to 1000..=600000."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn bash_exec_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Bash command to execute."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1000,
                        "maximum": 600000,
                        "description": "Optional command timeout in milliseconds. Defaults to 120000 and is clamped to 1000..=600000."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }
        }
    })
}
