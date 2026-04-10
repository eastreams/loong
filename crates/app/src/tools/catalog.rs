use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(not(feature = "tool-file"))]
use std::path::Path;
use std::sync::OnceLock;

use loongclaw_kernel::ToolConcurrencyClass;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;

use super::runtime_config::ToolRuntimeConfig;
use crate::config::ToolConfig;
use crate::conversation::ConstrainedSubagentContractView;
#[cfg(test)]
use crate::conversation::ConstrainedSubagentProfile;

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
    ProviderCore,
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

    pub fn is_provider_core(&self) -> bool {
        self.exposure == ToolExposureClass::ProviderCore
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
        let governance_profile = self.governance_profile();
        let approval_mode = governance_profile.approval_mode;
        let execution_kind = self.execution_kind;

        execution_kind == ToolExecutionKind::App && approval_mode == ToolApprovalMode::PolicyDriven
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
    pub capability_action_class: CapabilityActionClass,
    pub scheduling_class: ToolSchedulingClass,
    pub concurrency_class: ToolConcurrencyClass,
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
    descriptor_indices: BTreeMap<&'static str, usize>,
    resolved_name_indices: BTreeMap<&'static str, usize>,
    all_entries: Box<[ToolCatalogEntry]>,
    provider_core_entries: Box<[ToolCatalogEntry]>,
    discoverable_entries: Box<[ToolCatalogEntry]>,
    catalog_digest: String,
}

struct ToolCatalogEntryCaches {
    all_entries: Box<[ToolCatalogEntry]>,
    provider_core_entries: Box<[ToolCatalogEntry]>,
    discoverable_entries: Box<[ToolCatalogEntry]>,
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

    fn provider_core_entries(&self) -> &[ToolCatalogEntry] {
        &self.provider_core_entries
    }

