use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::config::ToolConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionKind {
    Core,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAvailability {
    Runtime,
    Planned,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub provider_name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub execution_kind: ToolExecutionKind,
    pub availability: ToolAvailability,
    provider_definition_builder: fn(&ToolDescriptor) -> Value,
}

impl ToolDescriptor {
    pub fn matches_name(&self, raw: &str) -> bool {
        self.name == raw
            || self.provider_name == raw
            || self.aliases.iter().any(|alias| *alias == raw)
    }

    pub fn provider_definition(&self) -> Value {
        (self.provider_definition_builder)(self)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolView {
    allowed_names: BTreeSet<String>,
}

impl ToolView {
    pub fn from_tool_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            allowed_names: names
                .into_iter()
                .map(|name| name.as_ref().to_owned())
                .collect(),
        }
    }

    pub fn contains(&self, tool_name: &str) -> bool {
        self.allowed_names.contains(tool_name)
    }

    pub fn iter<'a>(
        &'a self,
        catalog: &'a ToolCatalog,
    ) -> impl Iterator<Item = &'a ToolDescriptor> + 'a {
        catalog
            .descriptors
            .iter()
            .filter(move |descriptor| self.contains(descriptor.name))
    }
}

#[derive(Debug, Clone)]
pub struct ToolCatalog {
    descriptors: Vec<ToolDescriptor>,
}

impl ToolCatalog {
    pub fn descriptor(&self, tool_name: &str) -> Option<&ToolDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.name == tool_name)
    }

    pub fn resolve(&self, raw_tool_name: &str) -> Option<&ToolDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.matches_name(raw_tool_name))
    }

    pub fn descriptors(&self) -> &[ToolDescriptor] {
        &self.descriptors
    }
}

pub fn tool_catalog() -> ToolCatalog {
    let mut descriptors = Vec::new();

    #[cfg(feature = "tool-file")]
    {
        descriptors.push(ToolDescriptor {
            name: "file.read",
            provider_name: "file_read",
            aliases: &[],
            description: "Read file contents",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: file_read_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "file.write",
            provider_name: "file_write",
            aliases: &[],
            description: "Write file contents",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: file_write_definition,
        });
    }

    #[cfg(feature = "tool-shell")]
    {
        descriptors.push(ToolDescriptor {
            name: "shell.exec",
            provider_name: "shell_exec",
            aliases: &["shell"],
            description: "Execute shell commands",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: shell_exec_definition,
        });
    }

    descriptors.push(ToolDescriptor {
        name: "sessions_list",
        provider_name: "sessions_list",
        aliases: &[],
        description: "List visible sessions and their high-level state",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: sessions_list_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "session_events",
        provider_name: "session_events",
        aliases: &[],
        description: "Fetch session events for a visible session",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
            provider_definition_builder: session_events_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "memory_search",
        provider_name: "memory_search",
        aliases: &[],
        description: "Search visible transcript memory across persisted session turns",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: memory_search_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "sessions_history",
        provider_name: "sessions_history",
        aliases: &[],
        description: "Fetch transcript history for a visible session",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: sessions_history_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "session_status",
        provider_name: "session_status",
        aliases: &[],
        description: "Inspect the current status of a visible session",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: session_status_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "session_recover",
        provider_name: "session_recover",
        aliases: &[],
        description: "Recover an overdue queued async delegate child session by marking it failed",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: session_recover_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "session_archive",
        provider_name: "session_archive",
        aliases: &[],
        description: "Archive a visible terminal session from default session listings",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: session_archive_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "session_unarchive",
        provider_name: "session_unarchive",
        aliases: &[],
        description: "Restore a visible archived terminal session to default session listings",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: session_unarchive_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "session_cancel",
        provider_name: "session_cancel",
        aliases: &[],
        description: "Cancel a visible async delegate child session",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: session_cancel_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "session_wait",
        provider_name: "session_wait",
        aliases: &[],
        description: "Wait for a visible session to reach a terminal state",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: session_wait_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "sessions_send",
        provider_name: "sessions_send",
        aliases: &[],
        description: "Send an outbound text message to a known channel-backed root session",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: sessions_send_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "delegate",
        provider_name: "delegate",
        aliases: &[],
        description: "Delegate a focused subtask into a child session",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: delegate_definition,
    });
    descriptors.push(ToolDescriptor {
        name: "delegate_async",
        provider_name: "delegate_async",
        aliases: &[],
        description: "Delegate a focused subtask into a background child session",
        execution_kind: ToolExecutionKind::App,
        availability: ToolAvailability::Runtime,
        provider_definition_builder: delegate_async_definition,
    });

    descriptors.sort_by(|left, right| left.name.cmp(right.name));

    ToolCatalog { descriptors }
}

