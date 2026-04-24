use serde_json::{Map, Value, json};

use super::ToolDescriptor;

pub(super) fn tool_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Discover a specialized tool when the direct tools do not fit the task.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Short capability phrase for the specialized tool you need. Match the prompt snippets shown in the system prompt."
                    },
                    "exact_tool_id": {
                        "type": "string",
                        "description": "Optional exact tool id to refresh a known visible tool card."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Optional maximum number of search results to return."
                    }
                },
                "required": [],
                "additionalProperties": false
            }
        }
    })
}

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

pub(super) fn direct_grep_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Search workspace file contents for this text."
                    },
                    "root": {
                        "type": "string",
                        "description": "Optional search root path."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Optional file glob filter applied during content search."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Optional maximum result count."
                    },
                    "max_bytes_per_file": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 1_048_576,
                        "description": "Optional per-file scan budget."
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

pub(super) fn direct_find_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "List workspace-relative paths that match this glob pattern."
                    },
                    "root": {
                        "type": "string",
                        "description": "Optional search root path."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Optional maximum result count."
                    },
                    "include_directories": {
                        "type": "boolean",
                        "description": "Include matching directories. Defaults to false."
                    }
                },
                "required": ["pattern"],
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
                        "description": "Legacy single-block mode only. Replace all matches instead of requiring a unique match. Defaults to false."
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

pub(super) fn direct_exec_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Executable command or simple shell command. Routes to argv mode unless it clearly uses shell syntax."
                    },
                    "script": {
                        "type": "string",
                        "description": "Raw shell or bash script text. Use this for pipes, redirects, chaining, or multi-line commands."
                    },
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional command arguments for argv mode."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1000,
                        "maximum": 600000,
                        "description": "Optional command timeout in milliseconds."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory."
                    }
                },
                "anyOf": [
                    {
                        "required": ["command"]
                    },
                    {
                        "required": ["script"]
                    }
                ],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn direct_web_definition(descriptor: &ToolDescriptor) -> Value {
    let base_properties = || {
        Map::from_iter([
            (
                "url".to_owned(),
                json!({
                    "type": "string",
                    "description": "Fetch or request this HTTP or HTTPS URL without using a web-search provider."
                }),
            ),
            (
                "mode".to_owned(),
                json!({
                    "type": "string",
                    "enum": ["readable_text", "raw_text"],
                    "description": "Fetch rendering mode. Used only for plain fetch mode."
                }),
            ),
            (
                "max_bytes".to_owned(),
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": crate::config::MAX_WEB_FETCH_MAX_BYTES,
                    "description": "Optional response byte limit."
                }),
            ),
            (
                "method".to_owned(),
                json!({
                    "type": "string",
                    "description": "Optional HTTP method. When present, web routes to low-level request mode."
                }),
            ),
            (
                "headers".to_owned(),
                json!({
                    "type": "object",
                    "additionalProperties": {"type": "string"},
                    "description": "Optional HTTP headers for request mode."
                }),
            ),
            (
                "body".to_owned(),
                json!({
                    "type": "string",
                    "description": "Optional request body for request mode."
                }),
            ),
            (
                "content_type".to_owned(),
                json!({
                    "type": "string",
                    "description": "Optional Content-Type header for request mode."
                }),
            ),
        ])
    };

    #[cfg(feature = "tool-websearch")]
    let (properties, any_of) = {
        let mut properties = base_properties();
        properties.insert(
            "query".to_owned(),
            json!({
                "type": "string",
                "description": "Search the public web for this query through web-search providers. This is separate from plain URL fetch/request mode."
            }),
        );
        properties.insert(
            "provider".to_owned(),
            json!({
                "type": "string",
                "enum": crate::config::WEB_SEARCH_PROVIDER_SCHEMA_VALUES,
                "description": crate::config::web_search_provider_parameter_description()
            }),
        );
        properties.insert(
            "max_results".to_owned(),
            json!({
                "type": "integer",
                "minimum": 1,
                "maximum": 10,
                "description": "Optional maximum result count in search mode."
            }),
        );
        (
            properties,
            vec![json!({"required": ["url"]}), json!({"required": ["query"]})],
        )
    };

    #[cfg(not(feature = "tool-websearch"))]
    let (properties, any_of) = (base_properties(), vec![json!({"required": ["url"]})]);

    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "anyOf": any_of,
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn direct_browser_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Open a page or continue a browser session at this HTTP or HTTPS URL."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_WEB_FETCH_MAX_BYTES,
                        "description": "Optional byte limit used when opening a page."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Existing browser session identifier for follow-up reads or interactions."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["page_text", "title", "links", "selector_text", "summary", "html"],
                        "description": "Read mode for browser inspection."
                    },
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for focused extraction or browser interaction."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "Maximum extracted items when the browser result returns a list."
                    },
                    "link_id": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "One-based link identifier returned by the current page snapshot."
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type into the selected element."
                    },
                    "condition": {
                        "type": "string",
                        "description": "Optional wait condition for browser session progress."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30000,
                        "description": "Optional wait timeout in milliseconds."
                    }
                },
                "anyOf": [
                    {
                        "required": ["url"]
                    },
                    {
                        "required": ["session_id"]
                    },
                    {
                        "required": ["session_id", "link_id"]
                    },
                    {
                        "required": ["session_id", "selector"]
                    },
                    {
                        "required": ["session_id", "selector", "text"]
                    },
                    {
                        "required": ["session_id", "url"]
                    }
                ],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn direct_memory_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Search durable memory and canonical recall for this query."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 8,
                        "description": "Optional maximum number of memory hits to return."
                    },
                    "path": {
                        "type": "string",
                        "description": "Read one durable memory file at this path."
                    },
                    "from": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional 1-based starting line number for path mode."
                    },
                    "lines": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Optional number of lines to read in path mode."
                    }
                },
                "anyOf": [
                    {
                        "required": ["query"]
                    },
                    {
                        "required": ["path"]
                    }
                ],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn tool_invoke_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Invoke a discovered non-core tool using a valid lease from tool_search.",
            "parameters": {
                "type": "object",
                "properties": {
                    "tool_id": {
                        "type": "string",
                        "description": "Canonical id of the discovered tool."
                    },
                    "lease": {
                        "type": "string",
                        "description": "Short-lived lease returned by tool_search."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments for the discovered tool payload."
                    }
                },
                "required": ["tool_id", "lease", "arguments"],
                "additionalProperties": false
            }
        }
    })
}