    fn discoverable_entries(&self) -> &[ToolCatalogEntry] {
        &self.discoverable_entries
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
        "tool.search"
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
        "config.import"
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
            description: "Discover non-core tools",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::ProviderCore,
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
            description: "Invoke a discovered non-core tool",
            execution_kind: ToolExecutionKind::Core,
            availability: ToolAvailability::Runtime,
            exposure: ToolExposureClass::ProviderCore,
            visibility_gate: ToolVisibilityGate::Always,
            capability_action_class: CapabilityActionClass::ExecuteExisting,
            policy: DEFAULT_TOOL_POLICY_DESCRIPTOR,
            concurrency_class: ToolConcurrencyClass::Unknown,
            provider_definition_builder: tool_invoke_definition,
        },
        ToolDescriptor {
            name: "config.import",
            provider_name: "config_import",
            aliases: &["claw.migrate", "claw_migrate"],
            description: "Import legacy agent workspace config, profile, and external-skills mapping state into native LoongClaw settings",
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
            description: "Replace text in a file",
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
            description: "Execute shell commands",
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
            description: "Execute bash commands",
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
                "Search the web for APIs, documentation, and error messages using configured web search providers",
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
    let provider_core_entries = entry_caches.provider_core_entries;
    let discoverable_entries = entry_caches.discoverable_entries;
    let catalog_digest = build_tool_catalog_digest(all_entries.as_ref());

    ToolCatalog {
        descriptors,
        descriptor_indices,
        resolved_name_indices,
        all_entries,
        provider_core_entries,
        discoverable_entries,
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

fn build_resolved_name_indices(descriptors: &[ToolDescriptor]) -> BTreeMap<&'static str, usize> {
    let mut resolved_name_indices = BTreeMap::new();

    for (index, descriptor) in descriptors.iter().enumerate() {
        resolved_name_indices
            .entry(descriptor.name)
            .or_insert(index);
        resolved_name_indices
            .entry(descriptor.provider_name)
            .or_insert(index);

        for alias in descriptor.aliases {
            resolved_name_indices.entry(*alias).or_insert(index);
        }
    }

    resolved_name_indices
}

fn build_tool_catalog_entry_caches(descriptors: &[ToolDescriptor]) -> ToolCatalogEntryCaches {
    let mut all_entries = Vec::new();
    let mut provider_core_entries = Vec::new();
    let mut discoverable_entries = Vec::new();

    for descriptor in descriptors {
        let entry = descriptor_to_entry(descriptor);

        if descriptor.is_provider_core() {
            provider_core_entries.push(entry);
        }

        if descriptor.is_discoverable() {
            discoverable_entries.push(entry);
        }

        all_entries.push(entry);
    }

    ToolCatalogEntryCaches {
        all_entries: all_entries.into_boxed_slice(),
        provider_core_entries: provider_core_entries.into_boxed_slice(),
        discoverable_entries: discoverable_entries.into_boxed_slice(),
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

pub fn provider_core_tool_catalog() -> Vec<ToolCatalogEntry> {
    tool_catalog().provider_core_entries().to_vec()
}

pub fn discoverable_tool_catalog() -> Vec<ToolCatalogEntry> {
    tool_catalog().discoverable_entries().to_vec()
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
                        "description": "Natural-language description of the tool capability you need. Any language is acceptable."
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

fn config_import_definition(descriptor: &ToolDescriptor) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": "Import, discover, plan, merge, apply, and roll back legacy agent workspace config and related external-skills state into native LoongClaw config.",
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

fn external_skills_resolve_definition(descriptor: &ToolDescriptor) -> Value {
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

fn external_skills_search_definition(descriptor: &ToolDescriptor) -> Value {
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

fn external_skills_recommend_definition(descriptor: &ToolDescriptor) -> Value {
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

fn external_skills_source_search_definition(descriptor: &ToolDescriptor) -> Value {
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

fn session_search_definition(descriptor: &ToolDescriptor) -> Value {
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

fn session_tool_policy_status_definition(descriptor: &ToolDescriptor) -> Value {
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

fn session_tool_policy_set_definition(descriptor: &ToolDescriptor) -> Value {
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

fn session_tool_policy_clear_definition(descriptor: &ToolDescriptor) -> Value {
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
                "additionalProperties": false
            }
        }
    })
}

fn session_continue_definition(descriptor: &ToolDescriptor) -> Value {
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

fn tool_argument_hint(name: &str) -> &'static str {
    match name {
        "feishu.bitable.app.create" => {
            "account_id?:string,open_id?:string,name:string,folder_token?:string"
        }
        "feishu.bitable.app.get" => "account_id?:string,open_id?:string,app_token:string",
        "feishu.bitable.app.list" => {
            "account_id?:string,open_id?:string,folder_token?:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.app.patch" => {
            "account_id?:string,open_id?:string,app_token:string,name?:string,is_advanced?:boolean"
        }
        "feishu.bitable.app.copy" => {
            "account_id?:string,open_id?:string,app_token:string,name:string,folder_token?:string"
        }
        "feishu.bitable.list" => {
            "account_id?:string,open_id?:string,app_token:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.table.create" => {
            "account_id?:string,open_id?:string,app_token:string,name:string,default_view_name?:string,fields?:array"
        }
        "feishu.bitable.table.patch" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,name:string"
        }
        "feishu.bitable.table.batch_create" => {
            "account_id?:string,open_id?:string,app_token:string,tables:array"
        }
        "feishu.bitable.record.create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,fields:object"
        }
        "feishu.bitable.record.update" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,record_id:string,fields:object"
        }
        "feishu.bitable.record.delete" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,record_id:string"
        }
        "feishu.bitable.record.batch_create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,records:array"
        }
        "feishu.bitable.record.batch_update" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,records:array"
        }
        "feishu.bitable.record.batch_delete" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,records:array"
        }
        "feishu.bitable.field.create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,field_name:string,type:integer,property?:object"
        }
        "feishu.bitable.field.list" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id?:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.field.update" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,field_id:string,field_name:string,type:integer,property?:object"
        }
        "feishu.bitable.field.delete" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,field_id:string"
        }
        "feishu.bitable.view.create" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_name:string,view_type?:string"
        }
        "feishu.bitable.view.get" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id:string"
        }
        "feishu.bitable.view.list" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,page_size?:integer,page_token?:string"
        }
        "feishu.bitable.view.patch" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id:string,view_name:string"
        }
        "feishu.bitable.record.search" => {
            "account_id?:string,open_id?:string,app_token:string,table_id:string,view_id?:string,filter?:object,sort?:array,field_names?:string[],automatic_fields?:boolean,page_size?:integer,page_token?:string"
        }
        "feishu.calendar.freebusy" => {
            "account_id?:string,open_id?:string,time_min:string,time_max:string,user_id?:string,room_id?:string"
        }
        "feishu.calendar.list" => {
            "account_id?:string,open_id?:string,primary?:boolean,page_size?:integer,page_token?:string,sync_token?:string"
        }
        "feishu.card.update" => {
            "account_id?:string,callback_token?:string,card?:object,markdown?:string,shared?:boolean,open_ids?:string[]"
        }
        "feishu.doc.append" => {
            "account_id?:string,open_id?:string,url:string,content?:string,content_path?:string,content_type?:string"
        }
        "feishu.doc.create" => {
            "account_id?:string,open_id?:string,title?:string,folder_token?:string,content?:string,content_path?:string,content_type?:string"
        }
        "feishu.doc.read" => "account_id?:string,open_id?:string,url:string,lang?:integer",
        "feishu.messages.get" => "account_id?:string,open_id?:string,message_id:string",
        "feishu.messages.history" => {
            "account_id?:string,open_id?:string,container_id?:string,container_id_type?:string,page_size?:integer,page_token?:string"
        }
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" => {
            "account_id?:string,open_id?:string,message_id?:string,file_key?:string,type?:string,save_as?:string"
        }
        "feishu.messages.reply" => {
            "account_id?:string,open_id?:string,message_id:string,text?:string,post?:object,image_key?:string,file_key?:string,card?:object,markdown?:string"
        }
        "feishu.messages.search" => {
            "account_id?:string,open_id?:string,query:string,page_size?:integer,page_token?:string"
        }
        "feishu.messages.send" => {
            "account_id?:string,open_id?:string,receive_id:string,receive_id_type?:string,text?:string,post?:object,image_key?:string,file_key?:string,card?:object,markdown?:string"
        }
        "feishu.whoami" => "account_id?:string,open_id?:string",
        "tool.search" => "query?:string,exact_tool_id?:string,limit?:integer",
        "tool.invoke" => "tool_id:string,lease:string,arguments:object",
        "config.import" => {
            "input_path?:string,output_path?:string,mode?:string,source?:string,source_id?:string,primary_source_id?:string,safe_profile_merge?:boolean,apply_external_skills_plan?:boolean,force?:boolean"
        }
        "external_skills.fetch" => {
            "reference?:string,url?:string,approval_granted?:boolean,save_as?:string,max_bytes?:integer"
        }
        "external_skills.resolve" => "reference:string",
        "external_skills.search" => "query:string,limit:integer",
        "external_skills.recommend" => "query:string,limit:integer",
        "external_skills.source_search" => "query:string,max_results?:integer,sources?:string[]",
        "external_skills.inspect" => "skill_id:string",
        "external_skills.install" => {
            "path?:string,bundled_skill_id?:string,skill_id?:string,source_skill_id?:string,security_decision?:string,replace?:boolean"
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
        "http.request" => {
            "url:string,method?:string,headers?:object,body?:string,content_type?:string,max_bytes?:integer"
        }
        "file.read" => "path:string,max_bytes?:integer",
        "glob.search" => {
            "pattern:string,root?:string,max_results?:integer,include_directories?:boolean"
        }
        "content.search" => {
            "query:string,root?:string,glob?:string,max_results?:integer,max_bytes_per_file?:integer,case_sensitive?:boolean"
        }
        "memory_search" => "query:string,max_results?:integer",
        "memory_get" => "path:string,from?:integer,lines?:integer",
        "file.write" => "path:string,content:string,create_dirs?:boolean,overwrite?:boolean",
        "file.edit" => "path:string,old_string:string,new_string:string,replace_all?:boolean",
        "shell.exec" => "command:string,args?:string[],timeout_ms?:integer,cwd?:string",
        "bash.exec" => "command:string,cwd?:string,timeout_ms?:integer",
        "provider.switch" => "selector?:string",
        "delegate" | "delegate_async" => {
            "task:string,label?:string,profile?:string,isolation?:string,timeout_seconds?:integer"
        }
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => "session_id:string",
        "session_continue" => "session_id:string,input:string,timeout_seconds?:integer",
        "sessions_list" => "limit?:integer,offset?:integer,state?:string",
        "sessions_send" => "session_id:string,text:string",
        "web.search" => "query:string,provider?:string,max_results?:integer",
        _ => "",
    }
}

