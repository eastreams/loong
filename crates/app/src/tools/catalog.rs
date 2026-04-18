use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(not(feature = "tool-file"))]
use std::path::Path;
use std::sync::OnceLock;

use loong_kernel::ToolConcurrencyClass;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;

use super::runtime_config::ToolRuntimeConfig;
use crate::config::ToolConfig;
use crate::conversation::ConstrainedSubagentContractView;
#[cfg(test)]
use crate::conversation::ConstrainedSubagentProfile;

#[path = "catalog_metadata_support.rs"]
mod metadata_support;
use metadata_support::{
    tool_argument_hint, tool_parameter_types, tool_required_fields, tool_search_hint, tool_tags,
};
#[path = "catalog_core_definition_support.rs"]
mod core_definition_support;
use core_definition_support::{
    direct_browser_definition, direct_exec_definition, direct_memory_definition,
    direct_read_definition, direct_web_definition, direct_write_definition, tool_invoke_definition,
    tool_search_definition,
};
#[path = "catalog_browser_definition_support.rs"]
mod browser_definition_support;
use browser_definition_support::{
    browser_click_definition, browser_companion_click_definition,
    browser_companion_navigate_definition, browser_companion_session_start_definition,
    browser_companion_session_stop_definition, browser_companion_snapshot_definition,
    browser_companion_type_definition, browser_companion_wait_definition,
    browser_extract_definition, browser_open_definition,
};
#[path = "catalog_external_skills_definition_support.rs"]
mod external_skills_definition_support;
use external_skills_definition_support::{
    config_import_definition, external_skills_fetch_definition, external_skills_inspect_definition,
    external_skills_install_definition, external_skills_invoke_definition,
    external_skills_list_definition, external_skills_policy_definition,
    external_skills_recommend_definition, external_skills_remove_definition,
    external_skills_resolve_definition, external_skills_search_definition,
    external_skills_source_search_definition, provider_switch_definition,
};
#[path = "catalog_session_definition_support.rs"]
mod session_definition_support;
use session_definition_support::{
    approval_request_resolve_definition, approval_request_status_definition,
    approval_requests_list_definition, delegate_async_definition, delegate_definition,
    session_archive_definition, session_cancel_definition, session_continue_definition,
    session_events_definition, session_recover_definition, session_search_definition,
    session_status_definition, session_tool_policy_clear_definition,
    session_tool_policy_set_definition, session_tool_policy_status_definition,
    session_wait_definition, sessions_history_definition, sessions_list_definition,
    sessions_send_definition,
};

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

/// Semantic action families for the autonomy-policy kernel.
///
/// This taxonomy intentionally tracks policy-relevant boundary crossings
/// instead of modeling every possible side effect as its own class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityActionClass {
    Discover,
    // Ordinary already-visible execution stays in this bucket unless it crosses
    // a policy boundary such as acquisition, switching, or topology mutation.
    ExecuteExisting,
    CapabilityFetch,
    CapabilityInstall,
    CapabilityLoad,
    RuntimeSwitch,
    TopologyExpand,
    PolicyMutation,
    SessionMutation,
}

impl CapabilityActionClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discover => "discover",
            Self::ExecuteExisting => "execute_existing",
            Self::CapabilityFetch => "capability_fetch",
            Self::CapabilityInstall => "capability_install",
            Self::CapabilityLoad => "capability_load",
            Self::RuntimeSwitch => "runtime_switch",
            Self::TopologyExpand => "topology_expand",
            Self::PolicyMutation => "policy_mutation",
            Self::SessionMutation => "session_mutation",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct ToolPolicyDescriptor {
    scheduling_class: ToolSchedulingClass,
    governance_profile: ToolGovernanceProfile,
}

const ROUTINE_LOW_GOVERNANCE_PROFILE: ToolGovernanceProfile = ToolGovernanceProfile {
    scope: ToolGovernanceScope::Routine,
    risk_class: ToolRiskClass::Low,
    approval_mode: ToolApprovalMode::Never,
};

const ROUTINE_ELEVATED_GOVERNANCE_PROFILE: ToolGovernanceProfile = ToolGovernanceProfile {
    scope: ToolGovernanceScope::Routine,
    risk_class: ToolRiskClass::Elevated,
    approval_mode: ToolApprovalMode::PolicyDriven,
};

const ROUTINE_HIGH_GOVERNANCE_PROFILE: ToolGovernanceProfile = ToolGovernanceProfile {
    scope: ToolGovernanceScope::Routine,
    risk_class: ToolRiskClass::High,
    approval_mode: ToolApprovalMode::PolicyDriven,
};

const FAIL_CLOSED_GOVERNANCE_PROFILE: ToolGovernanceProfile = ROUTINE_HIGH_GOVERNANCE_PROFILE;

const TOPOLOGY_MUTATION_GOVERNANCE_PROFILE: ToolGovernanceProfile = ToolGovernanceProfile {
    scope: ToolGovernanceScope::TopologyMutation,
    risk_class: ToolRiskClass::High,
    approval_mode: ToolApprovalMode::PolicyDriven,
};

const DEFAULT_TOOL_POLICY_DESCRIPTOR: ToolPolicyDescriptor = ToolPolicyDescriptor {
    scheduling_class: ToolSchedulingClass::SerialOnly,
    governance_profile: ROUTINE_LOW_GOVERNANCE_PROFILE,
};

const PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR: ToolPolicyDescriptor = ToolPolicyDescriptor {
    scheduling_class: ToolSchedulingClass::ParallelSafe,
    governance_profile: ROUTINE_LOW_GOVERNANCE_PROFILE,
};

const ELEVATED_TOOL_POLICY_DESCRIPTOR: ToolPolicyDescriptor = ToolPolicyDescriptor {
    scheduling_class: ToolSchedulingClass::SerialOnly,
    governance_profile: ROUTINE_ELEVATED_GOVERNANCE_PROFILE,
};

const HIGH_RISK_TOOL_POLICY_DESCRIPTOR: ToolPolicyDescriptor = ToolPolicyDescriptor {
    scheduling_class: ToolSchedulingClass::SerialOnly,
    governance_profile: ROUTINE_HIGH_GOVERNANCE_PROFILE,
};

const TOPOLOGY_MUTATION_TOOL_POLICY_DESCRIPTOR: ToolPolicyDescriptor = ToolPolicyDescriptor {
    scheduling_class: ToolSchedulingClass::SerialOnly,
    governance_profile: TOPOLOGY_MUTATION_GOVERNANCE_PROFILE,
};

pub fn governance_profile_for_tool_name(tool_name: &str) -> ToolGovernanceProfile {
    let catalog = tool_catalog();
    let descriptor = catalog.resolve(tool_name);
    let Some(descriptor) = descriptor else {
        return FAIL_CLOSED_GOVERNANCE_PROFILE;
    };
    descriptor.governance_profile()
}

pub fn governance_profile_for_descriptor(descriptor: &ToolDescriptor) -> ToolGovernanceProfile {
    descriptor.governance_profile()
}

pub fn capability_action_class_for_tool_name(tool_name: &str) -> Option<CapabilityActionClass> {
    let catalog = tool_catalog();
    let descriptor = catalog.resolve(tool_name)?;
    Some(descriptor.capability_action_class())
}

