use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Value, json};

use super::runtime_config::ToolRuntimeConfig;
use crate::config::ToolConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolExecutionKind {
    Core,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolAvailability {
    Runtime,
    Planned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolSchedulingClass {
    SerialOnly,
    ParallelSafe,
}

impl ToolSchedulingClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SerialOnly => "serial_only",
            Self::ParallelSafe => "parallel_safe",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
        "browser.companion.click" | "browser.companion.type" => ToolGovernanceProfile {
            scope: ToolGovernanceScope::Routine,
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

pub fn scheduling_class_for_tool_name(tool_name: &str) -> ToolSchedulingClass {
    match tool_name {
        "tool.search" | "file.read" | "memory_search" | "memory_get" | "web.fetch"
        | "web.search" | "sessions_list" => ToolSchedulingClass::ParallelSafe,
        _ => ToolSchedulingClass::SerialOnly,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolExposureClass {
    ProviderCore,
    Discoverable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolVisibilityGate {
    Always,
    Sessions,
    Messages,
    Delegate,
    Browser,
    BrowserCompanion,
    ExternalSkills,
    MemoryFileRoot,
    WebFetch,
    WebSearch,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub provider_name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub execution_kind: ToolExecutionKind,
    pub availability: ToolAvailability,
    pub exposure: ToolExposureClass,
    pub visibility_gate: ToolVisibilityGate,
    provider_definition_builder: fn(&ToolDescriptor) -> Value,
}

impl ToolDescriptor {
    pub fn matches_name(&self, raw: &str) -> bool {
        self.name == raw || self.provider_name == raw || self.aliases.contains(&raw)
    }

    pub fn provider_definition(&self) -> Value {
        (self.provider_definition_builder)(self)
    }

    pub fn argument_hint(&self) -> &'static str {
        tool_argument_hint(self.name)
    }

    pub fn parameter_types(&self) -> &'static [(&'static str, &'static str)] {
        tool_parameter_types(self.name)
    }

    pub fn required_fields(&self) -> &'static [&'static str] {
        tool_required_fields(self.name)
    }

    pub fn tags(&self) -> &'static [&'static str] {
        tool_tags(self.name)
    }

    pub fn is_provider_core(&self) -> bool {
        self.exposure == ToolExposureClass::ProviderCore
    }

    pub fn is_discoverable(&self) -> bool {
        self.exposure == ToolExposureClass::Discoverable
    }

    pub fn scheduling_class(&self) -> ToolSchedulingClass {
        scheduling_class_for_tool_name(self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ToolCatalogEntry {
    pub canonical_name: &'static str,
    pub provider_function_name: &'static str,
    pub summary: &'static str,
    pub argument_hint: &'static str,
    pub parameter_types: &'static [(&'static str, &'static str)],
    pub required_fields: &'static [&'static str],
    pub tags: &'static [&'static str],
    pub exposure: ToolExposureClass,
    pub execution_kind: ToolExecutionKind,
    pub availability: ToolAvailability,
    pub scheduling_class: ToolSchedulingClass,
}

impl ToolCatalogEntry {
    pub fn is_provider_core(&self) -> bool {
        self.exposure == ToolExposureClass::ProviderCore
    }

    pub fn is_discoverable(&self) -> bool {
        self.exposure == ToolExposureClass::Discoverable
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

    pub fn tool_names(&self) -> impl Iterator<Item = &str> {
        self.allowed_names.iter().map(String::as_str)
    }

    pub fn intersect(&self, other: &ToolView) -> ToolView {
        let names: BTreeSet<String> = self
            .allowed_names
            .intersection(&other.allowed_names)
            .cloned()
            .collect();
        ToolView {
            allowed_names: names,
        }
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
    any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    )
))]
const fn runtime_messaging_tool_availability() -> ToolAvailability {
    ToolAvailability::Runtime
}

#[cfg(not(all(
    feature = "memory-sqlite",
    any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    )
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
            name: "tool.search",
            provider_name: "tool_search",
            aliases: &[],
            description: "Discover non-core tools",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::ProviderCore,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: tool_search_definition,
        },
        ToolDescriptor {
            name: "tool.invoke",
            provider_name: "tool_invoke",
            aliases: &[],
            description: "Invoke a discovered non-core tool",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::ProviderCore,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: tool_invoke_definition,
        },
        ToolDescriptor {
            name: "claw.migrate",
            provider_name: "claw_migrate",
            aliases: &[],
            description: "Migrate legacy Claw configs into native LoongClaw settings",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: claw_migrate_definition,
        },
        ToolDescriptor {
            name: "external_skills.fetch",
            provider_name: "external_skills_fetch",
            aliases: &[],
            description: "Download external skills artifacts with domain policy and approval guards",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            provider_definition_builder: external_skills_fetch_definition,
        },
        ToolDescriptor {
            name: "external_skills.inspect",
            provider_name: "external_skills_inspect",
            aliases: &[],
            description: "Read metadata for a resolved external skill across managed, user, and project scopes",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            provider_definition_builder: external_skills_inspect_definition,
        },
        ToolDescriptor {
            name: "external_skills.install",
            provider_name: "external_skills_install",
            aliases: &[],
            description: "Install a managed external skill from a local directory or archive",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            provider_definition_builder: external_skills_install_definition,
        },
        ToolDescriptor {
            name: "external_skills.invoke",
            provider_name: "external_skills_invoke",
            aliases: &[],
            description: "Load a resolved external skill into the conversation loop",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            provider_definition_builder: external_skills_invoke_definition,
        },
        ToolDescriptor {
            name: "external_skills.list",
            provider_name: "external_skills_list",
            aliases: &[],
            description: "List resolved external skills across managed, user, and project scopes",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            provider_definition_builder: external_skills_list_definition,
        },
        ToolDescriptor {
            name: "external_skills.policy",
            provider_name: "external_skills_policy",
            aliases: &[],
            description: "Read/update external skills domain allow/block policy at runtime",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: external_skills_policy_definition,
        },
        ToolDescriptor {
            name: "external_skills.remove",
            provider_name: "external_skills_remove",
            aliases: &[],
            description: "Remove an installed external skill from the managed runtime",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            provider_definition_builder: external_skills_remove_definition,
        },
        ToolDescriptor {
            name: "provider.switch",
            provider_name: "provider_switch",
            aliases: &[],
            description: "Inspect current provider state or switch the default provider profile for subsequent turns",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: provider_switch_definition,
        },
        ToolDescriptor {
            name: "approval_request_resolve",
            provider_name: "approval_request_resolve",
            aliases: &[],
            description: "Resolve one visible governed tool approval request",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: approval_request_resolve_definition,
        },
        ToolDescriptor {
            name: "approval_request_status",
            provider_name: "approval_request_status",
            aliases: &[],
            description: "Inspect full detail for a visible governed tool approval request",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: approval_request_status_definition,
        },
        ToolDescriptor {
            name: "approval_requests_list",
            provider_name: "approval_requests_list",
            aliases: &[],
            description: "List visible governed tool approval requests across the current session scope",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: approval_requests_list_definition,
        },
        ToolDescriptor {
            name: "delegate",
            provider_name: "delegate",
            aliases: &[],
            description: "Delegate a focused subtask into a child session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Delegate,
            provider_definition_builder: delegate_definition,
        },
        ToolDescriptor {
            name: "delegate_async",
            provider_name: "delegate_async",
            aliases: &[],
            description: "Delegate a focused subtask into a background child session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Delegate,
            provider_definition_builder: delegate_async_definition,
        },
        ToolDescriptor {
            name: "session_archive",
            provider_name: "session_archive",
            aliases: &[],
            description: "Archive a visible terminal session from default session listings",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: session_archive_definition,
        },
        ToolDescriptor {
            name: "session_cancel",
            provider_name: "session_cancel",
            aliases: &[],
            description: "Cancel a visible async delegate child session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: session_cancel_definition,
        },
        ToolDescriptor {
            name: "session_events",
            provider_name: "session_events",
            aliases: &[],
            description: "Fetch session events for a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: session_events_definition,
        },
        ToolDescriptor {
            name: "session_recover",
            provider_name: "session_recover",
            aliases: &[],
            description: "Recover an overdue queued async delegate child session by marking it failed",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: session_recover_definition,
        },
        ToolDescriptor {
            name: "session_status",
            provider_name: "session_status",
            aliases: &[],
            description: "Inspect the current status of a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: session_status_definition,
        },
        ToolDescriptor {
            name: "session_wait",
            provider_name: "session_wait",
            aliases: &[],
            description: "Wait for a visible session to reach a terminal state",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: session_wait_definition,
        },
        ToolDescriptor {
            name: "sessions_history",
            provider_name: "sessions_history",
            aliases: &[],
            description: "Fetch transcript history for a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: sessions_history_definition,
        },
        ToolDescriptor {
            name: "sessions_list",
            provider_name: "sessions_list",
            aliases: &[],
            description: "List visible sessions and their high-level state",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            provider_definition_builder: sessions_list_definition,
        },
        ToolDescriptor {
            name: "sessions_send",
            provider_name: "sessions_send",
            aliases: &[],
            description: "Send an outbound text message to a known channel-backed root session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_messaging_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Messages,
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
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: file_read_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "memory_search",
            provider_name: "memory_search",
            aliases: &[],
            description: "Search durable workspace memory files with bounded citation-bearing snippets",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::MemoryFileRoot,
            provider_definition_builder: memory_search_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "memory_get",
            provider_name: "memory_get",
            aliases: &[],
            description: "Read a bounded line window from one durable workspace memory file",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::MemoryFileRoot,
            provider_definition_builder: memory_get_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "file.write",
            provider_name: "file_write",
            aliases: &[],
            description: "Write file contents",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: file_write_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "file.edit",
            provider_name: "file_edit",
            aliases: &[],
            description: "Replace text in a file",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: file_edit_definition,
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
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            provider_definition_builder: shell_exec_definition,
        });
    }

    #[cfg(feature = "tool-browser")]
    {
        descriptors.push(ToolDescriptor {
            name: "browser.click",
            provider_name: "browser_click",
            aliases: &["browser_click"],
            description: "Follow one previously discovered page link within a bounded browser session",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Browser,
            provider_definition_builder: browser_click_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.companion.click",
            provider_name: "browser_companion_click",
            aliases: &["browser_companion_click"],
            description: "Click a page element inside a governed browser companion session after policy review",
            execution_kind: ToolExecutionKind::App,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BrowserCompanion,
            provider_definition_builder: browser_companion_click_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.companion.navigate",
            provider_name: "browser_companion_navigate",
            aliases: &["browser_companion_navigate"],
            description: "Navigate a governed browser companion session to a target URL",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BrowserCompanion,
            provider_definition_builder: browser_companion_navigate_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.companion.session.start",
            provider_name: "browser_companion_session_start",
            aliases: &["browser_companion_session_start"],
            description: "Start a governed browser companion session at a target URL",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BrowserCompanion,
            provider_definition_builder: browser_companion_session_start_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.companion.session.stop",
            provider_name: "browser_companion_session_stop",
            aliases: &["browser_companion_session_stop"],
            description: "Stop a governed browser companion session and release companion-side state",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BrowserCompanion,
            provider_definition_builder: browser_companion_session_stop_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.companion.snapshot",
            provider_name: "browser_companion_snapshot",
            aliases: &["browser_companion_snapshot"],
            description: "Capture a readable snapshot of the current browser companion page",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BrowserCompanion,
            provider_definition_builder: browser_companion_snapshot_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.companion.type",
            provider_name: "browser_companion_type",
            aliases: &["browser_companion_type"],
            description: "Type text into a page element inside a governed browser companion session after policy review",
            execution_kind: ToolExecutionKind::App,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BrowserCompanion,
            provider_definition_builder: browser_companion_type_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.companion.wait",
            provider_name: "browser_companion_wait",
            aliases: &["browser_companion_wait"],
            description: "Wait inside a governed browser companion session for a condition or timeout window",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BrowserCompanion,
            provider_definition_builder: browser_companion_wait_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.extract",
            provider_name: "browser_extract",
            aliases: &["browser_extract"],
            description: "Extract structured text or links from the current browser session page",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Browser,
            provider_definition_builder: browser_extract_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "browser.open",
            provider_name: "browser_open",
            aliases: &["browser_open"],
            description:
                "Open a public web page into a bounded browser session with safe link discovery",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Browser,
            provider_definition_builder: browser_open_definition,
        });
    }

    #[cfg(feature = "tool-webfetch")]
    {
        descriptors.push(ToolDescriptor {
            name: "web.fetch",
            provider_name: "web_fetch",
            aliases: &["web_fetch"],
            description: "Fetch a public web page with SSRF-safe guards and readable extraction",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::WebFetch,
            provider_definition_builder: web_fetch_definition,
        });
    }

    #[cfg(feature = "tool-websearch")]
    {
        descriptors.push(ToolDescriptor {
            name: "web.search",
            provider_name: "web_search",
            aliases: &[],
            description:
                "Search the web for APIs, documentation, and error messages using configured web search providers",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::WebSearch,
            provider_definition_builder: web_search_definition,
        });
    }

    descriptors.sort_by(|left, right| left.name.cmp(right.name));
    ToolCatalog { descriptors }
}