fn tool_search_hint(name: &str, fallback: &'static str) -> &'static str {
    match name {
        "tool.search" => {
            "discover a non-core tool for the task or refresh a known tool card by exact tool id"
        }
        "tool.invoke" => "invoke a discovered non-core tool with a valid short-lived lease",
        "http.request" => {
            "send a bounded http request, inspect status and headers, fetch text or binary responses"
        }
        "file.read" => "read a workspace file, inspect file contents, open a repo text file",
        "glob.search" => {
            "find workspace files by glob pattern, list files in a directory, browse folder contents, search repo paths, match files under a root"
        }
        "content.search" => {
            "search workspace file contents, find text in repo files, grep text in the project"
        }
        "file.write" => {
            "write a workspace file, save file content, create or overwrite a repo file"
        }
        "file.edit" => "edit a workspace file, patch file content, replace text in a repo file",
        "shell.exec" => {
            "run a shell command, execute a terminal command, bash, zsh, powershell, cli"
        }
        "web.fetch" => "fetch a web page, download page text, inspect http content from a url",
        "web.search" => "search the web, look up web results, find information online",
        "memory_search" => {
            "search durable workspace memory, recall prior notes, query stored memory"
        }
        "memory_get" => "read a memory note by path, inspect saved durable memory content",
        "provider.switch" => "switch model provider, change runtime provider selection",
        _ => fallback,
    }
}

fn tool_parameter_types(name: &str) -> &'static [(&'static str, &'static str)] {
    match name {
        "feishu.bitable.app.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("name", "string"),
            ("folder_token", "string"),
        ],
        "feishu.bitable.app.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
        ],
        "feishu.bitable.app.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("folder_token", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.app.patch" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("name", "string"),
            ("is_advanced", "boolean"),
        ],
        "feishu.bitable.app.copy" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("name", "string"),
            ("folder_token", "string"),
        ],
        "feishu.bitable.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.table.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("name", "string"),
            ("default_view_name", "string"),
            ("fields", "array"),
        ],
        "feishu.bitable.table.patch" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("name", "string"),
        ],
        "feishu.bitable.table.batch_create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("tables", "array"),
        ],
        "feishu.bitable.record.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("fields", "object"),
        ],
        "feishu.bitable.record.update" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("record_id", "string"),
            ("fields", "object"),
        ],
        "feishu.bitable.record.delete" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("record_id", "string"),
        ],
        "feishu.bitable.record.batch_create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("records", "array"),
        ],
        "feishu.bitable.record.batch_update" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("records", "array"),
        ],
        "feishu.bitable.record.batch_delete" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("records", "array"),
        ],
        "feishu.bitable.field.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("field_name", "string"),
            ("type", "integer"),
            ("property", "object"),
        ],
        "feishu.bitable.field.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.field.update" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("field_id", "string"),
            ("field_name", "string"),
            ("type", "integer"),
            ("property", "object"),
        ],
        "feishu.bitable.field.delete" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("field_id", "string"),
        ],
        "feishu.bitable.view.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_name", "string"),
            ("view_type", "string"),
        ],
        "feishu.bitable.view.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
        ],
        "feishu.bitable.view.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.bitable.view.patch" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
            ("view_name", "string"),
        ],
        "feishu.bitable.record.search" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("app_token", "string"),
            ("table_id", "string"),
            ("view_id", "string"),
            ("filter", "object"),
            ("sort", "array"),
            ("field_names", "array"),
            ("automatic_fields", "boolean"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.calendar.freebusy" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("time_min", "string"),
            ("time_max", "string"),
            ("user_id", "string"),
            ("room_id", "string"),
        ],
        "feishu.calendar.list" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("primary", "boolean"),
            ("page_size", "integer"),
            ("page_token", "string"),
            ("sync_token", "string"),
        ],
        "feishu.card.update" => &[
            ("account_id", "string"),
            ("callback_token", "string"),
            ("card", "object"),
            ("markdown", "string"),
            ("shared", "boolean"),
            ("open_ids", "array"),
        ],
        "feishu.doc.append" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("url", "string"),
            ("content", "string"),
            ("content_path", "string"),
            ("content_type", "string"),
        ],
        "feishu.doc.create" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("title", "string"),
            ("folder_token", "string"),
            ("content", "string"),
            ("content_path", "string"),
            ("content_type", "string"),
        ],
        "feishu.doc.read" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("url", "string"),
            ("lang", "integer"),
        ],
        "feishu.messages.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("message_id", "string"),
        ],
        "feishu.messages.history" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("container_id", "string"),
            ("container_id_type", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("message_id", "string"),
            ("file_key", "string"),
            ("type", "string"),
            ("save_as", "string"),
        ],
        "feishu.messages.reply" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("message_id", "string"),
            ("text", "string"),
            ("post", "object"),
            ("image_key", "string"),
            ("file_key", "string"),
            ("card", "object"),
            ("markdown", "string"),
        ],
        "feishu.messages.search" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("query", "string"),
            ("page_size", "integer"),
            ("page_token", "string"),
        ],
        "feishu.messages.send" => &[
            ("account_id", "string"),
            ("open_id", "string"),
            ("receive_id", "string"),
            ("receive_id_type", "string"),
            ("text", "string"),
            ("post", "object"),
            ("image_key", "string"),
            ("file_key", "string"),
            ("card", "object"),
            ("markdown", "string"),
        ],
        "feishu.whoami" => &[("account_id", "string"), ("open_id", "string")],
        "tool.search" => &[
            ("query", "string"),
            ("exact_tool_id", "string"),
            ("limit", "integer"),
        ],
        "tool.invoke" => &[
            ("tool_id", "string"),
            ("lease", "string"),
            ("arguments", "object"),
        ],
        "config.import" => &[
            ("input_path", "string"),
            ("output_path", "string"),
            ("mode", "string"),
            ("source", "string"),
            ("source_id", "string"),
            ("primary_source_id", "string"),
            ("safe_profile_merge", "boolean"),
            ("apply_external_skills_plan", "boolean"),
            ("force", "boolean"),
        ],
        "external_skills.fetch" => &[
            ("reference", "string"),
            ("url", "string"),
            ("approval_granted", "boolean"),
            ("save_as", "string"),
            ("max_bytes", "integer"),
        ],
        "external_skills.resolve" => &[("reference", "string")],
        "external_skills.search" => &[("query", "string"), ("limit", "integer")],
        "external_skills.recommend" => &[("query", "string"), ("limit", "integer")],
        "external_skills.source_search" => &[
            ("query", "string"),
            ("max_results", "integer"),
            ("sources", "array"),
        ],
        "external_skills.inspect" | "external_skills.invoke" | "external_skills.remove" => {
            &[("skill_id", "string")]
        }
        "external_skills.install" => &[
            ("path", "string"),
            ("bundled_skill_id", "string"),
            ("skill_id", "string"),
            ("source_skill_id", "string"),
            ("security_decision", "string"),
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
        "http.request" => &[
            ("url", "string"),
            ("method", "string"),
            ("headers", "object"),
            ("body", "string"),
            ("content_type", "string"),
            ("max_bytes", "integer"),
        ],
        "external_skills.policy" => &[
            ("action", "string"),
            ("enabled", "boolean"),
            ("allowed_domains", "array"),
            ("blocked_domains", "array"),
        ],
        "file.read" => &[("path", "string"), ("max_bytes", "integer")],
        "glob.search" => &[
            ("pattern", "string"),
            ("root", "string"),
            ("max_results", "integer"),
            ("include_directories", "boolean"),
        ],
        "content.search" => &[
            ("query", "string"),
            ("root", "string"),
            ("glob", "string"),
            ("max_results", "integer"),
            ("max_bytes_per_file", "integer"),
            ("case_sensitive", "boolean"),
        ],
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
            ("overwrite", "boolean"),
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
        "bash.exec" => &[
            ("command", "string"),
            ("cwd", "string"),
            ("timeout_ms", "integer"),
        ],
        "provider.switch" => &[("selector", "string")],
        "delegate" | "delegate_async" => &[
            ("task", "string"),
            ("label", "string"),
            ("profile", "string"),
            ("isolation", "string"),
            ("timeout_seconds", "integer"),
        ],
        "session_continue" => &[
            ("session_id", "string"),
            ("input", "string"),
            ("timeout_seconds", "integer"),
        ],
        "session_tool_policy_status" | "session_tool_policy_clear" => &[("session_id", "string")],
        "session_tool_policy_set" => &[
            ("session_id", "string"),
            ("tool_ids", "array"),
            ("runtime_narrowing", "object"),
        ],
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => &[("session_id", "string")],
        "sessions_list" => &[
            ("limit", "integer"),
            ("offset", "integer"),
            ("state", "string"),
        ],
        "session_search" => &[
            ("query", "string"),
            ("session_id", "string"),
            ("max_results", "integer"),
            ("include_archived", "boolean"),
            ("include_turns", "boolean"),
            ("include_events", "boolean"),
        ],
        "sessions_send" => &[("session_id", "string"), ("text", "string")],
        "web.search" => &[
            ("query", "string"),
            ("provider", "string"),
            ("max_results", "integer"),
        ],
        _ => &[],
    }
}