pub fn runtime_tool_view() -> ToolView {
    runtime_tool_view_for_config(&ToolConfig::default())
}

pub fn runtime_tool_view_for_config(config: &ToolConfig) -> ToolView {
    let catalog = tool_catalog();
    ToolView::from_tool_names(
        catalog
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.availability == ToolAvailability::Runtime)
            .filter(|descriptor| tool_is_enabled_for_runtime_view(descriptor.name, config))
            .map(|descriptor| descriptor.name),
    )
}

pub fn planned_root_tool_view() -> ToolView {
    let catalog = tool_catalog();
    ToolView::from_tool_names(
        catalog
            .descriptors()
            .iter()
            .map(|descriptor| descriptor.name),
    )
}

pub fn planned_delegate_child_tool_view() -> ToolView {
    delegate_child_tool_view_for_config(&ToolConfig::default())
}

pub fn delegate_child_tool_view_for_config(config: &ToolConfig) -> ToolView {
    delegate_child_tool_view_for_config_with_delegate(config, false)
}

pub fn delegate_child_tool_view_for_config_with_delegate(
    config: &ToolConfig,
    allow_delegate: bool,
) -> ToolView {
    let catalog = tool_catalog();
    let mut names = vec!["session_status", "sessions_history"];
    let allowlist = BTreeSet::<&str>::from_iter(
        config
            .delegate
            .child_tool_allowlist
            .iter()
            .map(String::as_str),
    );

    for descriptor in catalog.descriptors().iter().filter(|descriptor| {
        descriptor.execution_kind == ToolExecutionKind::Core
            && descriptor.availability == ToolAvailability::Runtime
    }) {
        match descriptor.name {
            "shell.exec" =>
            {
                #[cfg(feature = "tool-shell")]
                if config.delegate.allow_shell_in_child {
                    names.push(descriptor.name);
                }
            }
            name if allowlist.contains(name) => names.push(name),
            _ => {}
        }
    }

    if allow_delegate && config.delegate.enabled {
        names.push("delegate");
        names.push("delegate_async");
    }

    ToolView::from_tool_names(names)
}

fn tool_is_enabled_for_runtime_view(tool_name: &str, config: &ToolConfig) -> bool {
    match tool_name {
        "sessions_list" | "sessions_history" | "session_status" | "session_events"
        | "memory_search" | "session_archive" | "session_cancel" | "session_recover"
        | "session_unarchive" | "session_wait" => config.sessions.enabled,
        "sessions_send" => config.messages.enabled,
        "delegate" | "delegate_async" => config.delegate.enabled,
        _ => true,
    }
}

fn file_read_definition(descriptor: &ToolDescriptor) -> Value {
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

fn file_write_definition(descriptor: &ToolDescriptor) -> Value {
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
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }
        }
    })
}

fn shell_exec_definition(descriptor: &ToolDescriptor) -> Value {
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

fn sessions_list_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Maximum visible sessions to return after filtering."
                    },
                    "state": {
                        "type": "string",
                        "enum": ["ready", "running", "completed", "failed", "timed_out"],
                        "description": "Optional lifecycle state filter."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["root", "delegate_child"],
                        "description": "Optional session kind filter."
                    },
                    "parent_session_id": {
                        "type": "string",
                        "description": "Optional direct parent session filter."
                    },
                    "overdue_only": {
                        "type": "boolean",
                        "description": "When true, only return async delegate children whose lifecycle staleness is overdue."
                    },
                    "include_archived": {
                        "type": "boolean",
                        "description": "When true, include archived visible sessions in the returned list."
                    },
                    "include_delegate_lifecycle": {
                        "type": "boolean",
                        "description": "When true, include normalized delegate lifecycle metadata for returned sessions."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

fn sessions_history_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible session identifier to inspect."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Maximum transcript entries to return."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

fn memory_search_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Non-empty transcript text to search for within visible session memory."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional visible session identifier to search exclusively."
                    },
                    "session_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Optional visible session identifiers to search in one request."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Maximum transcript matches to return."
                    },
                    "excerpt_chars": {
                        "type": "integer",
                        "minimum": 40,
                        "maximum": 400,
                        "description": "Approximate match snippet length in characters."
                    }
                },
                "required": ["query"],
                "oneOf": [
                    { "required": ["query", "session_id"] },
                    { "required": ["query", "session_ids"] },
                    { "required": ["query"] }
                ],
                "additionalProperties": false
            }
        }
    })
}