pub fn runtime_tool_view() -> ToolView {
    runtime_tool_view_for_runtime_config(super::runtime_config::get_tool_runtime_config())
}

pub fn runtime_tool_view_for_config(config: &ToolConfig) -> ToolView {
    runtime_tool_view_for_config_with_external_skills(config, false)
}

pub fn runtime_tool_view_for_config_with_external_skills(
    config: &ToolConfig,
    external_skills_enabled: bool,
) -> ToolView {
    let catalog = tool_catalog();
    ToolView::from_tool_names(
        catalog
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.availability == ToolAvailability::Runtime)
            .filter(|descriptor| {
                tool_visibility_gate_enabled_for_runtime_view(
                    descriptor.visibility_gate,
                    config,
                    external_skills_enabled,
                )
            })
            .map(|descriptor| descriptor.name),
    )
}

pub fn runtime_tool_view_for_runtime_config(config: &ToolRuntimeConfig) -> ToolView {
    let catalog = tool_catalog();
    ToolView::from_tool_names(
        catalog
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.availability == ToolAvailability::Runtime)
            .filter(|descriptor| {
                tool_visibility_gate_enabled_for_runtime_policy(descriptor.visibility_gate, config)
            })
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
            name if allowlist.contains(name)
                && tool_visibility_gate_enabled_for_runtime_view(
                    descriptor.visibility_gate,
                    config,
                    false,
                ) =>
            {
                names.push(name);
            }
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

pub fn provider_core_tool_catalog() -> Vec<ToolCatalogEntry> {
    tool_catalog()
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.is_provider_core())
        .map(descriptor_to_entry)
        .collect()
}