fn tool_required_fields(name: &str) -> &'static [&'static str] {
    match name {
        "feishu.bitable.app.create" => &["name"],
        "feishu.bitable.app.get" => &["app_token"],
        "feishu.bitable.app.list" => &[],
        "feishu.bitable.app.patch" => &["app_token"],
        "feishu.bitable.app.copy" => &["app_token", "name"],
        "feishu.bitable.list" => &["app_token"],
        "feishu.bitable.table.create" => &["app_token", "name"],
        "feishu.bitable.table.patch" => &["app_token", "table_id", "name"],
        "feishu.bitable.table.batch_create" => &["app_token", "tables"],
        "feishu.bitable.record.create" => &["app_token", "table_id", "fields"],
        "feishu.bitable.record.update" => &["app_token", "table_id", "record_id", "fields"],
        "feishu.bitable.record.delete" => &["app_token", "table_id", "record_id"],
        "feishu.bitable.record.batch_create"
        | "feishu.bitable.record.batch_update"
        | "feishu.bitable.record.batch_delete" => &["app_token", "table_id", "records"],
        "feishu.bitable.field.create" => &["app_token", "table_id", "field_name", "type"],
        "feishu.bitable.field.list" => &["app_token", "table_id"],
        "feishu.bitable.field.update" => {
            &["app_token", "table_id", "field_id", "field_name", "type"]
        }
        "feishu.bitable.field.delete" => &["app_token", "table_id", "field_id"],
        "feishu.bitable.view.create" => &["app_token", "table_id", "view_name"],
        "feishu.bitable.view.get" => &["app_token", "table_id", "view_id"],
        "feishu.bitable.view.list" => &["app_token", "table_id"],
        "feishu.bitable.view.patch" => &["app_token", "table_id", "view_id", "view_name"],
        "feishu.bitable.record.search" => &["app_token", "table_id"],
        "feishu.calendar.freebusy" => &["time_min", "time_max"],
        "feishu.doc.append" | "feishu.doc.read" => &["url"],
        "feishu.messages.get" => &["message_id"],
        "feishu.messages.reply" => &["message_id"],
        "feishu.messages.search" => &["query"],
        "feishu.messages.send" => &["receive_id"],
        "tool.search" => &[],
        "tool.invoke" => &["tool_id", "lease", "arguments"],
        "external_skills.fetch" => &[],
        "external_skills.resolve" => &["reference"],
        "external_skills.search" => &["query", "limit"],
        "external_skills.recommend" => &["query", "limit"],
        "external_skills.source_search" => &["query"],
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
        "http.request" => &["url"],
        "file.read" => &["path"],
        "glob.search" => &["pattern"],
        "content.search" => &["query"],
        "memory_search" => &["query"],
        "memory_get" => &["path"],
        "file.write" => &["path", "content"],
        "file.edit" => &["path", "old_string", "new_string"],
        "shell.exec" => &["command"],
        "bash.exec" => &["command"],
        "delegate" | "delegate_async" => &["task"],
        "session_tool_policy_status" | "session_tool_policy_clear" => &[],
        "session_tool_policy_set" => &[],
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" => &["session_id"],
        "session_continue" => &["session_id", "input"],
        "sessions_send" => &["session_id", "text"],
        "web.search" => &["query"],
        _ => &[],
    }
}