pub fn capability_action_class_for_descriptor(
    descriptor: &ToolDescriptor,
) -> CapabilityActionClass {
    descriptor.capability_action_class()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolExposureClass {
    Direct,
    Gateway,
    Discoverable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolVisibilityGate {
    Always,
    Sessions,
    SessionMutation,
    Messages,
    Feishu,
    Delegate,
    Browser,
    BrowserCompanion,
    BashRuntime,
    ExternalSkills,
    MemorySearchCorpus,
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
    capability_action_class: CapabilityActionClass,
    policy: ToolPolicyDescriptor,
    concurrency_class: ToolConcurrencyClass,
    provider_definition_builder: fn(&ToolDescriptor) -> Value,
}

fn primary_surface_id(raw: &str) -> bool {
    matches!(
        raw,
        "read" | "write" | "exec" | "web" | "browser" | "memory" | "agent" | "skills" | "channel"
    )
}

impl ToolDescriptor {
    pub fn matches_name(&self, raw: &str) -> bool {
        if self.name == raw {
            return true;
        }
        if self.provider_name == raw {
            return true;
        }
        if self.aliases.contains(&raw) {
            return true;
        }

        if primary_surface_id(raw) {
            return false;
        }

        let discovery_name = super::tool_surface::discovery_tool_name_for_tool_name(self.name);
        if discovery_name == raw {
            return true;
        }

        super::tool_surface::legacy_discovery_tool_names_for_tool_name(self.name)
            .iter()
            .any(|legacy_name| legacy_name == raw)
    }

    pub fn provider_definition(&self) -> Value {
        (self.provider_definition_builder)(self)
    }

    pub fn argument_hint(&self) -> &'static str {
        tool_argument_hint(self.name)
    }

    pub fn search_hint(&self) -> &'static str {
        tool_search_hint(self.name, self.description)
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

    pub fn surface_id(&self) -> Option<&'static str> {
        super::tool_surface::tool_surface_id_for_name(self.name)
    }

    pub fn usage_guidance(&self) -> Option<&'static str> {
        super::tool_surface::tool_surface_usage_guidance(self.name)
    }

    pub fn is_direct(&self) -> bool {
        self.exposure == ToolExposureClass::Direct
    }

    pub fn is_gateway(&self) -> bool {
        self.exposure == ToolExposureClass::Gateway
    }

    pub fn is_provider_exposed(&self) -> bool {
        self.is_direct() || self.is_gateway()
    }

    pub fn is_discoverable(&self) -> bool {
        self.exposure == ToolExposureClass::Discoverable
    }

    pub fn capability_action_class(&self) -> CapabilityActionClass {
        self.capability_action_class
    }

    pub fn scheduling_class(&self) -> ToolSchedulingClass {
        self.policy.scheduling_class
    }

    pub fn concurrency_class(&self) -> ToolConcurrencyClass {
        self.concurrency_class
    }

    pub fn governance_profile(&self) -> ToolGovernanceProfile {
        self.policy.governance_profile
    }

    pub fn requires_kernel_binding(&self) -> bool {
        let execution_kind = self.execution_kind;
        if execution_kind != ToolExecutionKind::App {
            return false;
        }

        let governance_profile = self.governance_profile();
        let approval_mode = governance_profile.approval_mode;

        approval_mode == ToolApprovalMode::PolicyDriven
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
    pub surface_id: Option<&'static str>,
    pub usage_guidance: Option<&'static str>,
    pub exposure: ToolExposureClass,
    pub execution_kind: ToolExecutionKind,
    pub availability: ToolAvailability,
    pub capability_action_class: CapabilityActionClass,
    pub scheduling_class: ToolSchedulingClass,
    pub concurrency_class: ToolConcurrencyClass,
}

impl ToolCatalogEntry {
    pub fn is_direct(&self) -> bool {
        self.exposure == ToolExposureClass::Direct
    }

    pub fn is_gateway(&self) -> bool {
        self.exposure == ToolExposureClass::Gateway
    }

    pub fn is_provider_exposed(&self) -> bool {
        self.is_direct() || self.is_gateway()
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
    descriptor_indices: BTreeMap<&'static str, usize>,
    resolved_name_indices: BTreeMap<String, usize>,
    all_entries: Box<[ToolCatalogEntry]>,
    provider_exposed_entries: Box<[ToolCatalogEntry]>,
    catalog_digest: String,
}

struct ToolCatalogEntryCaches {
    all_entries: Box<[ToolCatalogEntry]>,
    provider_exposed_entries: Box<[ToolCatalogEntry]>,
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
        let index = self.descriptor_indices.get(tool_name)?;
        self.descriptors.get(*index)
    }

    pub fn resolve(&self, raw_tool_name: &str) -> Option<&ToolDescriptor> {
        let index = self.resolved_name_indices.get(raw_tool_name)?;
        self.descriptors.get(*index)
    }

    pub fn descriptors(&self) -> &[ToolDescriptor] {
        &self.descriptors
    }

    fn all_entries(&self) -> &[ToolCatalogEntry] {
        &self.all_entries
    }

    fn provider_exposed_entries(&self) -> &[ToolCatalogEntry] {
        &self.provider_exposed_entries
    }

    fn catalog_digest(&self) -> &str {
        self.catalog_digest.as_str()
    }
}

fn feishu_declared_concurrency_class(tool_name: &str) -> Option<ToolConcurrencyClass> {
    if !tool_name.starts_with("feishu.") {
        return None;
    }

    if tool_name == "feishu.messages.resource.get" {
        // This downloads remote content into the configured local file root.
        return Some(ToolConcurrencyClass::Mutating);
    }

    let tags = tool_tags(tool_name);

    if tags.contains(&"read") {
        return Some(ToolConcurrencyClass::ReadOnly);
    }

    if tags.contains(&"write") {
        return Some(ToolConcurrencyClass::Mutating);
    }

    if tags.contains(&"update") {
        return Some(ToolConcurrencyClass::Mutating);
    }

    if tags.contains(&"callback") {
        return Some(ToolConcurrencyClass::Mutating);
    }

    None
}

fn declared_concurrency_class(tool_name: &str) -> ToolConcurrencyClass {
    let explicit_class = match tool_name {
        "read"
        | "web"
        | "memory"
        | "tool.search"
        | "external_skills.resolve"
        | "external_skills.search"
        | "external_skills.recommend"
        | "external_skills.source_search"
        | "external_skills.inspect"
        | "external_skills.list"
        | "approval_request_status"
        | "approval_requests_list"
        | "session_events"
        | "session_tool_policy_status"
        | "session_search"
        | "session_status"
        | "session_wait"
        | "sessions_history"
        | "sessions_list"
        | "file.read"
        | "glob.search"
        | "content.search"
        | "memory_search"
        | "memory_get"
        | "browser.companion.snapshot"
        | "browser.companion.wait"
        | "browser.extract"
        | "web.fetch"
        | "web.search" => Some(ToolConcurrencyClass::ReadOnly),
        "write"
        | "exec"
        | "browser"
        | "config.import"
        | "external_skills.fetch"
        | "external_skills.install"
        | "external_skills.invoke"
        | "external_skills.policy"
        | "external_skills.remove"
        | "provider.switch"
        | "approval_request_resolve"
        | "delegate"
        | "delegate_async"
        | "session_archive"
        | "session_cancel"
        | "session_tool_policy_set"
        | "session_tool_policy_clear"
        | "session_recover"
        | "sessions_send"
        | "http.request"
        | "file.write"
        | "file.edit"
        | "shell.exec"
        | "bash.exec"
        | "browser.click"
        | "browser.companion.click"
        | "browser.companion.navigate"
        | "browser.companion.session.start"
        | "browser.companion.session.stop"
        | "browser.companion.type"
        | "browser.open" => Some(ToolConcurrencyClass::Mutating),
        _ => None,
    };

    if let Some(explicit_class) = explicit_class {
        return explicit_class;
    }

    if let Some(feishu_class) = feishu_declared_concurrency_class(tool_name) {
        return feishu_class;
    }

    ToolConcurrencyClass::Unknown
}

fn annotate_tool_concurrency_classes(descriptors: &mut [ToolDescriptor]) {
    for descriptor in descriptors {
        descriptor.concurrency_class = declared_concurrency_class(descriptor.name);
    }
}