pub fn discoverable_tool_catalog() -> Vec<ToolCatalogEntry> {
    tool_catalog()
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.is_discoverable())
        .map(descriptor_to_entry)
        .collect()
}

pub fn all_tool_catalog() -> Vec<ToolCatalogEntry> {
    tool_catalog()
        .descriptors()
        .iter()
        .map(descriptor_to_entry)
        .collect()
}

pub fn find_tool_catalog_entry(name: &str) -> Option<ToolCatalogEntry> {
    tool_catalog().resolve(name).map(descriptor_to_entry)
}

fn descriptor_to_entry(descriptor: &ToolDescriptor) -> ToolCatalogEntry {
    ToolCatalogEntry {
        canonical_name: descriptor.name,
        provider_function_name: descriptor.provider_name,
        summary: descriptor.description,
        argument_hint: descriptor.argument_hint(),
        parameter_types: descriptor.parameter_types(),
        required_fields: descriptor.required_fields(),
        tags: descriptor.tags(),
        exposure: descriptor.exposure,
        execution_kind: descriptor.execution_kind,
        availability: descriptor.availability,
        scheduling_class: descriptor.scheduling_class(),
    }
}

fn tool_visibility_gate_enabled_for_runtime_view(
    gate: ToolVisibilityGate,
    config: &ToolConfig,
    external_skills_enabled: bool,
) -> bool {
    match gate {
        ToolVisibilityGate::Always => true,
        ToolVisibilityGate::Sessions => config.sessions.enabled,
        ToolVisibilityGate::Messages => config.messages.enabled,
        ToolVisibilityGate::Delegate => config.delegate.enabled,
        ToolVisibilityGate::Browser => config.browser.enabled,
        ToolVisibilityGate::BrowserCompanion => false,
        ToolVisibilityGate::ExternalSkills => external_skills_enabled,
        ToolVisibilityGate::MemoryFileRoot => config
            .file_root
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        ToolVisibilityGate::WebFetch => config.web.enabled,
        ToolVisibilityGate::WebSearch => config.web_search.enabled,
    }
}

