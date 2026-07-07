use serde_json::{Map, Value, json};

use super::ToolDescriptor;

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
                    }
                },
                "required": ["path", "edits"],
                "additionalProperties": false
            }
        }
    })
}

#[cfg(feature = "tool-shell")]
pub(super) fn direct_bash_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Bash command string to run in the workspace."
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
                "required": ["command"],
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
                    "action": {
                        "type": "string",
                        "enum": ["open", "extract", "click"],
                        "description": "Optional bounded page-browser action override. Leave it unset for the default route."
                    },
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to open in a bounded browser page session."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Existing bounded browser session identifier for follow-up extraction or link traversal."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["page_text", "title", "links", "selector_text"],
                        "description": "Extraction mode used with `session_id`. Defaults to `page_text`."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Optional CSS selector used only with `selector_text` extraction."
                    },
                    "link_id": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "One-based link identifier returned by `browser.open` or `browser.extract` in `links` mode."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "Optional list limit used for `links` or `selector_text` extraction."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_WEB_FETCH_MAX_BYTES,
                        "description": "Optional per-call read limit in bytes when opening a page."
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