fn build_tool_catalog() -> ToolCatalog {
    let mut descriptors = vec![
        ToolDescriptor {
            name: "tool.search",
            provider_name: "tool_search",
            aliases: &[],
            description: "Discover hidden specialized tools relevant to the current task.",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Gateway,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::Discover,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: tool_search_definition,
        },
        ToolDescriptor {
            name: "tool.invoke",
            provider_name: "tool_invoke",
            aliases: &[],
            description: "Invoke a discovered hidden specialized tool using a valid lease from tool_search.",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Gateway,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: tool_invoke_definition,
        },
        ToolDescriptor {
            name: "read",
            provider_name: "read",
            aliases: &[],
            description: "Read workspace files, page through large files, search file contents, or list matching paths",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Direct,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: direct_read_definition,
        },
        ToolDescriptor {
            name: "write",
            provider_name: "write",
            aliases: &[],
            description: "Write workspace files or apply one or more exact text edits",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Direct,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: direct_write_definition,
        },
        ToolDescriptor {
            name: "exec",
            provider_name: "exec",
            aliases: &[],
            description: "Run guarded workspace commands or raw shell scripts",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Direct,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: direct_exec_definition,
        },
        ToolDescriptor {
            name: "web",
            provider_name: "web",
            aliases: &[],
            description: "Fetch a URL, send HTTP requests, or search the public web",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Direct,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: direct_web_definition,
        },
        ToolDescriptor {
            name: "browser",
            provider_name: "browser",
            aliases: &[],
            description: "Open pages, extract content, or follow discovered links",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Direct,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: direct_browser_definition,
        },
        ToolDescriptor {
            name: "memory",
            provider_name: "memory",
            aliases: &[],
            description: "Search or read durable memory notes",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Direct,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: direct_memory_definition,
        },
        ToolDescriptor {
            name: "config.import",
            provider_name: "config_import",
            aliases: &["claw.migrate", "claw_migrate"],
            description: "Import legacy agent workspace config, profile, and external-skills mapping state into native Loong settings",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: config_import_definition,
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
            capability_action_class: CapabilityActionClass::CapabilityFetch,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: external_skills_fetch_definition,
        },
        ToolDescriptor {
            name: "external_skills.resolve",
            provider_name: "external_skills_resolve",
            aliases: &[],
            description: "Normalize an external skill reference into a source-aware candidate",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            capability_action_class: CapabilityActionClass::Discover,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: external_skills_resolve_definition,
        },
        ToolDescriptor {
            name: "external_skills.search",
            provider_name: "external_skills_search",
            aliases: &[],
            description: "Search the resolved external-skills inventory for active and shadowed matches",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            capability_action_class: CapabilityActionClass::Discover,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: external_skills_search_definition,
        },
        ToolDescriptor {
            name: "external_skills.recommend",
            provider_name: "external_skills_recommend",
            aliases: &[],
            description: "Recommend the best-fit resolved external skills for an operator goal",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            capability_action_class: CapabilityActionClass::Discover,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: external_skills_recommend_definition,
        },
        ToolDescriptor {
            name: "external_skills.source_search",
            provider_name: "external_skills_source_search",
            aliases: &[],
            description: "Search preferred external skill ecosystems and return normalized source-aware candidates",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::ExternalSkills,
            capability_action_class: CapabilityActionClass::Discover,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: external_skills_source_search_definition,
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
            capability_action_class: CapabilityActionClass::Discover,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::CapabilityInstall,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::CapabilityLoad,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::Discover,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::PolicyMutation,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: ELEVATED_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::RuntimeSwitch,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::TopologyExpand,
            policy: TOPOLOGY_MUTATION_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::TopologyExpand,
            policy: TOPOLOGY_MUTATION_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            visibility_gate: ToolVisibilityGate::SessionMutation,
            capability_action_class: CapabilityActionClass::SessionMutation,
            policy: ELEVATED_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            visibility_gate: ToolVisibilityGate::SessionMutation,
            capability_action_class: CapabilityActionClass::SessionMutation,
            policy: ELEVATED_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: session_cancel_definition,
        },
        ToolDescriptor {
            name: "session_continue",
            provider_name: "session_continue",
            aliases: &[],
            description: "Continue a visible delegate child session with a follow-up task",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::SessionMutation,
            capability_action_class: CapabilityActionClass::SessionMutation,
            policy: ELEVATED_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: session_continue_definition,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: session_events_definition,
        },
        ToolDescriptor {
            name: "session_tool_policy_status",
            provider_name: "session_tool_policy_status",
            aliases: &[],
            description: "Inspect the session-scoped tool policy for a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: session_tool_policy_status_definition,
        },
        ToolDescriptor {
            name: "session_tool_policy_set",
            provider_name: "session_tool_policy_set",
            aliases: &[],
            description: "Update the session-scoped tool policy for a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::SessionMutation,
            capability_action_class: CapabilityActionClass::PolicyMutation,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: session_tool_policy_set_definition,
        },
        ToolDescriptor {
            name: "session_tool_policy_clear",
            provider_name: "session_tool_policy_clear",
            aliases: &[],
            description: "Clear the session-scoped tool policy for a visible session",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::SessionMutation,
            capability_action_class: CapabilityActionClass::PolicyMutation,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: session_tool_policy_clear_definition,
        },
        ToolDescriptor {
            name: "session_search",
            provider_name: "session_search",
            aliases: &[],
            description: "Search visible canonical session history across transcript turns and session events",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Sessions,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: session_search_definition,
        },
        ToolDescriptor {
            name: "session_recover",
            provider_name: "session_recover",
            aliases: &[],
            description: "Recover an overdue queued async delegate child session by marking it failed",
            execution_kind: ToolExecutionKind::App,
            availability: runtime_session_tool_availability(),
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::SessionMutation,
            capability_action_class: CapabilityActionClass::SessionMutation,
            policy: ELEVATED_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: ELEVATED_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: sessions_send_definition,
        },
    ];

    #[cfg(feature = "feishu-integration")]
    {
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.app.create",
            "feishu_bitable_app_create",
            "Create a Feishu Bitable app with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.app.get",
            "feishu_bitable_app_get",
            "Fetch Feishu Bitable app metadata with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.app.list",
            "feishu_bitable_app_list",
            "List Feishu Bitable apps through the Drive API with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.app.patch",
            "feishu_bitable_app_patch",
            "Update Feishu Bitable app metadata with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.app.copy",
            "feishu_bitable_app_copy",
            "Copy a Feishu Bitable app with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.list",
            "feishu_bitable_list",
            "List data tables in a Feishu Bitable app with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.table.create",
            "feishu_bitable_table_create",
            "Create a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.table.patch",
            "feishu_bitable_table_patch",
            "Rename a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.table.batch_create",
            "feishu_bitable_table_batch_create",
            "Batch create Feishu Bitable tables with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.record.create",
            "feishu_bitable_record_create",
            "Create a record in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.record.update",
            "feishu_bitable_record_update",
            "Update a record in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.record.delete",
            "feishu_bitable_record_delete",
            "Delete a record in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.record.batch_create",
            "feishu_bitable_record_batch_create",
            "Batch create records in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.record.batch_update",
            "feishu_bitable_record_batch_update",
            "Batch update records in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.record.batch_delete",
            "feishu_bitable_record_batch_delete",
            "Batch delete records in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.field.create",
            "feishu_bitable_field_create",
            "Create a field in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.field.list",
            "feishu_bitable_field_list",
            "List fields in a Feishu Bitable table with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.field.update",
            "feishu_bitable_field_update",
            "Update a field in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.field.delete",
            "feishu_bitable_field_delete",
            "Delete a field in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.view.create",
            "feishu_bitable_view_create",
            "Create a view in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.view.get",
            "feishu_bitable_view_get",
            "Fetch a view in a Feishu Bitable table with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.view.list",
            "feishu_bitable_view_list",
            "List views in a Feishu Bitable table with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.view.patch",
            "feishu_bitable_view_patch",
            "Patch a view in a Feishu Bitable table with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.bitable.record.search",
            "feishu_bitable_record_search",
            "Search or list records in a Feishu Bitable table with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.calendar.freebusy",
            "feishu_calendar_freebusy",
            "Query Feishu calendar free/busy for the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.calendar.list",
            "feishu_calendar_list",
            "List Feishu calendars or primary calendars for the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.calendar.primary.get",
            "feishu_calendar_primary_get",
            "Fetch the Feishu primary calendar entry for the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.card.update",
            "feishu_card_update",
            "Update a Feishu interactive card through the delayed callback API, using the current callback token when available",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.doc.append",
            "feishu_doc_append",
            "Append markdown or html content to an existing Feishu docx document with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.doc.create",
            "feishu_doc_create",
            "Create a Feishu docx document and optionally insert initial markdown or html content with the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.doc.read",
            "feishu_doc_read",
            "Read Feishu Doc raw content with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.messages.get",
            "feishu_messages_get",
            "Read one Feishu message detail using a tenant token resolved from the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.messages.history",
            "feishu_messages_history",
            "List Feishu message history using a tenant token resolved from the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        #[cfg(feature = "tool-file")]
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.messages.resource.get",
            "feishu_messages_resource_get",
            "Download one Feishu message image or file resource under the configured file root, with safe ingress defaults when the current Feishu turn carries exactly one resource reference",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.messages.reply",
            "feishu_messages_reply",
            "Reply to a Feishu message with text, post, image, file, or a markdown card using a tenant token resolved from the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.messages.search",
            "feishu_messages_search",
            "Search Feishu messages with the selected account grant",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.messages.send",
            "feishu_messages_send",
            "Send a Feishu text, post, image, file, or markdown card message with a tenant token resolved from the selected account grant",
            ELEVATED_TOOL_POLICY_DESCRIPTOR,
        );
        push_feishu_tool_descriptor(
            &mut descriptors,
            "feishu.whoami",
            "feishu_whoami",
            "Inspect the active Feishu grant principal and profile",
            DEFAULT_TOOL_POLICY_DESCRIPTOR,
        );
    }

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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: file_read_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "glob.search",
            provider_name: "glob_search",
            aliases: &[],
            description: "Search the workspace for files matching a glob pattern",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: glob_search_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "content.search",
            provider_name: "content_search",
            aliases: &[],
            description: "Search workspace file contents for a text match with bounded results",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: content_search_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "memory_search",
            provider_name: "memory_search",
            aliases: &[],
            description: "Search durable workspace memory files and canonical cross-session recall with bounded snippets",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::MemorySearchCorpus,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: file_write_definition,
        });
        descriptors.push(ToolDescriptor {
            name: "file.edit",
            provider_name: "file_edit",
            aliases: &[],
            description: "Apply one or more exact text edits to a file",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: file_edit_definition,
        });
    }

    #[cfg(feature = "tool-shell")]
    {
        descriptors.push(ToolDescriptor {
            name: "shell.exec",
            provider_name: "shell_exec",
            aliases: &["shell"],
            description: "Execute shell commands. Inline stdout and stderr are capped; details.handoff.recommended_payload and details.handoff.recipes expose read-ready follow-up payloads when full output is saved.",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: shell_exec_definition,
        });
    }

    #[cfg(feature = "tool-shell")]
    {
        descriptors.push(ToolDescriptor {
            name: "bash.exec",
            provider_name: "bash_exec",
            aliases: &[],
            description: "Execute bash commands. Inline stdout and stderr are capped; details.handoff.recommended_payload and details.handoff.recipes expose read-ready follow-up payloads when full output is saved.",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::BashRuntime,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: bash_exec_definition,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: browser_open_definition,
        });
    }

    #[cfg(feature = "tool-http")]
    {
        descriptors.push(ToolDescriptor {
            name: "http.request",
            provider_name: "http_request",
            aliases: &["http_request"],
            description:
                "Send a bounded HTTP request with status, headers, and text or binary body output",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::WebFetch,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: HIGH_RISK_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: http_request_definition,
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
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
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
                "Search the web for APIs, documentation, and error messages using configured web-search providers. This search mode is separate from plain URL fetch/request network access",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::Discoverable,
            visibility_gate: ToolVisibilityGate::WebSearch,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: PARALLEL_SAFE_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: web_search_definition,
        });
    }

    annotate_tool_concurrency_classes(&mut descriptors);
    descriptors.sort_by(|left, right| left.name.cmp(right.name));

    let descriptor_indices = build_descriptor_indices(descriptors.as_slice());
    let resolved_name_indices = build_resolved_name_indices(descriptors.as_slice());
    let entry_caches = build_tool_catalog_entry_caches(descriptors.as_slice());
    let all_entries = entry_caches.all_entries;
    let provider_exposed_entries = entry_caches.provider_exposed_entries;
    let catalog_digest = build_tool_catalog_digest(all_entries.as_ref());

    ToolCatalog {
        descriptors,
        descriptor_indices,
        resolved_name_indices,
        all_entries,
        provider_exposed_entries,
        catalog_digest,
    }
}