fn tool_visibility_gate_enabled_for_runtime_policy(
    gate: ToolVisibilityGate,
    config: &ToolRuntimeConfig,
) -> bool {
    match gate {
        ToolVisibilityGate::Always => true,
        ToolVisibilityGate::Sessions => config.sessions_enabled,
        ToolVisibilityGate::Messages => config.messages_enabled,
        ToolVisibilityGate::Delegate => config.delegate_enabled,
        ToolVisibilityGate::Browser => config.browser.enabled,
        ToolVisibilityGate::BrowserCompanion => config.browser_companion.is_runtime_ready(),
        ToolVisibilityGate::ExternalSkills => config.external_skills.enabled,
        ToolVisibilityGate::MemoryFileRoot => {
            let has_file_root = config
                .file_root
                .as_ref()
                .is_some_and(|value| !value.as_os_str().is_empty());

            if !has_file_root {
                return false;
            }

            #[cfg(feature = "tool-file")]
            {
                let memory_corpus_available = super::memory_tools::memory_corpus_available(config);
                memory_corpus_available
            }

            #[cfg(not(feature = "tool-file"))]
            {
                has_file_root
            }
        }
        ToolVisibilityGate::WebFetch => config.web_fetch.enabled,
        ToolVisibilityGate::WebSearch => config.web_search.enabled,
    }
}

fn tool_search_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Discover non-core tools relevant to the current task.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "English natural-language description of the tool capability you need."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Optional maximum number of search results to return."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_open_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "HTTP or HTTPS URL to open."
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