fn tool_tags(name: &str) -> &'static [&'static str] {
    match name {
        "feishu.bitable.app.get" | "feishu.bitable.app.list" => {
            &["feishu", "bitable", "app", "read"]
        }
        "feishu.bitable.app.create" | "feishu.bitable.app.patch" | "feishu.bitable.app.copy" => {
            &["feishu", "bitable", "app", "write"]
        }
        "feishu.bitable.list" | "feishu.bitable.record.search" => &["feishu", "bitable", "read"],
        "feishu.bitable.table.create"
        | "feishu.bitable.table.patch"
        | "feishu.bitable.table.batch_create" => &["feishu", "bitable", "table", "write"],
        "feishu.bitable.record.create"
        | "feishu.bitable.record.update"
        | "feishu.bitable.record.delete"
        | "feishu.bitable.record.batch_create"
        | "feishu.bitable.record.batch_update"
        | "feishu.bitable.record.batch_delete" => &["feishu", "bitable", "write"],
        "feishu.bitable.field.list" => &["feishu", "bitable", "field", "read"],
        "feishu.bitable.field.create"
        | "feishu.bitable.field.update"
        | "feishu.bitable.field.delete" => &["feishu", "bitable", "field", "write"],
        "feishu.bitable.view.get" | "feishu.bitable.view.list" => {
            &["feishu", "bitable", "view", "read"]
        }
        "feishu.bitable.view.create" | "feishu.bitable.view.patch" => {
            &["feishu", "bitable", "view", "write"]
        }
        "feishu.calendar.freebusy" | "feishu.calendar.list" => &["feishu", "calendar", "read"],
        "feishu.card.update" => &["feishu", "card", "update", "callback"],
        "feishu.doc.read" => &["feishu", "docs", "read"],
        "feishu.doc.create" | "feishu.doc.append" => &["feishu", "docs", "write"],
        "feishu.messages.get" | "feishu.messages.history" | "feishu.messages.search" => {
            &["feishu", "messages", "read"]
        }
        #[cfg(feature = "tool-file")]
        "feishu.messages.resource.get" => &["feishu", "messages", "resource", "file"],
        "feishu.messages.send" | "feishu.messages.reply" => &["feishu", "messages", "write"],
        "feishu.whoami" => &["feishu", "identity", "read"],
        "tool.search" => &["core", "discover", "search"],
        "tool.invoke" => &["core", "dispatch", "invoke"],
        "config.import" => &["config", "import", "migration", "workspace", "legacy"],
        "external_skills.fetch" => &["skills", "download", "external", "fetch"],
        "external_skills.resolve" => &["skills", "resolve", "normalize", "external"],
        "external_skills.search" => &["skills", "search", "inventory", "discover"],
        "external_skills.recommend" => &["skills", "recommend", "inventory", "discover"],
        "external_skills.source_search" => &["skills", "search", "discover", "external"],
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
        "http.request" => &["http", "request", "web", "network", "external"],
        "file.read" => &["file", "read", "filesystem", "repo"],
        "glob.search" => &[
            "file",
            "search",
            "glob",
            "filesystem",
            "repo",
            "directory",
            "folder",
            "list",
            "browse",
        ],
        "content.search" => &["file", "search", "content", "filesystem", "repo"],
        "memory_search" => &["memory", "search", "recall", "durable", "workspace"],
        "memory_get" => &["memory", "read", "recall", "durable", "workspace"],
        "file.write" => &["file", "write", "filesystem"],
        "file.edit" => &["file", "edit", "filesystem"],
        "shell.exec" => &["shell", "command", "process", "exec"],
        "bash.exec" => &["bash", "command", "process", "exec"],
        "provider.switch" => &["provider", "switch", "model", "runtime"],
        "delegate" | "delegate_async" => &["session", "delegate", "child"],
        "session_tool_policy_status" | "session_tool_policy_set" | "session_tool_policy_clear" => {
            &["session", "policy", "tools", "security"]
        }
        "session_archive" | "session_cancel" | "session_events" | "session_recover"
        | "session_status" | "session_wait" | "sessions_history" | "sessions_list" => {
            &["session", "history", "runtime"]
        }
        "session_continue" => &["session", "continue", "delegate", "child"],
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
        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        crate::memory::append_turn_direct(
            "canonical-search-gate-session",
            "assistant",
            "Rollback checklist includes smoke tests and release notes.",
            &memory_config,
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
        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        crate::memory::append_turn_direct(
            "canonical-view-session",
            "assistant",
            "Rollback checklist includes smoke tests and release notes.",
            &memory_config,
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
                    app_id: Some(loongclaw_contracts::SecretRef::Inline(
                        "cli_a1b2c3".to_owned(),
                    )),
                    app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                        "app-secret".to_owned(),
                    )),
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

        let bash_exec = find_tool_catalog_entry("bash.exec").expect("bash.exec catalog entry");
        assert_eq!(bash_exec.scheduling_class, ToolSchedulingClass::SerialOnly);
        assert_eq!(bash_exec.concurrency_class, ToolConcurrencyClass::Mutating);
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
        let expected_provider_core_entries = descriptor_identity_list(
            catalog
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.is_provider_core()),
        );
        let expected_discoverable_entries = descriptor_identity_list(
            catalog
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.is_discoverable()),
        );

        let actual_all_entries = entry_identity_list(all_tool_catalog().iter());
        let actual_provider_core_entries = entry_identity_list(provider_core_tool_catalog().iter());
        let actual_discoverable_entries = entry_identity_list(discoverable_tool_catalog().iter());

        assert_eq!(actual_all_entries, expected_all_entries);
        assert_eq!(actual_provider_core_entries, expected_provider_core_entries);
        assert_eq!(actual_discoverable_entries, expected_discoverable_entries);
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
}