fn build_descriptor_indices(descriptors: &[ToolDescriptor]) -> BTreeMap<&'static str, usize> {
    let mut descriptor_indices = BTreeMap::new();

    for (index, descriptor) in descriptors.iter().enumerate() {
        descriptor_indices.insert(descriptor.name, index);
    }

    descriptor_indices
}

fn build_resolved_name_indices(descriptors: &[ToolDescriptor]) -> BTreeMap<String, usize> {
    let mut resolved_name_indices = BTreeMap::new();

    for (index, descriptor) in descriptors.iter().enumerate() {
        resolved_name_indices
            .entry(descriptor.name.to_owned())
            .or_insert(index);
        resolved_name_indices
            .entry(descriptor.provider_name.to_owned())
            .or_insert(index);

        for alias in descriptor.aliases {
            resolved_name_indices
                .entry((*alias).to_owned())
                .or_insert(index);
        }

        let discovery_name =
            super::tool_surface::discovery_tool_name_for_tool_name(descriptor.name);
        let primary_surface_name = primary_surface_id(discovery_name.as_str());
        let descriptor_is_primary_surface = descriptor.name == discovery_name;
        if !primary_surface_name || descriptor_is_primary_surface {
            resolved_name_indices.entry(discovery_name).or_insert(index);
        }

        for legacy_name in
            super::tool_surface::legacy_discovery_tool_names_for_tool_name(descriptor.name)
        {
            resolved_name_indices.entry(legacy_name).or_insert(index);
        }
    }

    resolved_name_indices
}

fn build_tool_catalog_entry_caches(descriptors: &[ToolDescriptor]) -> ToolCatalogEntryCaches {
    let mut all_entries = Vec::new();
    let mut provider_exposed_entries = Vec::new();

    for descriptor in descriptors {
        let entry = descriptor_to_entry(descriptor);

        if descriptor.is_provider_exposed() {
            provider_exposed_entries.push(entry);
        }

        all_entries.push(entry);
    }

    ToolCatalogEntryCaches {
        all_entries: all_entries.into_boxed_slice(),
        provider_exposed_entries: provider_exposed_entries.into_boxed_slice(),
    }
}

fn build_tool_catalog_digest(entries: &[ToolCatalogEntry]) -> String {
    let payload = serde_json::to_vec(entries).unwrap_or_default();
    let digest = sha2::Sha256::digest(payload);
    hex::encode(digest)
}

pub(crate) fn stable_tool_catalog_digest() -> &'static str {
    tool_catalog().catalog_digest()
}