fn browser_extract_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Bounded browser session identifier returned by browser.open."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["page_text", "title", "links", "selector_text"],
                        "description": "Extraction mode. Defaults to `page_text`."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Optional CSS selector used only with `selector_text` mode."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "Maximum extracted items when the mode returns a list."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_click_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Bounded browser session identifier returned by browser.open."
                    },
                    "link_id": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": crate::config::MAX_BROWSER_MAX_LINKS,
                        "description": "One-based link identifier returned in the current page snapshot."
                    }
                },
                "required": ["session_id", "link_id"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_companion_session_start_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "HTTP or HTTPS URL to open in the managed browser companion session."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_companion_navigate_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "url": {
                        "type": "string",
                        "description": "HTTP or HTTPS URL to load next."
                    }
                },
                "required": ["session_id", "url"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_companion_snapshot_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["summary", "html", "links"],
                        "description": "Optional snapshot mode. Defaults to `summary`."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_companion_wait_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "condition": {
                        "type": "string",
                        "description": "Optional companion-side wait condition."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 30000,
                        "description": "Optional maximum wait in milliseconds."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_companion_session_stop_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_companion_click_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Selector for the element to click."
                    }
                },
                "required": ["session_id", "selector"],
                "additionalProperties": false
            }
        }
    })
}

fn browser_companion_type_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Browser companion session identifier returned by browser.companion.session.start."
                    },
                    "selector": {
                        "type": "string",
                        "description": "Selector for the element to type into."
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to enter."
                    }
                },
                "required": ["session_id", "selector", "text"],
                "additionalProperties": false
            }
        }
    })
}

fn tool_invoke_definition(descriptor: &ToolDescriptor) -> Value {
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

fn claw_migrate_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Path to the legacy Claw workspace, config root, or portable migration file. Required for all modes except rollback_last_apply."
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

fn external_skills_install_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Install a managed external skill from a local directory, local .tgz/.tar.gz archive, or a first-party bundled skill id.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to a local directory containing SKILL.md or a local .tgz/.tar.gz archive."
                    },
                    "bundled_skill_id": {
                        "type": "string",
                        "description": "Optional first-party bundled skill identifier, for example `browser-companion-preview`."
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
                "anyOf": [
                    { "required": ["path"] },
                    { "required": ["bundled_skill_id"] }
                ],
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

fn external_skills_list_definition(descriptor: &ToolDescriptor) -> Value {
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
                        "description": "Natural-language lookup query for durable workspace memory."
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

fn memory_get_definition(descriptor: &ToolDescriptor) -> Value {
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

fn file_edit_definition(descriptor: &ToolDescriptor) -> Value {
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
                    "old_string": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Literal substring to find. Must be non-empty. \
                                        Must match exactly once unless replace_all is true."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement text."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace all occurrences instead of requiring a unique match. \
                                        Zero-match still fails regardless of this flag. Defaults to false."
                    }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }
        }
    })
}