fn session_events_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible session identifier to inspect."
                    },
                    "after_id": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Optional event id cursor; when present only newer events are returned."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Maximum event rows to return."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

fn session_status_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible session identifier to inspect."
                    },
                    "session_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Visible session identifiers to inspect in one request."
                    }
                },
                "oneOf": [
                    { "required": ["session_id"] },
                    { "required": ["session_ids"] }
                ],
                "additionalProperties": false
            }
        }
    })
}

fn session_recover_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible delegate child session identifier to recover."
                    },
                    "session_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Visible delegate child session identifiers to recover in one request."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "When true, preview which targets are recoverable without mutating state."
                    }
                },
                "oneOf": [
                    { "required": ["session_id"] },
                    { "required": ["session_ids"] }
                ],
                "additionalProperties": false
            }
        }
    })
}

fn session_archive_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible terminal session identifier to archive."
                    },
                    "session_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Visible terminal session identifiers to archive in one request."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "When true, preview which targets are archivable without mutating state."
                    }
                },
                "oneOf": [
                    { "required": ["session_id"] },
                    { "required": ["session_ids"] }
                ],
                "additionalProperties": false
            }
        }
    })
}

fn session_unarchive_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible archived terminal session identifier to restore to default listings."
                    },
                    "session_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Visible archived terminal session identifiers to restore in one request."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "When true, preview which targets are unarchivable without mutating state."
                    }
                },
                "oneOf": [
                    { "required": ["session_id"] },
                    { "required": ["session_ids"] }
                ],
                "additionalProperties": false
            }
        }
    })
}

fn session_cancel_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible async delegate child session identifier to cancel."
                    },
                    "session_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Visible async delegate child session identifiers to cancel in one request."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "When true, preview which targets are cancellable without mutating state."
                    }
                },
                "oneOf": [
                    { "required": ["session_id"] },
                    { "required": ["session_ids"] }
                ],
                "additionalProperties": false
            }
        }
    })
}

fn session_wait_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Visible session identifier to wait on."
                    },
                    "session_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Visible session identifiers to wait on in one request."
                    },
                    "after_id": {
                        "type": "integer",
                        "description": "Optional event cursor. When present, the response also returns session events with id greater than this value."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30000,
                        "description": "Bounded wait timeout in milliseconds."
                    }
                },
                "oneOf": [
                    { "required": ["session_id"] },
                    { "required": ["session_ids"] }
                ],
                "additionalProperties": false
            }
        }
    })
}

fn sessions_send_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Known Telegram or Feishu root session identifier to receive the outbound text message."
                    },
                    "text": {
                        "type": "string",
                        "description": "Outbound plain-text message content."
                    }
                },
                "required": ["session_id", "text"],
                "additionalProperties": false
            }
        }
    })
}

fn delegate_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "Focused subtask to run in a child session."
                    },
                    "label": {
                        "type": "string",
                        "description": "Optional human-readable label for the child session."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional timeout for the delegated task."
                    }
                },
                "required": ["task"],
                "additionalProperties": false
            }
        }
    })
}

fn delegate_async_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "Focused subtask to run in a background child session."
                    },
                    "label": {
                        "type": "string",
                        "description": "Optional human-readable label for the child session."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional timeout for the delegated task."
                    }
                },
                "required": ["task"],
                "additionalProperties": false
            }
        }
    })
}