pub fn tool_catalog() -> &'static ToolCatalog {
    static TOOL_CATALOG: OnceLock<ToolCatalog> = OnceLock::new();

    TOOL_CATALOG.get_or_init(build_tool_catalog)
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
    delegate_child_tool_view_with_constraints(
        config,
        &config.delegate.child_tool_allowlist,
        config.delegate.allow_shell_in_child,
        false,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn delegate_child_tool_view_for_runtime_config(
    config: &ToolConfig,
    runtime_config: &ToolRuntimeConfig,
) -> ToolView {
    delegate_child_tool_view_for_runtime_config_with_delegate(config, runtime_config, false)
}

pub fn delegate_child_tool_view_for_config_with_delegate(
    config: &ToolConfig,
    allow_delegate: bool,
) -> ToolView {
    build_delegate_child_tool_view(
        config,
        None,
        &config.delegate.child_tool_allowlist,
        config.delegate.allow_shell_in_child,
        allow_delegate,
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn delegate_child_tool_view_for_runtime_config_with_delegate(
    config: &ToolConfig,
    runtime_config: &ToolRuntimeConfig,
    allow_delegate: bool,
) -> ToolView {
    build_delegate_child_tool_view(
        config,
        Some(runtime_config),
        &config.delegate.child_tool_allowlist,
        config.delegate.allow_shell_in_child,
        allow_delegate,
    )
}

pub fn delegate_child_tool_view_with_constraints(
    config: &ToolConfig,
    child_tool_allowlist: &[String],
    allow_shell_in_child: bool,
    allow_delegate: bool,
) -> ToolView {
    build_delegate_child_tool_view(
        config,
        None,
        child_tool_allowlist,
        allow_shell_in_child,
        allow_delegate,
    )
}

pub fn delegate_child_tool_view_for_contract(
    config: &ToolConfig,
    contract: Option<&ConstrainedSubagentContractView>,
) -> ToolView {
    let Some(contract) = contract else {
        return delegate_child_tool_view_for_config_with_delegate(config, false);
    };
    let allow_delegate = contract.allows_child_delegation();
    let allow_shell_in_child = contract.allow_shell_in_child.unwrap_or(false);
    delegate_child_tool_view_with_constraints(
        config,
        &contract.child_tool_allowlist,
        allow_shell_in_child,
        allow_delegate,
    )
}

fn build_delegate_child_tool_view(
    config: &ToolConfig,
    runtime_config: Option<&ToolRuntimeConfig>,
    child_tool_allowlist: &[String],
    allow_shell_in_child: bool,
    allow_delegate: bool,
) -> ToolView {
    let catalog = tool_catalog();
    let mut names = Vec::new();
    let allowlist = BTreeSet::<&str>::from_iter(child_tool_allowlist.iter().map(String::as_str));

    for descriptor in catalog.descriptors().iter().filter(|descriptor| {
        descriptor.execution_kind == ToolExecutionKind::Core
            && descriptor.availability == ToolAvailability::Runtime
    }) {
        match descriptor.name {
            "shell.exec" =>
            {
                #[cfg(feature = "tool-shell")]
                if allow_shell_in_child {
                    names.push(descriptor.name);
                }
            }
            name if allowlist.contains(name)
                && tool_visibility_gate_enabled_for_delegate_child(
                    descriptor.visibility_gate,
                    config,
                    runtime_config,
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

fn tool_visibility_gate_enabled_for_delegate_child(
    gate: ToolVisibilityGate,
    config: &ToolConfig,
    runtime_config: Option<&ToolRuntimeConfig>,
) -> bool {
    match gate {
        ToolVisibilityGate::BashRuntime => runtime_config.is_some_and(|config| {
            tool_visibility_gate_enabled_for_runtime_policy(ToolVisibilityGate::BashRuntime, config)
        }),
        ToolVisibilityGate::Always
        | ToolVisibilityGate::Sessions
        | ToolVisibilityGate::SessionMutation
        | ToolVisibilityGate::Messages
        | ToolVisibilityGate::Feishu
        | ToolVisibilityGate::Delegate
        | ToolVisibilityGate::Browser
        | ToolVisibilityGate::BrowserCompanion
        | ToolVisibilityGate::ExternalSkills
        | ToolVisibilityGate::MemorySearchCorpus
        | ToolVisibilityGate::MemoryFileRoot
        | ToolVisibilityGate::WebFetch
        | ToolVisibilityGate::WebSearch => {
            tool_visibility_gate_enabled_for_runtime_view(gate, config, false)
        }
    }
}

pub fn provider_exposed_tool_catalog() -> Vec<ToolCatalogEntry> {
    tool_catalog().provider_exposed_entries().to_vec()
}

pub fn all_tool_catalog() -> Vec<ToolCatalogEntry> {
    tool_catalog().all_entries().to_vec()
}

pub fn find_tool_catalog_entry(name: &str) -> Option<ToolCatalogEntry> {
    let catalog = tool_catalog();
    let descriptor = catalog.resolve(name)?;
    let index = catalog.descriptor_indices.get(descriptor.name)?;
    let entry = catalog.all_entries().get(*index)?;

    Some(*entry)
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
        surface_id: descriptor.surface_id(),
        usage_guidance: descriptor.usage_guidance(),
        exposure: descriptor.exposure,
        execution_kind: descriptor.execution_kind,
        availability: descriptor.availability,
        capability_action_class: descriptor.capability_action_class(),
        scheduling_class: descriptor.scheduling_class(),
        concurrency_class: descriptor.concurrency_class(),
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
        ToolVisibilityGate::SessionMutation => {
            let sessions_enabled = config.sessions.enabled;
            let allow_mutation = config.sessions.allow_mutation;
            sessions_enabled && allow_mutation
        }
        ToolVisibilityGate::Messages => config.messages.enabled,
        ToolVisibilityGate::Feishu => false,
        ToolVisibilityGate::Delegate => config.delegate.enabled,
        ToolVisibilityGate::Browser => config.browser.enabled,
        ToolVisibilityGate::BrowserCompanion => false,
        ToolVisibilityGate::BashRuntime => false,
        ToolVisibilityGate::ExternalSkills => external_skills_enabled,
        ToolVisibilityGate::MemorySearchCorpus => config
            .file_root
            .as_deref()
            .is_some_and(text_has_non_whitespace_segments),
        ToolVisibilityGate::MemoryFileRoot => config
            .file_root
            .as_deref()
            .is_some_and(text_has_non_whitespace_segments),
        ToolVisibilityGate::WebFetch => config.web.enabled,
        ToolVisibilityGate::WebSearch => config.web_search.enabled,
    }
}

fn text_has_non_whitespace_segments(value: &str) -> bool {
    !value.trim().is_empty()
}

#[cfg(not(feature = "tool-file"))]
fn path_has_non_whitespace_segments<P>(path: P) -> bool
where
    P: AsRef<Path>,
{
    let path_ref = path.as_ref();
    let path_text = path_ref.as_os_str().to_string_lossy();

    !path_text.trim().is_empty()
}

fn tool_visibility_gate_enabled_for_runtime_policy(
    gate: ToolVisibilityGate,
    config: &ToolRuntimeConfig,
) -> bool {
    match gate {
        ToolVisibilityGate::Always => true,
        ToolVisibilityGate::Sessions => config.sessions_enabled,
        ToolVisibilityGate::SessionMutation => {
            let sessions_enabled = config.sessions_enabled;
            let allow_mutation = config.sessions_allow_mutation;
            sessions_enabled && allow_mutation
        }
        ToolVisibilityGate::Messages => config.messages_enabled,
        ToolVisibilityGate::Feishu => {
            #[cfg(feature = "feishu-integration")]
            {
                config.feishu.is_some()
            }

            #[cfg(not(feature = "feishu-integration"))]
            {
                false
            }
        }
        ToolVisibilityGate::Delegate => config.delegate_enabled,
        ToolVisibilityGate::Browser => config.browser.enabled,
        ToolVisibilityGate::BrowserCompanion => config.browser_companion.is_runtime_ready(),
        ToolVisibilityGate::BashRuntime => config.bash_exec.is_discoverable(),
        ToolVisibilityGate::ExternalSkills => config.external_skills.enabled,
        ToolVisibilityGate::MemorySearchCorpus => {
            #[cfg(feature = "tool-file")]
            {
                super::memory_tools::memory_corpus_available(config)
            }

            #[cfg(not(feature = "tool-file"))]
            {
                config
                    .file_root
                    .as_deref()
                    .is_some_and(path_has_non_whitespace_segments)
            }
        }
        ToolVisibilityGate::MemoryFileRoot => {
            #[cfg(feature = "tool-file")]
            {
                super::memory_tools::workspace_memory_corpus_available(config)
            }

            #[cfg(not(feature = "tool-file"))]
            {
                config
                    .file_root
                    .as_deref()
                    .is_some_and(path_has_non_whitespace_segments)
            }
        }
        ToolVisibilityGate::WebFetch => config.web_fetch.enabled,
        ToolVisibilityGate::WebSearch => config.web_search.enabled,
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

fn glob_search_definition(descriptor: &ToolDescriptor) -> Value {
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

fn content_search_definition(descriptor: &ToolDescriptor) -> Value {
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

fn http_request_definition(descriptor: &ToolDescriptor) -> Value {
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

fn bash_exec_definition(descriptor: &ToolDescriptor) -> Value {
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

#[cfg(feature = "feishu-integration")]
fn push_feishu_tool_descriptor(
    descriptors: &mut Vec<ToolDescriptor>,
    name: &'static str,
    provider_name: &'static str,
    description: &'static str,
    policy: ToolPolicyDescriptor,
) {
    descriptors.push(ToolDescriptor {
        name,
        provider_name,
        aliases: &[],
        description,
        execution_kind: ToolExecutionKind::Core,
        availability: ToolAvailability::Runtime,
        exposure: ToolExposureClass::Discoverable,
        visibility_gate: ToolVisibilityGate::Feishu,
        capability_action_class: CapabilityActionClass::ExecuteExisting,
        policy,
        concurrency_class: ToolConcurrencyClass::Unknown,
        provider_definition_builder: feishu_definition,
    });
}

#[cfg(feature = "feishu-integration")]
fn feishu_definition(descriptor: &ToolDescriptor) -> Value {
    crate::tools::feishu::feishu_provider_tool_definition(descriptor.name).unwrap_or_else(|| {
        json!({
            "type": "function",
            "function": {
                "name": descriptor.provider_name,
                "description": descriptor.description,
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            }
        })
    })
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
    fn memory_file_root_visibility_gate_rejects_whitespace_only_paths() {
        let view_config = ToolConfig {
            file_root: Some("   ".to_owned()),
            ..ToolConfig::default()
        };
        let runtime_config = ToolRuntimeConfig {
            file_root: Some(std::path::PathBuf::from("   ")),
            ..ToolRuntimeConfig::default()
        };

        assert!(!tool_visibility_gate_enabled_for_runtime_view(
            ToolVisibilityGate::MemoryFileRoot,
            &view_config,
            false
        ));
        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::MemoryFileRoot,
            &runtime_config
        ));
        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::MemorySearchCorpus,
            &runtime_config
        ));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn memory_search_corpus_visibility_gate_allows_canonical_memory_without_workspace_files() {
        let runtime_dir = tempdir().expect("tempdir");
        let db_path = runtime_dir.path().join("memory.sqlite3");
        crate::memory::append_turn_direct_with_sqlite_path(
            "canonical-search-gate-session",
            "assistant",
            "Rollback checklist includes smoke tests and release notes.",
            &db_path,
        )
        .expect("append canonical turn");

        let runtime = ToolRuntimeConfig {
            file_root: None,
            memory_sqlite_path: Some(db_path),
            ..ToolRuntimeConfig::default()
        };
        assert!(tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::MemorySearchCorpus,
            &runtime
        ));
        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::MemoryFileRoot,
            &runtime
        ));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn runtime_tool_view_includes_memory_search_for_canonical_memory_without_workspace_files() {
        let runtime_dir = tempdir().expect("tempdir");
        let db_path = runtime_dir.path().join("memory.sqlite3");
        crate::memory::append_turn_direct_with_sqlite_path(
            "canonical-view-session",
            "assistant",
            "Rollback checklist includes smoke tests and release notes.",
            &db_path,
        )
        .expect("append canonical turn");

        let runtime = ToolRuntimeConfig {
            file_root: None,
            memory_sqlite_path: Some(db_path),
            ..ToolRuntimeConfig::default()
        };
        let tool_view = runtime_tool_view_for_runtime_config(&runtime);

        assert!(tool_view.contains("memory_search"));
        assert!(!tool_view.contains("memory_get"));
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

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_visibility_gate_requires_runtime_configuration() {
        let hidden_runtime = ToolRuntimeConfig::default();
        let hidden_view = runtime_tool_view_for_runtime_config(&hidden_runtime);

        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::Feishu,
            &hidden_runtime
        ));
        assert!(!hidden_view.contains("feishu.card.update"));

        let visible_runtime = ToolRuntimeConfig {
            feishu: Some(crate::tools::runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
                    app_secret: Some(loong_contracts::SecretRef::Inline("app-secret".to_owned())),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig::default(),
            }),
            ..ToolRuntimeConfig::default()
        };
        let visible_view = runtime_tool_view_for_runtime_config(&visible_runtime);
        let descriptor = tool_catalog()
            .resolve("feishu_card_update")
            .expect("feishu card update descriptor");

        assert!(tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::Feishu,
            &visible_runtime
        ));
        assert_eq!(descriptor.name, "feishu.card.update");
        assert_eq!(descriptor.visibility_gate, ToolVisibilityGate::Feishu);
        assert!(visible_view.contains("feishu.card.update"));

        let primary_descriptor = tool_catalog()
            .resolve("feishu_calendar_primary_get")
            .expect("feishu.calendar.primary.get descriptor");
        assert_eq!(primary_descriptor.name, "feishu.calendar.primary.get");
        assert_eq!(
            primary_descriptor.visibility_gate,
            ToolVisibilityGate::Feishu
        );
        assert!(visible_view.contains("feishu.calendar.primary.get"));
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
    fn delegate_child_tool_view_for_contract_fails_closed_without_profile() {
        let config = ToolConfig::default();
        let child_view = delegate_child_tool_view_for_contract(&config, None);

        assert!(!child_view.contains("delegate"));
        assert!(!child_view.contains("delegate_async"));
    }

    #[test]
    fn delegate_child_tool_view_for_contract_allows_nested_delegate_when_profile_permits() {
        let config = ToolConfig::default();
        let contract = ConstrainedSubagentContractView::from_profile(ConstrainedSubagentProfile {
            role: crate::conversation::ConstrainedSubagentRole::Orchestrator,
            control_scope: crate::conversation::ConstrainedSubagentControlScope::Children,
        });
        let child_view = delegate_child_tool_view_for_contract(&config, Some(&contract));

        assert!(child_view.contains("delegate"));
        assert!(child_view.contains("delegate_async"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn delegate_child_tool_view_hides_allowlisted_bash_exec_without_runtime_visibility() {
        let mut config = ToolConfig::default();
        config.delegate.child_tool_allowlist = vec!["bash.exec".to_owned()];

        let child_view = delegate_child_tool_view_for_config(&config);

        assert!(!child_view.contains("bash.exec"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn bash_runtime_visibility_gate_hides_bash_exec_when_governance_rules_failed_to_load() {
        let runtime = ToolRuntimeConfig {
            bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
                available: true,
                command: Some(std::path::PathBuf::from("bash")),
                governance: crate::tools::runtime_config::BashGovernanceRuntimePolicy {
                    load_error: Some("broken rules".to_owned()),
                    ..crate::tools::runtime_config::BashGovernanceRuntimePolicy::default()
                },
                ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
            },
            ..ToolRuntimeConfig::default()
        };

        assert!(!tool_visibility_gate_enabled_for_runtime_policy(
            ToolVisibilityGate::BashRuntime,
            &runtime
        ));
        assert!(!runtime_tool_view_for_runtime_config(&runtime).contains("bash.exec"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn delegate_child_tool_view_hides_allowlisted_bash_exec_when_governance_rules_failed_to_load() {
        let mut config = ToolConfig::default();
        config.delegate.child_tool_allowlist = vec!["bash.exec".to_owned()];
        let runtime = ToolRuntimeConfig {
            bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
                available: true,
                command: Some(std::path::PathBuf::from("bash")),
                governance: crate::tools::runtime_config::BashGovernanceRuntimePolicy {
                    load_error: Some("broken rules".to_owned()),
                    ..crate::tools::runtime_config::BashGovernanceRuntimePolicy::default()
                },
                ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
            },
            ..ToolRuntimeConfig::default()
        };

        let child_view = delegate_child_tool_view_for_runtime_config(&config, &runtime);

        assert!(!child_view.contains("bash.exec"));
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn delegate_child_tool_view_exposes_allowlisted_bash_exec_when_runtime_ready() {
        let mut config = ToolConfig::default();
        config.delegate.child_tool_allowlist = vec!["bash.exec".to_owned()];
        let runtime = ToolRuntimeConfig {
            bash_exec: crate::tools::runtime_config::BashExecRuntimePolicy {
                available: true,
                command: Some(std::path::PathBuf::from("bash")),
                ..crate::tools::runtime_config::BashExecRuntimePolicy::default()
            },
            ..ToolRuntimeConfig::default()
        };

        let child_view = delegate_child_tool_view_for_runtime_config(&config, &runtime);

        assert!(child_view.contains("bash.exec"));
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
                .descriptor("session_search")
                .expect("session_search descriptor")
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
    fn tool_catalog_entries_expose_concurrency_class() {
        let search = find_tool_catalog_entry("tool.search").expect("tool.search catalog entry");
        assert_eq!(search.scheduling_class, ToolSchedulingClass::ParallelSafe);
        assert_eq!(search.concurrency_class, ToolConcurrencyClass::ReadOnly);

        let invoke = find_tool_catalog_entry("tool.invoke").expect("tool.invoke catalog entry");
        assert_eq!(invoke.scheduling_class, ToolSchedulingClass::SerialOnly);
        assert_eq!(invoke.concurrency_class, ToolConcurrencyClass::Unknown);

        let delegate_async =
            find_tool_catalog_entry("delegate_async").expect("delegate_async catalog entry");
        assert_eq!(
            delegate_async.scheduling_class,
            ToolSchedulingClass::SerialOnly
        );
        assert_eq!(
            delegate_async.concurrency_class,
            ToolConcurrencyClass::Mutating
        );

        #[cfg(feature = "tool-http")]
        {
            let http_request =
                find_tool_catalog_entry("http.request").expect("http.request catalog entry");
            assert_eq!(
                http_request.scheduling_class,
                ToolSchedulingClass::SerialOnly
            );
            assert_eq!(
                http_request.concurrency_class,
                ToolConcurrencyClass::Mutating
            );
        }

        let file_write = find_tool_catalog_entry("file.write").expect("file.write catalog entry");
        assert_eq!(file_write.scheduling_class, ToolSchedulingClass::SerialOnly);
        assert_eq!(file_write.concurrency_class, ToolConcurrencyClass::Mutating);
        assert_eq!(file_write.surface_id, Some("write"));
        assert!(
            file_write
                .usage_guidance
                .is_some_and(|guidance| guidance.contains("normal patching and file creation"))
        );

        let bash_exec = find_tool_catalog_entry("bash.exec").expect("bash.exec catalog entry");
        assert_eq!(bash_exec.scheduling_class, ToolSchedulingClass::SerialOnly);
        assert_eq!(bash_exec.concurrency_class, ToolConcurrencyClass::Mutating);
        assert_eq!(bash_exec.surface_id, Some("exec"));
    }

    #[test]
    fn tool_catalog_resolve_preserves_canonical_provider_and_alias_lookup() {
        let catalog = tool_catalog();

        let canonical = catalog.resolve("tool.search").expect("canonical lookup");
        let provider_name = catalog.resolve("tool_search").expect("provider lookup");
        let alias = catalog.resolve("shell").expect("alias lookup");

        assert_eq!(canonical.name, "tool.search");
        assert_eq!(provider_name.name, "tool.search");
        assert_eq!(alias.name, "shell.exec");
    }

    #[test]
    fn cached_catalog_entry_partitions_match_descriptor_filters() {
        let catalog = tool_catalog();

        let expected_all_entries = descriptor_identity_list(catalog.descriptors().iter());
        let expected_provider_exposed_entries = descriptor_identity_list(
            catalog
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.is_provider_exposed()),
        );

        let actual_all_entries = entry_identity_list(all_tool_catalog().iter());
        let actual_provider_exposed_entries =
            entry_identity_list(provider_exposed_tool_catalog().iter());

        assert_eq!(actual_all_entries, expected_all_entries);
        assert_eq!(
            actual_provider_exposed_entries,
            expected_provider_exposed_entries
        );
    }

    fn descriptor_identity_list<'a>(
        descriptors: impl Iterator<Item = &'a ToolDescriptor>,
    ) -> Vec<(&'static str, &'static str, ToolExposureClass)> {
        let mut identities = Vec::new();

        for descriptor in descriptors {
            let identity = (
                descriptor.name,
                descriptor.provider_name,
                descriptor.exposure,
            );
            identities.push(identity);
        }

        identities
    }

    fn entry_identity_list<'a>(
        entries: impl Iterator<Item = &'a ToolCatalogEntry>,
    ) -> Vec<(&'static str, &'static str, ToolExposureClass)> {
        let mut identities = Vec::new();

        for entry in entries {
            let identity = (
                entry.canonical_name,
                entry.provider_function_name,
                entry.exposure,
            );
            identities.push(identity);
        }

        identities
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_tool_catalog_entries_expose_explicit_concurrency_class() {
        let catalog = tool_catalog();
        let feishu_descriptors: Vec<&ToolDescriptor> = catalog
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.name.starts_with("feishu."))
            .collect();

        assert!(!feishu_descriptors.is_empty());

        for descriptor in feishu_descriptors {
            assert_ne!(
                descriptor.concurrency_class(),
                ToolConcurrencyClass::Unknown,
                "{} should expose an explicit concurrency class",
                descriptor.name
            );
        }

        let calendar_list =
            find_tool_catalog_entry("feishu.calendar.list").expect("feishu.calendar.list entry");
        assert_eq!(
            calendar_list.concurrency_class,
            ToolConcurrencyClass::ReadOnly
        );

        let calendar_primary_get = find_tool_catalog_entry("feishu.calendar.primary.get")
            .expect("feishu.calendar.primary.get entry");
        assert_eq!(
            calendar_primary_get.concurrency_class,
            ToolConcurrencyClass::ReadOnly
        );
        assert_eq!(
            calendar_primary_get.required_fields,
            &[] as &[&str],
            "primary.get has no required fields"
        );
        assert!(
            calendar_primary_get.tags.contains(&"feishu")
                && calendar_primary_get.tags.contains(&"calendar")
                && calendar_primary_get.tags.contains(&"read"),
            "primary.get should carry the feishu+calendar+read tags, got: {:?}",
            calendar_primary_get.tags
        );

        let messages_send =
            find_tool_catalog_entry("feishu.messages.send").expect("feishu.messages.send entry");
        assert_eq!(
            messages_send.concurrency_class,
            ToolConcurrencyClass::Mutating
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "tool-file"))]
    #[test]
    fn feishu_resource_download_catalog_entry_is_mutating() {
        let entry = find_tool_catalog_entry("feishu.messages.resource.get")
            .expect("feishu.messages.resource.get entry");

        assert_eq!(entry.concurrency_class, ToolConcurrencyClass::Mutating);
    }

    #[test]
    fn governance_profile_follows_descriptor_declared_policy() {
        let catalog = tool_catalog();

        let delegate_async = catalog
            .descriptor("delegate_async")
            .expect("delegate_async descriptor");
        let delegate_async_policy = governance_profile_for_descriptor(delegate_async);

        assert_eq!(
            delegate_async_policy.scope,
            ToolGovernanceScope::TopologyMutation
        );
        assert_eq!(delegate_async_policy.risk_class, ToolRiskClass::High);
        assert_eq!(
            delegate_async_policy.approval_mode,
            ToolApprovalMode::PolicyDriven
        );

        let sessions_send_policy = governance_profile_for_tool_name("sessions_send");

        assert_eq!(sessions_send_policy.scope, ToolGovernanceScope::Routine);
        assert_eq!(sessions_send_policy.risk_class, ToolRiskClass::Elevated);
        assert_eq!(
            sessions_send_policy.approval_mode,
            ToolApprovalMode::PolicyDriven
        );

        let external_skills_policy = governance_profile_for_tool_name("external_skills.policy");

        assert_eq!(external_skills_policy.scope, ToolGovernanceScope::Routine);
        assert_eq!(external_skills_policy.risk_class, ToolRiskClass::High);
        assert_eq!(
            external_skills_policy.approval_mode,
            ToolApprovalMode::PolicyDriven
        );

        let unknown_policy = governance_profile_for_tool_name("unknown.tool");

        assert_eq!(unknown_policy, FAIL_CLOSED_GOVERNANCE_PROFILE);
    }

    #[cfg(feature = "tool-browser")]
    #[test]
    fn governance_profile_resolves_alias_backed_tool_metadata() {
        let policy = governance_profile_for_tool_name("browser_companion_click");

        assert_eq!(policy.scope, ToolGovernanceScope::Routine);
        assert_eq!(policy.risk_class, ToolRiskClass::High);
        assert_eq!(policy.approval_mode, ToolApprovalMode::PolicyDriven);
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn governance_profile_resolves_alias_distinct_from_provider_name() {
        let catalog = tool_catalog();
        let descriptor = catalog
            .descriptor("shell.exec")
            .expect("shell.exec descriptor");
        let expected_policy = governance_profile_for_descriptor(descriptor);
        let alias_policy = governance_profile_for_tool_name("shell");

        assert_ne!(descriptor.provider_name, "shell");
        assert!(descriptor.aliases.contains(&"shell"));
        assert_eq!(alias_policy, expected_policy);
    }

    #[cfg(feature = "tool-shell")]
    #[test]
    fn bash_exec_uses_high_risk_governance_profile() {
        let policy = governance_profile_for_tool_name("bash.exec");

        assert_eq!(policy.scope, ToolGovernanceScope::Routine);
        assert_eq!(policy.risk_class, ToolRiskClass::High);
        assert_eq!(policy.approval_mode, ToolApprovalMode::PolicyDriven);
    }

    #[test]
    fn config_import_alias_resolves_descriptor_governance() {
        let catalog = tool_catalog();
        let descriptor = catalog
            .descriptor("config.import")
            .expect("config.import descriptor");
        let expected_policy = governance_profile_for_descriptor(descriptor);
        let legacy_alias_policy = governance_profile_for_tool_name("claw.migrate");

        assert!(descriptor.aliases.contains(&"claw.migrate"));
        assert!(descriptor.aliases.contains(&"claw_migrate"));
        assert_eq!(legacy_alias_policy, expected_policy);
    }

    #[test]
    fn autonomy_capability_action_is_independent_from_governance_profile() {
        let catalog = tool_catalog();
        let migrate = catalog
            .descriptor("config.import")
            .expect("config.import descriptor");
        let provider_switch = catalog
            .descriptor("provider.switch")
            .expect("provider.switch descriptor");
        let migrate_policy = governance_profile_for_descriptor(migrate);
        let provider_switch_policy = governance_profile_for_descriptor(provider_switch);

        assert_eq!(migrate_policy, provider_switch_policy);
        assert_eq!(
            migrate.scheduling_class(),
            provider_switch.scheduling_class()
        );
        assert_eq!(
            capability_action_class_for_descriptor(migrate),
            CapabilityActionClass::ExecuteExisting
        );
        assert_eq!(
            capability_action_class_for_descriptor(provider_switch),
            CapabilityActionClass::RuntimeSwitch
        );
        assert_ne!(
            migrate.capability_action_class(),
            provider_switch.capability_action_class()
        );
    }

    #[test]
    fn autonomy_capability_action_classifies_representative_tool_families() {
        let expectations = [
            ("tool.search", CapabilityActionClass::Discover),
            ("tool_search", CapabilityActionClass::Discover),
            ("tool.invoke", CapabilityActionClass::ExecuteExisting),
            ("config.import", CapabilityActionClass::ExecuteExisting),
            (
                "external_skills.fetch",
                CapabilityActionClass::CapabilityFetch,
            ),
            (
                "external_skills.install",
                CapabilityActionClass::CapabilityInstall,
            ),
            (
                "external_skills.invoke",
                CapabilityActionClass::CapabilityLoad,
            ),
            ("provider.switch", CapabilityActionClass::RuntimeSwitch),
            ("delegate", CapabilityActionClass::TopologyExpand),
            ("delegate_async", CapabilityActionClass::TopologyExpand),
            (
                "approval_request_resolve",
                CapabilityActionClass::ExecuteExisting,
            ),
            (
                "external_skills.policy",
                CapabilityActionClass::PolicyMutation,
            ),
            ("session_archive", CapabilityActionClass::SessionMutation),
            ("session_cancel", CapabilityActionClass::SessionMutation),
            ("session_continue", CapabilityActionClass::SessionMutation),
            ("session_events", CapabilityActionClass::ExecuteExisting),
            (
                "session_tool_policy_status",
                CapabilityActionClass::ExecuteExisting,
            ),
            (
                "session_tool_policy_set",
                CapabilityActionClass::PolicyMutation,
            ),
            (
                "session_tool_policy_clear",
                CapabilityActionClass::PolicyMutation,
            ),
            ("session_search", CapabilityActionClass::ExecuteExisting),
            ("session_recover", CapabilityActionClass::SessionMutation),
        ];

        for (tool_name, expected_action_class) in expectations {
            let resolved_action_class = capability_action_class_for_tool_name(tool_name)
                .unwrap_or_else(|| panic!("missing action class for `{tool_name}`"));

            assert_eq!(resolved_action_class, expected_action_class);
        }
    }

    #[test]
    fn autonomy_capability_action_catalog_entries_expose_serializable_metadata() {
        let delegate_async =
            find_tool_catalog_entry("delegate_async").expect("delegate_async catalog entry");
        let delegate_async_value =
            serde_json::to_value(delegate_async).expect("serialize delegate_async catalog entry");
        let search = find_tool_catalog_entry("tool.search").expect("tool.search catalog entry");
        let search_value =
            serde_json::to_value(search).expect("serialize tool.search catalog entry");
        let invoke = find_tool_catalog_entry("tool.invoke").expect("tool.invoke catalog entry");
        let invoke_value =
            serde_json::to_value(invoke).expect("serialize tool.invoke catalog entry");

        assert_eq!(
            delegate_async.capability_action_class,
            CapabilityActionClass::TopologyExpand
        );
        assert_eq!(
            delegate_async_value["capability_action_class"],
            "topology_expand"
        );
        assert_eq!(delegate_async_value["concurrency_class"], "mutating");
        assert_eq!(search_value["concurrency_class"], "read_only");
        assert_eq!(invoke_value["concurrency_class"], "unknown");
    }

    #[test]
    fn autonomy_capability_action_returns_none_for_unknown_tools() {
        let action_class = capability_action_class_for_tool_name("unknown.tool");

        assert_eq!(action_class, None);
    }

    #[test]
    fn tool_catalog_lookup_tokens_are_globally_unambiguous() {
        let catalog = tool_catalog();
        let mut token_owners = std::collections::BTreeMap::new();

        for descriptor in catalog.descriptors() {
            let owner = descriptor.name;
            let mut lookup_tokens = BTreeSet::new();

            lookup_tokens.insert(descriptor.name);
            lookup_tokens.insert(descriptor.provider_name);

            for alias in descriptor.aliases {
                lookup_tokens.insert(*alias);
            }

            for token in lookup_tokens {
                let previous_owner = token_owners.insert(token, owner);

                if let Some(previous_owner) = previous_owner {
                    assert_eq!(
                        previous_owner, owner,
                        "lookup token `{token}` resolves to both `{previous_owner}` and `{owner}`"
                    );
                }
            }
        }
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

    #[test]
    fn session_tool_policy_set_definition_surfaces_runtime_narrowing_shape() {
        let descriptor = tool_catalog()
            .descriptor("session_tool_policy_set")
            .expect("session_tool_policy_set descriptor");
        let definition = descriptor.provider_definition();
        let runtime_narrowing =
            &definition["function"]["parameters"]["properties"]["runtime_narrowing"];

        assert_eq!(runtime_narrowing["type"], "object");
        assert!(
            runtime_narrowing["properties"]["browser"]["properties"]["max_sessions"].is_object()
        );
        assert!(
            runtime_narrowing["properties"]["web_fetch"]["properties"]["allowed_domains"]
                .is_object()
        );
    }

    #[test]
    fn delegate_definitions_surface_shared_and_worktree_isolation_modes() {
        let catalog = tool_catalog();

        for tool_name in ["delegate", "delegate_async"] {
            let descriptor = catalog.descriptor(tool_name).expect("delegate descriptor");
            let definition = descriptor.provider_definition();
            let isolation =
                &definition["function"]["parameters"]["properties"]["isolation"]["enum"];

            assert_eq!(*isolation, json!(["shared", "worktree"]));
        }
    }

    #[test]
    fn external_skills_policy_definition_surfaces_update_controls() {
        let descriptor = tool_catalog()
            .descriptor("external_skills.policy")
            .expect("external_skills.policy descriptor");
        let definition = descriptor.provider_definition();
        let properties = &definition["function"]["parameters"]["properties"];

        assert_eq!(properties["action"]["enum"], json!(["get", "set", "reset"]));
        assert!(properties["policy_update_approved"].is_object());
        assert!(properties["allowed_domains"].is_object());
        assert!(properties["blocked_domains"].is_object());
    }

    #[cfg(feature = "tool-browser")]
    #[test]
    fn browser_companion_type_definition_requires_session_selector_and_text() {
        let descriptor = tool_catalog()
            .descriptor("browser.companion.type")
            .expect("browser.companion.type descriptor");
        let definition = descriptor.provider_definition();
        let required = &definition["function"]["parameters"]["required"];

        assert_eq!(required, &json!(["session_id", "selector", "text"]));
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_bitable_record_search_catalog_metadata_includes_automatic_fields() {
        let descriptor = tool_catalog()
            .descriptor("feishu.bitable.record.search")
            .expect("feishu bitable record search descriptor");

        assert!(
            descriptor
                .argument_hint()
                .contains("automatic_fields?:boolean")
        );
        assert!(
            descriptor
                .parameter_types()
                .contains(&("automatic_fields", "boolean"))
        );
    }

    #[test]
    fn sessions_list_definition_and_hint_surface_offset_pagination() {
        let catalog = tool_catalog();
        let descriptor = catalog
            .descriptor("sessions_list")
            .expect("sessions_list descriptor");
        let definition = descriptor.provider_definition();
        let function_definition = &definition["function"];
        let parameter_definition = &function_definition["parameters"];
        let property_definition = &parameter_definition["properties"];
        let offset_definition = &property_definition["offset"];
        let offset_description_value = &offset_definition["description"];
        let offset_description = offset_description_value
            .as_str()
            .expect("offset description");
        let parameter_types = descriptor.parameter_types();
        let has_offset_parameter = parameter_types.contains(&("offset", "integer"));

        assert!(offset_description.contains("skip"));
        assert_eq!(
            descriptor.argument_hint(),
            "limit?:integer,offset?:integer,state?:string"
        );
        assert!(has_offset_parameter);
    }

    #[test]
    fn read_definitions_surface_line_window_fields() {
        let catalog = tool_catalog();
        let direct_descriptor = catalog.descriptor("read").expect("read descriptor");
        let direct_definition = direct_descriptor.provider_definition();
        let direct_properties = &direct_definition["function"]["parameters"]["properties"];
        let direct_parameter_types = direct_descriptor.parameter_types();

        assert!(direct_properties.get("offset").is_some());
        assert!(direct_properties.get("limit").is_some());
        assert!(
            direct_descriptor
                .argument_hint()
                .contains("offset?:integer")
        );
        assert!(direct_descriptor.argument_hint().contains("limit?:integer"));
        assert!(direct_parameter_types.contains(&("offset", "integer")));
        assert!(direct_parameter_types.contains(&("limit", "integer")));

        let file_descriptor = catalog
            .descriptor("file.read")
            .expect("file.read descriptor");
        let file_definition = file_descriptor.provider_definition();
        let file_properties = &file_definition["function"]["parameters"]["properties"];
        let file_parameter_types = file_descriptor.parameter_types();

        assert!(file_properties.get("offset").is_some());
        assert!(file_properties.get("limit").is_some());
        assert!(file_descriptor.argument_hint().contains("offset?:integer"));
        assert!(file_descriptor.argument_hint().contains("limit?:integer"));
        assert!(file_parameter_types.contains(&("offset", "integer")));
        assert!(file_parameter_types.contains(&("limit", "integer")));
    }

    #[test]
    fn exec_definition_supports_script_mode() {
        let catalog = tool_catalog();
        let descriptor = catalog.descriptor("exec").expect("exec descriptor");
        let definition = descriptor.provider_definition();
        let properties = &definition["function"]["parameters"]["properties"];
        let any_of = &definition["function"]["parameters"]["anyOf"];

        assert!(properties.get("script").is_some());
        assert!(descriptor.argument_hint().contains("script?:string"));
        assert!(descriptor.parameter_types().contains(&("script", "string")));
        assert_eq!(descriptor.required_fields(), Vec::<&str>::new());
        assert!(any_of.is_array());
    }
}