#[cfg(feature = "tool-webfetch")]
fn web_fetch_definition(descriptor: &ToolDescriptor) -> Value {
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
fn web_search_definition(descriptor: &ToolDescriptor) -> Value {
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

fn tool_argument_hint(name: &str) -> &'static str {
    match name {
        "tool.search" => "query:string,limit?:integer",
        "tool.invoke" => "tool_id:string,lease:string,arguments:object",
        "claw.migrate" => "input_path?:string,mode?:string,source?:string",
        "external_skills.fetch" => {
            "url:string,approval_granted?:boolean,save_as?:string,max_bytes?:integer"
        }
        "external_skills.inspect" => "skill_id:string",
        "external_skills.install" => {
            "path?:string,bundled_skill_id?:string,skill_id?:string,replace?:boolean"
        }
        "external_skills.invoke" => "skill_id:string",
        "external_skills.list" => "",
        "external_skills.policy" => {
            "action?:string,enabled?:boolean,allowed_domains?:string[],blocked_domains?:string[]"
        }
        "external_skills.remove" => "skill_id:string",
        "browser.companion.session.start" => "url:string",
        "browser.companion.navigate" => "session_id:string,url:string",
        "browser.companion.snapshot" => "session_id:string,mode?:string",
        "browser.companion.wait" => "session_id:string,condition?:string,timeout_ms?:integer",
        "browser.companion.session.stop" => "session_id:string",
        "browser.companion.click" => "session_id:string,selector:string",
        "browser.companion.type" => "session_id:string,selector:string,text:string",
        "file.read" => "path:string,max_bytes?:integer",
        "memory_search" => "query:string,max_results?:integer",
        "memory_get" => "path:string,from?:integer,lines?:integer",
        "file.write" => "path:string,content:string,create_dirs?:boolean",
        "file.edit" => "path:string,old_string:string,new_string:string,replace_all?:boolean",
        "shell.exec" => "command:string,args?:string[],timeout_ms?:integer,cwd?:string",
        "provider.switch" => "selector?:string",
        "delegate" | "delegate_async" => "task:string,label?:string,timeout_seconds?:integer",
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => "session_id:string",
        "sessions_list" => "limit?:integer,state?:string",
        "sessions_send" => "session_id:string,text:string",
        "web.search" => "query:string,provider?:string,max_results?:integer",
        _ => "",
    }
}

fn tool_parameter_types(name: &str) -> &'static [(&'static str, &'static str)] {
    match name {
        "tool.search" => &[("query", "string"), ("limit", "integer")],
        "tool.invoke" => &[
            ("tool_id", "string"),
            ("lease", "string"),
            ("arguments", "object"),
        ],
        "claw.migrate" => &[
            ("input_path", "string"),
            ("mode", "string"),
            ("source", "string"),
        ],
        "external_skills.fetch" => &[
            ("url", "string"),
            ("approval_granted", "boolean"),
            ("save_as", "string"),
            ("max_bytes", "integer"),
        ],
        "external_skills.inspect" | "external_skills.invoke" | "external_skills.remove" => {
            &[("skill_id", "string")]
        }
        "external_skills.install" => &[
            ("path", "string"),
            ("bundled_skill_id", "string"),
            ("skill_id", "string"),
            ("replace", "boolean"),
        ],
        "external_skills.list" => &[],
        "browser.companion.session.start" => &[("url", "string")],
        "browser.companion.navigate" => &[("session_id", "string"), ("url", "string")],
        "browser.companion.snapshot" => &[("session_id", "string"), ("mode", "string")],
        "browser.companion.wait" => &[
            ("session_id", "string"),
            ("condition", "string"),
            ("timeout_ms", "integer"),
        ],
        "browser.companion.session.stop" => &[("session_id", "string")],
        "browser.companion.click" => &[("session_id", "string"), ("selector", "string")],
        "browser.companion.type" => &[
            ("session_id", "string"),
            ("selector", "string"),
            ("text", "string"),
        ],
        "external_skills.policy" => &[
            ("action", "string"),
            ("enabled", "boolean"),
            ("allowed_domains", "array"),
            ("blocked_domains", "array"),
        ],
        "file.read" => &[("path", "string"), ("max_bytes", "integer")],
        "memory_search" => &[("query", "string"), ("max_results", "integer")],
        "memory_get" => &[
            ("path", "string"),
            ("from", "integer"),
            ("lines", "integer"),
        ],
        "file.write" => &[
            ("path", "string"),
            ("content", "string"),
            ("create_dirs", "boolean"),
        ],
        "file.edit" => &[
            ("path", "string"),
            ("old_string", "string"),
            ("new_string", "string"),
            ("replace_all", "boolean"),
        ],
        "shell.exec" => &[
            ("command", "string"),
            ("args", "array"),
            ("timeout_ms", "integer"),
            ("cwd", "string"),
        ],
        "provider.switch" => &[("selector", "string")],
        "delegate" | "delegate_async" => &[
            ("task", "string"),
            ("label", "string"),
            ("timeout_seconds", "integer"),
        ],
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => &[("session_id", "string")],
        "sessions_list" => &[("limit", "integer"), ("state", "string")],
        "sessions_send" => &[("session_id", "string"), ("text", "string")],
        "web.search" => &[
            ("query", "string"),
            ("provider", "string"),
            ("max_results", "integer"),
        ],
        _ => &[],
    }
}

const EMPTY_REQUIRED_FIELD_GROUPS: &[&[&str]] = &[];
const EXTERNAL_SKILLS_INSTALL_REQUIRED_FIELD_GROUPS: &[&[&str]] =
    &[&["path"], &["bundled_skill_id"]];

fn tool_required_fields(name: &str) -> &'static [&'static str] {
    match name {
        "tool.search" => &["query"],
        "tool.invoke" => &["tool_id", "lease", "arguments"],
        "external_skills.fetch" => &["url"],
        "external_skills.inspect" | "external_skills.invoke" | "external_skills.remove" => {
            &["skill_id"]
        }
        // Grouped requirements are the source of truth for this tool's anyOf shape.
        "external_skills.install" => &[],
        "browser.companion.session.start" => &["url"],
        "browser.companion.navigate" => &["session_id", "url"],
        "browser.companion.snapshot"
        | "browser.companion.wait"
        | "browser.companion.session.stop" => &["session_id"],
        "browser.companion.click" => &["session_id", "selector"],
        "browser.companion.type" => &["session_id", "selector", "text"],
        "file.read" => &["path"],
        "memory_search" => &["query"],
        "memory_get" => &["path"],
        "file.write" => &["path", "content"],
        "file.edit" => &["path", "old_string", "new_string"],
        "shell.exec" => &["command"],
        "delegate" | "delegate_async" => &["task"],
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => &["session_id"],
        "sessions_send" => &["session_id", "text"],
        "web.search" => &["query"],
        _ => &[],
    }
}

