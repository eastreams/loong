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
#[path = "catalog_io_definition_support.rs"]
mod io_definition_support;
#[cfg(feature = "tool-websearch")]
use io_definition_support::web_search_definition;
use io_definition_support::{
    bash_exec_definition, content_search_definition, file_edit_definition, file_read_definition,
    file_write_definition, glob_search_definition, http_request_definition, memory_get_definition,
    memory_search_definition, shell_exec_definition, web_fetch_definition,
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
#[path = "catalog_tests.rs"]
mod tests;
