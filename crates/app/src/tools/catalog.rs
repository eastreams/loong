use std::collections::BTreeSet;

use serde_json::{Value, json};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolGovernanceScope {
    Routine,
    TopologyMutation,
}

impl ToolGovernanceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Routine => "routine",
            Self::TopologyMutation => "topology_mutation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRiskClass {
    Low,
    Elevated,
    High,
}

impl ToolRiskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Elevated => "elevated",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolApprovalMode {
    Never,
    PolicyDriven,
}

impl ToolApprovalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::PolicyDriven => "policy_driven",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolGovernanceProfile {
    pub scope: ToolGovernanceScope,
    pub risk_class: ToolRiskClass,
    pub approval_mode: ToolApprovalMode,
}

pub fn governance_profile_for_tool_name(tool_name: &str) -> ToolGovernanceProfile {
    match tool_name {
        "delegate" | "delegate_async" => ToolGovernanceProfile {
            scope: ToolGovernanceScope::TopologyMutation,
            risk_class: ToolRiskClass::High,
            approval_mode: ToolApprovalMode::PolicyDriven,
        },
        "session_archive" | "session_cancel" | "session_recover" | "sessions_send" => {
            ToolGovernanceProfile {
                scope: ToolGovernanceScope::Routine,
                risk_class: ToolRiskClass::Elevated,
                approval_mode: ToolApprovalMode::PolicyDriven,
            }
        }
        _ => ToolGovernanceProfile {
            scope: ToolGovernanceScope::Routine,
            risk_class: ToolRiskClass::Low,
            approval_mode: ToolApprovalMode::Never,
        },
    }
}

pub fn governance_profile_for_descriptor(descriptor: &ToolDescriptor) -> ToolGovernanceProfile {
    governance_profile_for_tool_name(descriptor.name)
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
        self.name == raw || self.provider_name == raw || self.aliases.contains(&raw)
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

#[cfg(feature = "memory-sqlite")]
const fn runtime_session_tool_availability() -> ToolAvailability {
    ToolAvailability::Runtime
}

#[cfg(not(feature = "memory-sqlite"))]
const fn runtime_session_tool_availability() -> ToolAvailability {
    ToolAvailability::Planned
}

#[cfg(all(
    feature = "memory-sqlite",
    any(feature = "channel-telegram", feature = "channel-feishu")
))]
const fn runtime_messaging_tool_availability() -> ToolAvailability {
    ToolAvailability::Runtime
}

#[cfg(not(all(
    feature = "memory-sqlite",
    any(feature = "channel-telegram", feature = "channel-feishu")
)))]
const fn runtime_messaging_tool_availability() -> ToolAvailability {
    ToolAvailability::Planned
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
    let mut descriptors = vec![
        ToolDescriptor {
            name: "claw.import",
            provider_name: "claw_import",
            aliases: &["import_claw"],
            description: "Import legacy Claw configs into native LoongClaw settings",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: claw_import_definition,
        },
        ToolDescriptor {
            name: "external_skills.fetch",
            provider_name: "external_skills_fetch",
            aliases: &[],
            description: "Download external skills artifacts with domain policy and approval guards",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: external_skills_fetch_definition,
        },
        ToolDescriptor {
            name: "external_skills.inspect",
            provider_name: "external_skills_inspect",
            aliases: &[],
            description: "Read metadata for an installed external skill",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: external_skills_inspect_definition,
        },
        ToolDescriptor {
            name: "external_skills.install",
            provider_name: "external_skills_install",
            aliases: &[],
            description: "Install a managed external skill from a local directory or archive",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: external_skills_install_definition,
        },
        ToolDescriptor {
            name: "external_skills.invoke",
            provider_name: "external_skills_invoke",
            aliases: &[],
            description: "Load an installed external skill into the conversation loop",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: external_skills_invoke_definition,
        },
        ToolDescriptor {
            name: "external_skills.list",
            provider_name: "external_skills_list",
            aliases: &[],
            description: "List managed external skills available for invocation",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: external_skills_list_definition,
        },
        ToolDescriptor {
            name: "external_skills.policy",
            provider_name: "external_skills_policy",
            aliases: &[],
            description: "Read/update external skills domain allow/block policy at runtime",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: external_skills_policy_definition,
        },
        ToolDescriptor {
            name: "external_skills.remove",
            provider_name: "external_skills_remove",
            aliases: &[],
            description: "Remove an installed external skill from the managed runtime",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: external_skills_remove_definition,
        },
        ToolDescriptor {
            name: "provider.switch",
            provider_name: "provider_switch",
            aliases: &[],
            description: "Inspect current provider state or switch the default provider profile for subsequent turns",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            provider_definition_builder: provider_switch_definition,
        },
        ToolDescriptor {
            name: "approval_request_resolve",
            provider_name: "approval_request_resolve",
            aliases: &[],
            description: "Resolve one visible governed tool approval request",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: approval_request_resolve_definition,
        },
        ToolDescriptor {
            name: "approval_request_status",
            provider_name: "approval_request_status",
            aliases: &[],
            description: "Inspect full detail for a visible governed tool approval request",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: approval_request_status_definition,
        },
        ToolDescriptor {
            name: "approval_requests_list",
            provider_name: "approval_requests_list",
            aliases: &[],
            description: "List visible governed tool approval requests across the current session scope",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: approval_requests_list_definition,
        },
        ToolDescriptor {
            name: "delegate",
            provider_name: "delegate",
            aliases: &[],
            description: "Delegate a focused subtask into a child session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: delegate_definition,
        },
        ToolDescriptor {
            name: "delegate_async",
            provider_name: "delegate_async",
            aliases: &[],
            description: "Delegate a focused subtask into a background child session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: delegate_async_definition,
        },
        ToolDescriptor {
            name: "session_archive",
            provider_name: "session_archive",
            aliases: &[],
            description: "Archive a visible terminal session from default session listings",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: session_archive_definition,
        },
        ToolDescriptor {
            name: "session_cancel",
            provider_name: "session_cancel",
            aliases: &[],
            description: "Cancel a visible async delegate child session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: session_cancel_definition,
        },
        ToolDescriptor {
            name: "session_events",
            provider_name: "session_events",
            aliases: &[],
            description: "Fetch session events for a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: session_events_definition,
        },
        ToolDescriptor {
            name: "session_recover",
            provider_name: "session_recover",
            aliases: &[],
            description: "Recover an overdue queued async delegate child session by marking it failed",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: session_recover_definition,
        },
        ToolDescriptor {
            name: "session_status",
            provider_name: "session_status",
            aliases: &[],
            description: "Inspect the current status of a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: session_status_definition,
        },
        ToolDescriptor {
            name: "session_wait",
            provider_name: "session_wait",
            aliases: &[],
            description: "Wait for a visible session to reach a terminal state",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: session_wait_definition,
        },
        ToolDescriptor {
            name: "sessions_history",
            provider_name: "sessions_history",
            aliases: &[],
            description: "Fetch transcript history for a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: sessions_history_definition,
        },
        ToolDescriptor {
            name: "sessions_list",
            provider_name: "sessions_list",
            aliases: &[],
            description: "List visible sessions and their high-level state",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            provider_definition_builder: sessions_list_definition,
        },
        ToolDescriptor {
            name: "sessions_send",
            provider_name: "sessions_send",
            aliases: &[],
            description: "Send an outbound text message to a known channel-backed root session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_messaging_tool_availability(),
            provider_definition_builder: sessions_send_definition,
        },
    ];

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
    let mut names = Vec::new();
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

    if allow_delegate
        && config.delegate.enabled
        && catalog
            .descriptor("delegate")
            .is_some_and(|descriptor| descriptor.availability == ToolAvailability::Runtime)
    {
        names.push("delegate");
    }
    if allow_delegate
        && config.delegate.enabled
        && catalog
            .descriptor("delegate_async")
            .is_some_and(|descriptor| descriptor.availability == ToolAvailability::Runtime)
    {
        names.push("delegate_async");
    }

    ToolView::from_tool_names(names)
}

