use serde_json::{Value, json};

use super::ToolDescriptor;

pub(super) fn config_import_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Import, discover, plan, merge, apply, and roll back legacy agent workspace config and related external-skills state into native Loong config.",
            "parameters": {
                "type": "object",
                "properties": {
                    "input_path": {
                        "type": "string",
                        "description": "Path to the legacy agent workspace, config root, or portable import file. Required for all modes except rollback_last_apply."
                    },
                    "mode": {
                        "type": "string",
                        "enum": [
                            "plan",
                            "apply",
                            "discover",
                            "plan_many",
                            "recommend_primary",
                            "merge_profiles",
                            "map_external_skills",
                            "apply_selected",
                            "rollback_last_apply"
                        ],
                        "description": "Migration mode. Defaults to `plan` when omitted."
                    },
                    "source": {
                        "type": "string",
                        "enum": ["auto", "nanobot", "openclaw", "picoclaw", "zeroclaw", "nanoclaw"],
                        "description": "Optional claw-family source hint for plan/apply modes. Defaults to automatic detection."
                    },
                    "source_id": {
                        "type": "string",
                        "description": "Selected source identifier for apply_selected mode."
                    },
                    "selection_id": {
                        "type": "string",
                        "description": "Alias of source_id for apply_selected mode."
                    },
                    "primary_source_id": {
                        "type": "string",
                        "description": "Primary source identifier for safe profile merge in apply_selected mode."
                    },
                    "primary_selection_id": {
                        "type": "string",
                        "description": "Alias of primary_source_id for safe profile merge in apply_selected mode."
                    },
                    "safe_profile_merge": {
                        "type": "boolean",
                        "description": "Enable safe multi-source profile merge in apply_selected mode."
                    },
                    "apply_external_skills_plan": {
                        "type": "boolean",
                        "description": "When true, apply a generated external-skills mapping addendum into profile_note during apply_selected."
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Optional target config path. In plan, when present, config.import reads this path to preview the merged result. Required in apply/apply_selected/rollback_last_apply modes."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Overwrite an existing target config when applying. Defaults to false."
                    }
                },
                "required": [],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn provider_switch_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Inspect current provider state or switch the default provider profile for subsequent turns when the user explicitly wants future replies to use another configured provider, profile, or model.",
            "parameters": {
                "type": "object",
                "properties": {
                    "selector": {
                        "type": "string",
                        "description": format!(
                            "Optional provider selector. Accepts a {} such as `openai-gpt-5`, `gpt-5.1-codex`, or `deepseek`. When omitted, the tool reports current provider state without changing it.",
                            crate::config::PROVIDER_SELECTOR_HUMAN_SUMMARY
                        )
                    }
                },
                "required": []
            }
        }
    })
}

pub(super) fn external_skills_policy_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Get, set, or reset runtime policy for external skills downloads (enabled flag, approval gate, domain allowlist/blocklist).",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "set", "reset"],
                        "description": "Policy action. Defaults to `get`."
                    },
                    "policy_update_approved": {
                        "type": "boolean",
                        "description": "Explicit user authorization for policy updates. Required for `set` and `reset`."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Whether external skills runtime/download is enabled."
                    },
                    "require_download_approval": {
                        "type": "boolean",
                        "description": "When true, every external skills download requires explicit approval_granted=true."
                    },
                    "allowed_domains": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional domain allowlist (supports exact domains and wildcard forms like *.example.com). Empty list means allow all domains unless blocked."
                    },
                    "blocked_domains": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional domain blocklist (supports exact domains and wildcard forms like *.example.com). Blocklist always takes precedence."
                    }
                },
                "required": [],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_fetch_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Resolve and download an external skill artifact from a direct URL, GitHub reference, skills.sh page, clawhub.ai page, or npm package with strict domain policy checks and explicit approval gating.",
            "parameters": {
                "type": "object",
                "properties": {
                    "reference": {
                        "type": "string",
                        "description": "Preferred external skill reference. Supports direct URLs, GitHub refs, skills.sh pages, clawhub.ai pages, and npm packages."
                    },
                    "url": {
                        "type": "string",
                        "description": "Backward-compatible alias for `reference` when passing a direct URL or ecosystem reference."
                    },
                    "approval_granted": {
                        "type": "boolean",
                        "description": "Explicit user authorization for this download. Required when require_download_approval=true."
                    },
                    "save_as": {
                        "type": "string",
                        "description": "Optional output filename (stored under configured file root / external-skills-downloads)."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20971520,
                        "description": "Maximum download size in bytes. Defaults to 5242880 and is capped at 20971520."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_resolve_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Normalize a direct URL, GitHub reference, skills.sh page, ClawHub page, or npm package into a source-aware external skill candidate.",
            "parameters": {
                "type": "object",
                "properties": {
                    "reference": {
                        "type": "string",
                        "description": "External skill reference to normalize."
                    }
                },
                "required": ["reference"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Search the resolved external-skills inventory for active and shadowed matches.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Task phrase, capability phrase, or skill name to rank against discovered skills."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Maximum number of ranked matches to return."
                    }
                },
                "required": ["query", "limit"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_recommend_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Recommend the best-fit resolved external skills for an operator goal.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Operator goal, task phrase, or workflow description."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Maximum number of ranked recommendations to return."
                    }
                },
                "required": ["query", "limit"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_source_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Search preferred external skill ecosystems and return normalized source-aware candidates ranked by source priority.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query or external skill reference."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Maximum number of normalized candidates to return."
                    },
                    "sources": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Optional source filter list. Supported values: skills_sh, clawhub, github, npm."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_inspect_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Read metadata and a short preview for a resolved external skill across managed, user, and project scopes.",
            "parameters": {
                "type": "object",
                "properties": {
                    "skill_id": {
                        "type": "string",
                        "description": "Resolved external skill identifier."
                    }
                },
                "required": ["skill_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_install_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Install a managed external skill from a local directory, local .tgz/.tar.gz/.zip archive, or a first-party bundled skill id.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to a local directory containing SKILL.md or a local .tgz/.tar.gz/.zip archive."
                    },
                    "bundled_skill_id": {
                        "type": "string",
                        "description": "Optional first-party bundled skill identifier, for example `browser-companion-preview`."
                    },
                    "skill_id": {
                        "type": "string",
                        "description": "Optional explicit managed skill id override."
                    },
                    "source_skill_id": {
                        "type": "string",
                        "description": "Optional source skill selector when the input archive or directory contains multiple SKILL.md roots."
                    },
                    "security_decision": {
                        "type": "string",
                        "enum": ["approve_once", "deny"],
                        "description": "Optional one-time security override after a risky install was scanned and returned needs_approval."
                    },
                    "replace": {
                        "type": "boolean",
                        "description": "Replace an existing installed skill with the same id. Defaults to false."
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_invoke_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Load a resolved external skill's SKILL.md instructions into the conversation loop across managed, user, and project scopes.",
            "parameters": {
                "type": "object",
                "properties": {
                    "skill_id": {
                        "type": "string",
                        "description": "Resolved external skill identifier."
                    }
                },
                "required": ["skill_id"],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_list_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "List resolved external skills available for invocation across managed, user, and project scopes.",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }
        }
    })
}

pub(super) fn external_skills_remove_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Remove an installed external skill from the managed runtime.",
            "parameters": {
                "type": "object",
                "properties": {
                    "skill_id": {
                        "type": "string",
                        "description": "Managed external skill identifier."
                    }
                },
                "required": ["skill_id"],
                "additionalProperties": false
            }
        }
    })
}
