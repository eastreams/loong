use serde_json::{Value, json};

use super::ToolDescriptor;

pub(super) fn approval_request_resolve_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "approval_request_id": {
                        "type": "string",
                        "description": "Visible approval request identifier to resolve."
                    },
                    "decision": {
                        "type": "string",
                        "enum": ["approve_once", "approve_always", "deny"],
                        "description": "Operator decision for the pending approval request."
                    },
                    "session_consent_mode": {
                        "type": "string",
                        "enum": ["auto", "full"],
                        "description": "Optional session consent mode to persist when approve_once wins the request."
                    }
                },
                "required": ["approval_request_id", "decision"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn approval_request_status_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "approval_request_id": {
                        "type": "string",
                        "description": "Visible approval request identifier to inspect in detail."
                    }
                },
                "required": ["approval_request_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn approval_requests_list_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Optional visible session identifier to scope approval requests to one session."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "approved", "executing", "executed", "denied", "expired", "cancelled"],
                        "description": "Optional approval request status filter."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Maximum visible approval requests to return after filtering."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn sessions_list_definition(descriptor: &ToolDescriptor) -> Value {
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
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Number of matching visible sessions to skip before applying limit."
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

pub(super) fn sessions_history_definition(descriptor: &ToolDescriptor) -> Value {
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

pub(super) fn session_events_definition(descriptor: &ToolDescriptor) -> Value {
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

pub(super) fn session_search_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Natural-language search query over visible canonical session history."
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional visible session id to narrow the search scope."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Optional maximum number of ranked hits to return. Defaults to 5."
                    },
                    "include_archived": {
                        "type": "boolean",
                        "description": "Include archived visible sessions when true. Defaults to false."
                    },
                    "include_turns": {
                        "type": "boolean",
                        "description": "Include transcript turn matches. Defaults to true."
                    },
                    "include_events": {
                        "type": "boolean",
                        "description": "Include session event matches. Defaults to true."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_status_definition(descriptor: &ToolDescriptor) -> Value {
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
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn task_status_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Durable task identifier to inspect."
                    },
                    "task_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Durable task identifiers to inspect in one request."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn task_wait_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Durable task identifier to wait on."
                    },
                    "after_id": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Optional event id cursor; when present only newer events are returned."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30000,
                        "description": "Maximum time to wait before returning timeout."
                    }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn task_history_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Durable task identifier to inspect."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Maximum transcript entries to return."
                    }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn task_events_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": descriptor.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "Durable task identifier to inspect."
                    },
                    "after_id": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Optional global event id cursor; when present only newer task events are returned."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200,
                        "description": "Maximum task event rows to return across the visible task lineage."
                    }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn tasks_list_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Maximum number of visible tasks to return."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Zero-based page offset for visible tasks."
                    },
                    "task_state": {
                        "type": "string",
                        "description": "Optional task state filter such as waiting, blocked, completed, or failed."
                    },
                    "stable_only": {
                        "type": "boolean",
                        "description": "When true, return only tasks in a stable state."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn tasks_search_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Search query over visible durable task metadata."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum number of matching tasks to return."
                    },
                    "task_state": {
                        "type": "string",
                        "description": "Optional task state filter such as waiting, blocked, completed, or failed."
                    },
                    "stable_only": {
                        "type": "boolean",
                        "description": "When true, return only tasks in a stable state."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

fn session_tool_runtime_narrowing_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "browser": {
                "type": "object",
                "properties": {
                    "max_sessions": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional upper bound for browser session count."
                    },
                    "max_links": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional upper bound for extracted browser links."
                    },
                    "max_text_chars": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional upper bound for extracted browser text characters."
                    }
                },
                "additionalProperties": false
            },
            "web_fetch": {
                "type": "object",
                "properties": {
                    "allow_private_hosts": {
                        "type": "boolean",
                        "description": "Optional narrowing for private-host access. Use false to deny private hosts."
                    },
                    "enforce_allowed_domains": {
                        "type": "boolean",
                        "description": "When true, enforce the provided allowed_domains list even when it is empty."
                    },
                    "allowed_domains": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "Optional allowlist intersection for web.fetch."
                    },
                    "blocked_domains": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "Optional additional blocked domains for web.fetch."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional maximum web.fetch timeout."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional maximum web.fetch response size in bytes."
                    },
                    "max_redirects": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Optional maximum web.fetch redirect count."
                    }
                },
                "additionalProperties": false
            }
        },
        "additionalProperties": false
    })
}

pub(super) fn session_tool_policy_status_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Optional visible session identifier to inspect. Defaults to the current session."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_tool_policy_set_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Optional visible session identifier to update. Defaults to the current session."
                    },
                    "tool_ids": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "Optional replacement visible tool id set. Use an empty array to clear the session-specific tool surface restriction."
                    },
                    "runtime_narrowing": session_tool_runtime_narrowing_schema()
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_tool_policy_clear_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Optional visible session identifier to clear. Defaults to the current session."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_recover_definition(descriptor: &ToolDescriptor) -> Value {
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
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_archive_definition(descriptor: &ToolDescriptor) -> Value {
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
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_cancel_definition(descriptor: &ToolDescriptor) -> Value {
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
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_continue_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Visible delegate child session identifier to continue."
                    },
                    "input": {
                        "type": "string",
                        "description": "Follow-up user input to execute inside the target child session."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional bounded timeout override for the continued child turn."
                    }
                },
                "required": ["session_id", "input"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn session_wait_definition(descriptor: &ToolDescriptor) -> Value {
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
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn sessions_send_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Known channel-backed root session identifier to receive the outbound text message (for example Telegram, Feishu, or Matrix)."
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

pub(super) fn delegate_definition(descriptor: &ToolDescriptor) -> Value {
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
                    "profile": {
                        "type": "string",
                        "enum": ["research", "plan", "verify"],
                        "description": "Optional builtin child profile preset. `research`, `plan`, and `verify` apply bounded delegate role defaults."
                    },
                    "isolation": {
                        "type": "string",
                        "enum": ["shared", "worktree"],
                        "description": "Optional child workspace isolation mode. `shared` reuses the current workspace root. `worktree` is reserved for a dedicated git worktree-backed child root and currently returns a not-supported error until that runtime lane lands."
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

pub(super) fn delegate_async_definition(descriptor: &ToolDescriptor) -> Value {
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
                    "profile": {
                        "type": "string",
                        "enum": ["research", "plan", "verify"],
                        "description": "Optional builtin child profile preset. `research`, `plan`, and `verify` apply bounded delegate role defaults."
                    },
                    "isolation": {
                        "type": "string",
                        "enum": ["shared", "worktree"],
                        "description": "Optional child workspace isolation mode. `shared` reuses the current workspace root. `worktree` is reserved for a dedicated git worktree-backed child root and currently returns a not-supported error until that runtime lane lands."
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