pub(crate) fn tool_required_field_groups(name: &str) -> &'static [&'static [&'static str]] {
    match name {
        "external_skills.install" => EXTERNAL_SKILLS_INSTALL_REQUIRED_FIELD_GROUPS,
        _ => EMPTY_REQUIRED_FIELD_GROUPS,
    }
}

fn tool_tags(name: &str) -> &'static [&'static str] {
    match name {
        "tool.search" => &["core", "discover", "search"],
        "tool.invoke" => &["core", "dispatch", "invoke"],
        "claw.migrate" => &["migration", "migrate", "config", "legacy"],
        "external_skills.fetch" => &["skills", "download", "external", "fetch"],
        "external_skills.inspect" => &["skills", "inspect", "metadata"],
        "external_skills.install" => &["skills", "install", "package"],
        "external_skills.invoke" => &["skills", "invoke", "instructions"],
        "external_skills.list" => &["skills", "list", "discover"],
        "external_skills.policy" => &["skills", "policy", "security"],
        "external_skills.remove" => &["skills", "remove", "uninstall"],
        "browser.companion.session.start"
        | "browser.companion.navigate"
        | "browser.companion.snapshot"
        | "browser.companion.wait"
        | "browser.companion.session.stop" => &["browser", "companion", "session", "read"],
        "browser.companion.click" | "browser.companion.type" => {
            &["browser", "companion", "write", "approval"]
        }
        "file.read" => &["file", "read", "filesystem", "repo"],
        "memory_search" => &["memory", "search", "recall", "durable", "workspace"],
        "memory_get" => &["memory", "read", "recall", "durable", "workspace"],
        "file.write" => &["file", "write", "filesystem"],
        "file.edit" => &["file", "edit", "filesystem"],
        "shell.exec" => &["shell", "command", "process", "exec"],
        "provider.switch" => &["provider", "switch", "model", "runtime"],
        "delegate" | "delegate_async" => &["session", "delegate", "child"],
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" | "sessions_list" => {
            &["session", "history", "runtime"]
        }
        "sessions_send" => &["session", "message", "channel"],
        "web.search" => &["web", "search", "discover", "external"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(feature = "tool-browser")]
    #[test]
    fn browser_companion_visibility_surface_requires_runtime_readiness_for_all_companion_tools() {
        let catalog = tool_catalog();
        let expected = [
            ("browser.companion.session.start", ToolExecutionKind::Core),
            ("browser.companion.navigate", ToolExecutionKind::Core),
            ("browser.companion.snapshot", ToolExecutionKind::Core),
            ("browser.companion.wait", ToolExecutionKind::Core),
            ("browser.companion.session.stop", ToolExecutionKind::Core),
            ("browser.companion.click", ToolExecutionKind::App),
            ("browser.companion.type", ToolExecutionKind::App),
        ];

        let mut hidden = ToolRuntimeConfig::default();
        hidden.browser_companion.enabled = true;
        hidden.browser_companion.ready = false;
        hidden.browser_companion.command = Some("browser-companion".to_owned());
        let hidden_view = runtime_tool_view_for_runtime_config(&hidden);

        let mut visible = ToolRuntimeConfig::default();
        visible.browser_companion.enabled = true;
        visible.browser_companion.ready = true;
        visible.browser_companion.command = Some("browser-companion".to_owned());
        let visible_view = runtime_tool_view_for_runtime_config(&visible);

        for (tool_name, execution_kind) in expected {
            let descriptor = catalog
                .resolve(tool_name)
                .unwrap_or_else(|| panic!("missing browser companion descriptor `{tool_name}`"));
            assert_eq!(
                descriptor.visibility_gate,
                ToolVisibilityGate::BrowserCompanion
            );
            assert_eq!(descriptor.execution_kind, execution_kind);
            assert!(
                !hidden_view.contains(tool_name),
                "tool should stay hidden until runtime-ready: {tool_name}"
            );
            assert!(
                visible_view.contains(tool_name),
                "tool should appear once runtime-ready: {tool_name}"
            );
        }
    }

    #[test]
    fn browser_companion_visibility_gate_requires_runtime_readiness() {
        let mut config = ToolRuntimeConfig::default();
        config.browser_companion.enabled = true;
        config.browser_companion.ready = false;
        config.browser_companion.command = Some("browser-companion".to_owned());

        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::BrowserCompanion,
            &config
        ));

        config.browser_companion.ready = true;

        assert!(tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::BrowserCompanion,
            &config
        ));
    }

    #[test]
    fn browser_companion_visibility_gate_stays_hidden_for_config_only_views() {
        let mut config = ToolConfig::default();
        config.browser_companion.enabled = true;

        assert!(!tool_visibility_gate_enabled_for_runtime_view(
            ToolVisibilityGate::BrowserCompanion,
            &config,
            false
        ));
    }

    #[test]
    fn memory_file_root_visibility_gate_requires_safe_root_configuration() {
        let hidden_config = ToolConfig::default();
        assert!(!tool_visibility_gate_enabled_for_runtime_view(
            ToolVisibilityGate::MemoryFileRoot,
            &hidden_config,
            false
        ));

        let visible_config = ToolConfig {
            file_root: Some("/tmp/workspace".to_owned()),
            ..ToolConfig::default()
        };
        assert!(tool_visibility_gate_enabled_for_runtime_view(
            ToolVisibilityGate::MemoryFileRoot,
            &visible_config,
            false
        ));

        let hidden_runtime = ToolRuntimeConfig::default();
        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::MemoryFileRoot,
            &hidden_runtime
        ));

        let empty_runtime_dir = tempdir().expect("tempdir");
        let empty_runtime = ToolRuntimeConfig {
            file_root: Some(empty_runtime_dir.path().to_path_buf()),
            ..ToolRuntimeConfig::default()
        };
        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::MemoryFileRoot,
            &empty_runtime
        ));

        let visible_runtime_dir = tempdir().expect("tempdir");
        let visible_memory_path = visible_runtime_dir.path().join("MEMORY.md");
        std::fs::write(
            &visible_memory_path,
            "# Durable Notes\nDeploy freeze window is Friday.\n",
        )
        .expect("write root memory");

        let visible_runtime = ToolRuntimeConfig {
            file_root: Some(visible_runtime_dir.path().to_path_buf()),
            ..ToolRuntimeConfig::default()
        };
        assert!(tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::MemoryFileRoot,
            &visible_runtime
        ));
    }

    #[test]
    fn browser_visibility_gate_is_independent_from_companion_settings() {
        let mut config = ToolRuntimeConfig::default();
        config.browser.enabled = true;
        config.browser_companion.enabled = false;
        config.browser_companion.ready = false;

        assert!(tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::Browser,
            &config
        ));
    }

    #[test]
    fn delegate_child_tool_view_respects_visibility_gates() {
        let mut config = ToolConfig::default();
        config.web.enabled = false;
        config.delegate.child_tool_allowlist = vec!["web.fetch".to_owned()];

        let child_view = delegate_child_tool_view_for_config(&config);

        assert!(!child_view.contains("web.fetch"));
    }

    #[test]
    fn scheduling_class_marks_parallel_safe_subset() {
        let catalog = tool_catalog();
        assert_eq!(
            catalog
                .descriptor("tool.search")
                .expect("tool.search descriptor")
                .scheduling_class(),
            ToolSchedulingClass::ParallelSafe
        );
        #[cfg(feature = "tool-file")]
        assert_eq!(
            catalog
                .descriptor("file.read")
                .expect("file.read descriptor")
                .scheduling_class(),
            ToolSchedulingClass::ParallelSafe
        );
        #[cfg(feature = "tool-file")]
        assert_eq!(
            catalog
                .descriptor("memory_search")
                .expect("memory_search descriptor")
                .scheduling_class(),
            ToolSchedulingClass::ParallelSafe
        );
        #[cfg(feature = "tool-file")]
        assert_eq!(
            catalog
                .descriptor("memory_get")
                .expect("memory_get descriptor")
                .scheduling_class(),
            ToolSchedulingClass::ParallelSafe
        );
        #[cfg(feature = "tool-webfetch")]
        assert_eq!(
            catalog
                .descriptor("web.fetch")
                .expect("web.fetch descriptor")
                .scheduling_class(),
            ToolSchedulingClass::ParallelSafe
        );
        assert_eq!(
            catalog
                .descriptor("sessions_list")
                .expect("sessions_list descriptor")
                .scheduling_class(),
            ToolSchedulingClass::ParallelSafe
        );
        assert_eq!(
            catalog
                .descriptor("delegate_async")
                .expect("delegate_async descriptor")
                .scheduling_class(),
            ToolSchedulingClass::SerialOnly
        );
    }

    #[test]
    fn sessions_send_definition_mentions_generic_channel_sessions() {
        let catalog = tool_catalog();
        let descriptor = catalog
            .descriptor("sessions_send")
            .expect("sessions_send descriptor");
        let definition = descriptor.provider_definition();
        let description =
            definition["function"]["parameters"]["properties"]["session_id"]["description"]
                .as_str()
                .expect("session_id description");

        assert!(description.contains("channel-backed"));
        assert!(description.contains("Matrix"));
    }
}