fn tool_is_enabled_for_runtime_view(tool_name: &str, config: &ToolConfig) -> bool {
    match tool_name {
        "approval_request_resolve"
        | "approval_request_status"
        | "approval_requests_list"
        | "sessions_list"
        | "sessions_history"
        | "session_status"
        | "session_events"
        | "session_archive"
        | "session_cancel"
        | "session_recover"
        | "session_wait" => config.sessions.enabled,
        "sessions_send" => config.messages.enabled,
        "delegate" | "delegate_async" => config.delegate.enabled,
        _ => true,
    }
}

fn claw_import_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Import, discover, plan, merge, apply, and rollback legacy Claw workspace migration into native LoongClaw config.",
            "parameters": {
                "type": "object",
                "properties": {
                    "input_path": {
                        "type": "string",
                        "description": "Path to the legacy Claw workspace, config root, or portable import file. Required for all modes except rollback_last_apply."
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
                        "description": "Optional source hint for plan/apply modes. Defaults to automatic detection."
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
                        "description": "Target config path. Required in apply/apply_selected/rollback_last_apply modes."
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

fn provider_switch_definition(descriptor: &ToolDescriptor) -> Value {
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

fn external_skills_policy_definition(descriptor: &ToolDescriptor) -> Value {
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

fn external_skills_fetch_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Download an external skill artifact with strict domain policy checks and explicit approval gating.",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "HTTPS URL to download."
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
                "required": ["url"],
                "additionalProperties": false
            }
        }
    })
}

fn external_skills_inspect_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Read metadata and a short preview for an installed external skill.",
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

fn external_skills_install_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Install a managed external skill from a local directory or local .tgz/.tar.gz archive under the configured file root.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to a local directory containing SKILL.md or a local .tgz/.tar.gz archive."
                    },
                    "skill_id": {
                        "type": "string",
                        "description": "Optional explicit managed skill id override."
                    },
                    "replace": {
                        "type": "boolean",
                        "description": "Replace an existing installed skill with the same id. Defaults to false."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }
    })
}

fn external_skills_invoke_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Load an installed external skill's SKILL.md instructions into the conversation loop.",
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

fn external_skills_list_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "List managed external skills available for invocation.",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }
        }
    })
}

fn external_skills_remove_definition(descriptor: &ToolDescriptor) -> Value {
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

fn approval_request_resolve_definition(descriptor: &ToolDescriptor) -> Value {
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
                    }
                },
                "required": ["approval_request_id", "decision"],
                "additionalProperties": false
            }
        }
    })
}

fn approval_request_status_definition(descriptor: &ToolDescriptor) -> Value {
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

fn approval_requests_list_definition(descriptor: &ToolDescriptor) -> Value {
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
