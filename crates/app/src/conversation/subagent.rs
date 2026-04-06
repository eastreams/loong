use std::path::PathBuf;

use serde::de::Error as DeError;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::runtime_config::ToolRuntimeNarrowing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentMode {
    Inline,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentIsolation {
    #[default]
    Shared,
    Worktree,
}

impl ConstrainedSubagentIsolation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Worktree => "worktree",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegateBuiltinProfile {
    Research,
    Plan,
    Verify,
}

impl DelegateBuiltinProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Plan => "plan",
            Self::Verify => "verify",
        }
    }

    pub const fn default_label(self) -> &'static str {
        match self {
            Self::Research => "Research",
            Self::Plan => "Plan",
            Self::Verify => "Verify",
        }
    }

    pub const fn default_timeout_seconds(self) -> u64 {
        match self {
            Self::Research => 60,
            Self::Plan => 30,
            Self::Verify => 45,
        }
    }

    pub const fn allows_shell_in_child(self) -> bool {
        matches!(self, Self::Verify)
    }

    pub fn from_lifecycle_profile(value: &str) -> Option<Self> {
        let trimmed_value = value.trim();
        if trimmed_value.is_empty() {
            return None;
        }
        match trimmed_value {
            "research" => Some(Self::Research),
            "plan" => Some(Self::Plan),
            "verify" => Some(Self::Verify),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentTerminalReason {
    Completed,
    Failed,
    TimedOut,
    SpawnFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentRole {
    Orchestrator,
    Leaf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentControlScope {
    Children,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentRuntimeBinding {
    Direct,
    KernelBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentBudgetSnapshot {
    pub current: usize,
    pub max: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConstrainedSubagentIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specialization: Option<String>,
}

impl ConstrainedSubagentIdentity {
    pub fn is_empty(&self) -> bool {
        let nickname_missing = self.nickname.is_none();
        let specialization_missing = self.specialization.is_none();
        nickname_missing && specialization_missing
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConstrainedSubagentHandle {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ConstrainedSubagentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<ConstrainedSubagentContractView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coordination: Vec<ConstrainedSubagentCoordinationAction>,
}

impl ConstrainedSubagentHandle {
    pub fn new(session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        Self {
            session_id,
            ..Self::default()
        }
    }

    pub fn with_parent_session_id(mut self, parent_session_id: Option<String>) -> Self {
        self.parent_session_id = parent_session_id;
        self
    }

    pub fn with_label(mut self, label: Option<String>) -> Self {
        self.label = label;
        self
    }

    pub fn with_state(mut self, state: Option<String>) -> Self {
        self.state = state;
        self
    }

    pub fn with_phase(mut self, phase: Option<String>) -> Self {
        self.phase = phase;
        self
    }

    pub fn with_identity(mut self, identity: Option<ConstrainedSubagentIdentity>) -> Self {
        let filtered_identity = identity.filter(|value| !value.is_empty());
        if let Some(identity) = filtered_identity {
            self.identity = Some(identity);
        }
        self
    }

    pub fn with_contract(mut self, contract: Option<ConstrainedSubagentContractView>) -> Self {
        let filtered_contract = contract.filter(|value| !value.is_empty());
        if let Some(contract) = filtered_contract {
            if self.identity.is_none() {
                let resolved_identity = contract.resolved_identity().cloned();
                self.identity = resolved_identity;
            }
            self.contract = Some(contract);
        }
        self
    }

    pub fn with_coordination(
        mut self,
        coordination: Vec<ConstrainedSubagentCoordinationAction>,
    ) -> Self {
        self.coordination = coordination;
        self
    }

    pub fn resolved_identity(&self) -> Option<&ConstrainedSubagentIdentity> {
        let explicit_identity = self.identity.as_ref();
        if explicit_identity.is_some() {
            return explicit_identity;
        }
        let contract = self.contract.as_ref()?;
        contract.resolved_identity()
    }

    pub fn resolved_profile(&self) -> Option<ConstrainedSubagentProfile> {
        let contract = self.contract.as_ref()?;
        contract.resolved_profile()
    }
}

pub fn subagent_surface_fields(subagent: Option<&ConstrainedSubagentHandle>) -> Map<String, Value> {
    let mut fields = Map::new();

    let subagent_identity = subagent
        .and_then(ConstrainedSubagentHandle::resolved_identity)
        .cloned();
    let subagent_profile = subagent.and_then(ConstrainedSubagentHandle::resolved_profile);
    let subagent_contract = subagent.and_then(|handle| handle.contract.clone());
    let subagent_value = subagent.map(|handle| json!(handle)).unwrap_or(Value::Null);
    let subagent_identity_value = subagent_identity
        .map(|identity| json!(identity))
        .unwrap_or(Value::Null);
    let subagent_profile_value = subagent_profile
        .map(|profile| json!(profile))
        .unwrap_or(Value::Null);
    let subagent_contract_value = subagent_contract
        .map(|contract| json!(contract))
        .unwrap_or(Value::Null);

    fields.insert("subagent_identity".to_owned(), subagent_identity_value);
    fields.insert("subagent_profile".to_owned(), subagent_profile_value);
    fields.insert("subagent_contract".to_owned(), subagent_contract_value);
    fields.insert("subagent".to_owned(), subagent_value);

    fields
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstrainedSubagentProfile {
    Orchestrator,
    Leaf,
}

impl ConstrainedSubagentProfile {
    pub fn for_child_depth(depth: usize, max_depth: usize) -> Self {
        let below_max_depth = depth < max_depth;
        if below_max_depth {
            return Self::Orchestrator;
        }
        Self::Leaf
    }

    pub fn allows_child_delegation(self) -> bool {
        matches!(self, Self::Orchestrator)
    }

    pub const fn role(self) -> ConstrainedSubagentRole {
        match self {
            Self::Orchestrator => ConstrainedSubagentRole::Orchestrator,
            Self::Leaf => ConstrainedSubagentRole::Leaf,
        }
    }

    pub const fn control_scope(self) -> ConstrainedSubagentControlScope {
        match self {
            Self::Orchestrator => ConstrainedSubagentControlScope::Children,
            Self::Leaf => ConstrainedSubagentControlScope::None,
        }
    }
}

impl Serialize for ConstrainedSubagentProfile {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ConstrainedSubagentProfile", 2)?;
        let role = self.role();
        let control_scope = self.control_scope();
        state.serialize_field("role", &role)?;
        state.serialize_field("control_scope", &control_scope)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ConstrainedSubagentProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ProfileRepr {
            LegacyString(String),
            Structured {
                role: ConstrainedSubagentRole,
                control_scope: ConstrainedSubagentControlScope,
            },
        }

        let profile_repr = ProfileRepr::deserialize(deserializer)?;
        match profile_repr {
            ProfileRepr::LegacyString(value) => match value.as_str() {
                "orchestrator" => Ok(ConstrainedSubagentProfile::Orchestrator),
                "leaf" => Ok(ConstrainedSubagentProfile::Leaf),
                other => Err(D::Error::custom(format!(
                    "unknown subagent profile `{other}`"
                ))),
            },
            ProfileRepr::Structured {
                role,
                control_scope,
            } => match (role, control_scope) {
                (
                    ConstrainedSubagentRole::Orchestrator,
                    ConstrainedSubagentControlScope::Children,
                ) => Ok(ConstrainedSubagentProfile::Orchestrator),
                (ConstrainedSubagentRole::Leaf, ConstrainedSubagentControlScope::None) => {
                    Ok(ConstrainedSubagentProfile::Leaf)
                }
                _ => Err(D::Error::custom(
                    "invalid subagent profile role/control_scope combination",
                )),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentCoordinationActionKind {
    InspectStatus,
    ReadHistory,
    ReadEvents,
    Wait,
    Cancel,
    Recover,
    Archive,
}

impl ConstrainedSubagentCoordinationActionKind {
    pub const fn tool_name(self) -> &'static str {
        match self {
            Self::InspectStatus => "session_status",
            Self::ReadHistory => "sessions_history",
            Self::ReadEvents => "session_events",
            Self::Wait => "session_wait",
            Self::Cancel => "session_cancel",
            Self::Recover => "session_recover",
            Self::Archive => "session_archive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentCoordinationAction {
    pub kind: ConstrainedSubagentCoordinationActionKind,
    pub tool_name: String,
}

impl ConstrainedSubagentCoordinationAction {
    pub fn from_kind(kind: ConstrainedSubagentCoordinationActionKind) -> Self {
        let tool_name = kind.tool_name().to_owned();
        Self { kind, tool_name }
    }
}

pub fn coordination_actions_for_subagent_handle(
    terminal: bool,
    phase: Option<&str>,
    mode: Option<ConstrainedSubagentMode>,
    overdue: bool,
) -> Vec<ConstrainedSubagentCoordinationAction> {
    let mut actions = vec![
        ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::InspectStatus,
        ),
        ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::ReadHistory,
        ),
        ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::ReadEvents,
        ),
    ];

    if terminal {
        let archive_action = ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::Archive,
        );
        actions.push(archive_action);
        return actions;
    }

    let wait_action = ConstrainedSubagentCoordinationAction::from_kind(
        ConstrainedSubagentCoordinationActionKind::Wait,
    );
    actions.push(wait_action);

    let async_mode = matches!(mode, Some(ConstrainedSubagentMode::Async));
    let can_cancel = async_mode && matches!(phase, Some("queued" | "running"));
    if can_cancel {
        let cancel_action = ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::Cancel,
        );
        actions.push(cancel_action);
    }

    if overdue {
        let recover_action = ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::Recover,
        );
        actions.push(recover_action);
    }

    actions
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConstrainedSubagentContractView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ConstrainedSubagentMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ConstrainedSubagentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ConstrainedSubagentProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<ConstrainedSubagentIsolation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_budget: Option<ConstrainedSubagentBudgetSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_child_budget: Option<ConstrainedSubagentBudgetSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_shell_in_child: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_tool_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "ToolRuntimeNarrowing::is_empty")]
    pub runtime_narrowing: ToolRuntimeNarrowing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_binding: Option<ConstrainedSubagentRuntimeBinding>,
}

impl ConstrainedSubagentContractView {
    pub fn from_execution(execution: &ConstrainedSubagentExecution) -> Self {
        let mode = Some(execution.mode);
        let identity = execution.identity.clone();
        let profile = Some(execution.resolved_profile());
        let isolation = Some(execution.isolation);
        let depth_budget = Some(ConstrainedSubagentBudgetSnapshot {
            current: execution.depth,
            max: execution.max_depth,
        });
        let active_child_budget = Some(ConstrainedSubagentBudgetSnapshot {
            current: execution.active_children,
            max: execution.max_active_children,
        });
        let timeout_seconds = Some(execution.timeout_seconds);
        let allow_shell_in_child = Some(execution.allow_shell_in_child);
        let child_tool_allowlist = execution.child_tool_allowlist.clone();
        let workspace_root = execution.workspace_root.clone();
        let runtime_narrowing = execution.runtime_narrowing.clone();
        let runtime_binding = if execution.kernel_bound {
            Some(ConstrainedSubagentRuntimeBinding::KernelBound)
        } else {
            Some(ConstrainedSubagentRuntimeBinding::Direct)
        };

        Self {
            mode,
            identity,
            profile,
            isolation,
            depth_budget,
            active_child_budget,
            timeout_seconds,
            allow_shell_in_child,
            child_tool_allowlist,
            workspace_root,
            runtime_narrowing,
            runtime_binding,
        }
    }

    pub fn from_profile(profile: ConstrainedSubagentProfile) -> Self {
        Self {
            profile: Some(profile),
            ..Self::default()
        }
    }

    pub fn from_depth_budget(depth: usize, max_depth: usize) -> Self {
        let mut contract = Self::default();
        let depth_budget = ConstrainedSubagentBudgetSnapshot {
            current: depth,
            max: max_depth,
        };
        contract.depth_budget = Some(depth_budget);
        contract
    }

    pub fn from_identity(identity: ConstrainedSubagentIdentity) -> Self {
        Self {
            identity: Some(identity),
            ..Self::default()
        }
    }

    pub fn from_runtime_narrowing(runtime_narrowing: ToolRuntimeNarrowing) -> Self {
        Self {
            runtime_narrowing,
            ..Self::default()
        }
    }

    pub fn with_profile(mut self, profile: ConstrainedSubagentProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    pub fn with_identity(mut self, identity: ConstrainedSubagentIdentity) -> Self {
        if !identity.is_empty() {
            self.identity = Some(identity);
        }
        self
    }

    pub fn with_runtime_narrowing(mut self, runtime_narrowing: ToolRuntimeNarrowing) -> Self {
        if !runtime_narrowing.is_empty() {
            self.runtime_narrowing = runtime_narrowing;
        }
        self
    }

    pub fn with_workspace_root(mut self, workspace_root: PathBuf) -> Self {
        self.workspace_root = Some(workspace_root);
        self
    }

    pub fn with_isolation(mut self, isolation: ConstrainedSubagentIsolation) -> Self {
        self.isolation = Some(isolation);
        self
    }

    pub fn with_depth_budget(mut self, depth: usize, max_depth: usize) -> Self {
        let depth_budget = ConstrainedSubagentBudgetSnapshot {
            current: depth,
            max: max_depth,
        };
        self.depth_budget = Some(depth_budget);
        self
    }

    pub fn resolved_profile(&self) -> Option<ConstrainedSubagentProfile> {
        self.profile
    }

    pub fn resolved_identity(&self) -> Option<&ConstrainedSubagentIdentity> {
        self.identity.as_ref()
    }

    pub fn allows_child_delegation(&self) -> bool {
        if let Some(profile) = self.profile {
            return profile.allows_child_delegation();
        }
        let depth_budget = self.depth_budget;
        let Some(depth_budget) = depth_budget else {
            return false;
        };
        depth_budget.current < depth_budget.max
    }

    pub fn is_empty(&self) -> bool {
        let mode_missing = self.mode.is_none();
        let identity_missing = self.identity.is_none();
        let profile_missing = self.profile.is_none();
        let isolation_missing = self.isolation.is_none();
        let depth_budget_missing = self.depth_budget.is_none();
        let active_child_budget_missing = self.active_child_budget.is_none();
        let timeout_missing = self.timeout_seconds.is_none();
        let shell_missing = self.allow_shell_in_child.is_none();
        let allowlist_empty = self.child_tool_allowlist.is_empty();
        let workspace_root_missing = self.workspace_root.is_none();
        let narrowing_empty = self.runtime_narrowing.is_empty();
        let binding_missing = self.runtime_binding.is_none();

        mode_missing
            && identity_missing
            && profile_missing
            && isolation_missing
            && depth_budget_missing
            && active_child_budget_missing
            && timeout_missing
            && shell_missing
            && allowlist_empty
            && workspace_root_missing
            && narrowing_empty
            && binding_missing
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentExecution {
    pub mode: ConstrainedSubagentMode,
    #[serde(default)]
    pub isolation: ConstrainedSubagentIsolation,
    pub depth: usize,
    pub max_depth: usize,
    pub active_children: usize,
    pub max_active_children: usize,
    pub timeout_seconds: u64,
    pub allow_shell_in_child: bool,
    pub child_tool_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "ToolRuntimeNarrowing::is_empty")]
    pub runtime_narrowing: ToolRuntimeNarrowing,
    pub kernel_bound: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ConstrainedSubagentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ConstrainedSubagentProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentSpawnEventPayload {
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<DelegateBuiltinProfile>,
    pub execution: ConstrainedSubagentExecution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentTerminalEventPayload {
    pub terminal_reason: ConstrainedSubagentTerminalReason,
    pub execution: ConstrainedSubagentExecution,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ConstrainedSubagentExecution {
    pub fn resolved_profile(&self) -> ConstrainedSubagentProfile {
        let explicit_profile = self.profile;
        if let Some(profile) = explicit_profile {
            return profile;
        }
        ConstrainedSubagentProfile::for_child_depth(self.depth, self.max_depth)
    }

    pub fn with_resolved_profile(mut self) -> Self {
        if self.profile.is_none() {
            self.profile = Some(self.resolved_profile());
        }
        self
    }

    pub fn allows_nested_delegate_children(&self) -> bool {
        let resolved_profile = self.resolved_profile();
        let can_delegate = resolved_profile.allows_child_delegation();
        can_delegate && self.depth < self.max_depth
    }

    pub fn contract_view(&self) -> ConstrainedSubagentContractView {
        ConstrainedSubagentContractView::from_execution(self)
    }

    pub fn spawn_payload(&self, task: &str, label: Option<&str>) -> Value {
        self.spawn_payload_with_profile_and_runtime_self_continuity(task, label, None, None)
    }

    pub fn spawn_payload_with_profile(
        &self,
        task: &str,
        label: Option<&str>,
        profile: Option<DelegateBuiltinProfile>,
    ) -> Value {
        self.spawn_payload_with_profile_and_runtime_self_continuity(task, label, profile, None)
    }

    pub(crate) fn spawn_payload_with_profile_and_runtime_self_continuity(
        &self,
        task: &str,
        label: Option<&str>,
        profile: Option<DelegateBuiltinProfile>,
        runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    ) -> Value {
        let task = task.to_owned();
        let label = label.map(ToOwned::to_owned);
        let runtime_self_continuity = runtime_self_continuity.cloned();
        json!(ConstrainedSubagentSpawnEventPayload {
            task,
            label,
            profile,
            execution: self.clone(),
            runtime_self_continuity,
        })
    }

    pub fn terminal_payload(
        &self,
        terminal_reason: ConstrainedSubagentTerminalReason,
        duration_ms: u64,
        turn_count: Option<usize>,
        error: Option<&str>,
    ) -> Value {
        let error = error.map(ToOwned::to_owned);
        json!(ConstrainedSubagentTerminalEventPayload {
            terminal_reason,
            execution: self.clone(),
            duration_ms,
            turn_count,
            error,
        })
    }

    pub fn from_event_payload(payload: &Value) -> Option<Self> {
        let execution = payload.get("execution")?;
        let execution = execution.clone();
        serde_json::from_value(execution).ok()
    }

    pub fn profile_from_event_payload(payload: &Value) -> Option<DelegateBuiltinProfile> {
        let profile = payload.get("profile")?;
        let profile = profile.clone();
        serde_json::from_value(profile).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_identity::{ResolvedRuntimeIdentity, RuntimeIdentitySource};
    use crate::runtime_self::RuntimeSelfModel;
    use crate::runtime_self_continuity::RuntimeSelfContinuity;

    #[test]
    fn constrained_subagent_execution_round_trips_event_payload() {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Async,
            isolation: ConstrainedSubagentIsolation::Shared,
            depth: 1,
            max_depth: 2,
            active_children: 0,
            max_active_children: 3,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec![
                "file.read".to_owned(),
                "file.write".to_owned(),
                "file.edit".to_owned(),
            ],
            workspace_root: Some(PathBuf::from("/tmp/child-workspace")),
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: true,
            identity: None,
            profile: Some(ConstrainedSubagentProfile::for_child_depth(1, 2)),
        };

        let payload = execution.spawn_payload_with_profile(
            "research",
            Some("child"),
            Some(DelegateBuiltinProfile::Research),
        );
        let restored_execution = ConstrainedSubagentExecution::from_event_payload(&payload);
        let restored_profile = ConstrainedSubagentExecution::profile_from_event_payload(&payload);

        assert_eq!(restored_execution, Some(execution));
        assert_eq!(restored_profile, Some(DelegateBuiltinProfile::Research));
    }

    #[test]
    fn constrained_subagent_execution_preserves_runtime_self_continuity_in_spawn_payload() {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Inline,
            isolation: ConstrainedSubagentIsolation::Shared,
            depth: 1,
            max_depth: 2,
            active_children: 0,
            max_active_children: 2,
            timeout_seconds: 30,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["web.fetch".to_owned()],
            workspace_root: None,
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: None,
            profile: Some(ConstrainedSubagentProfile::for_child_depth(1, 2)),
        };
        let continuity = RuntimeSelfContinuity {
            runtime_self: RuntimeSelfModel {
                standing_instructions: vec!["Keep continuity explicit.".to_owned()],
                tool_usage_policy: vec!["Search memory before guessing workspace facts.".to_owned()],
                soul_guidance: vec!["Prefer rigorous execution.".to_owned()],
                identity_context: vec!["# Identity\n- Name: Child".to_owned()],
                user_context: vec!["User prefers concise output.".to_owned()],
            },
            resolved_identity: Some(ResolvedRuntimeIdentity {
                source: RuntimeIdentitySource::WorkspaceSelf,
                content: "# Identity\n- Name: Child".to_owned(),
            }),
            session_profile_projection: Some(
                "## Session Profile\nDurable preferences and advisory session context carried into this session:\nUser prefers concise output.".to_owned(),
            ),
        };

        let payload = execution.spawn_payload_with_profile_and_runtime_self_continuity(
            "research",
            Some("child"),
            Some(DelegateBuiltinProfile::Plan),
            Some(&continuity),
        );

        assert_eq!(
            payload["runtime_self_continuity"]["resolved_identity"]["content"],
            continuity
                .resolved_identity
                .as_ref()
                .expect("resolved identity")
                .content
        );
        assert_eq!(
            payload["runtime_self_continuity"]["runtime_self"]["tool_usage_policy"][0],
            "Search memory before guessing workspace facts."
        );
    }

    #[test]
    fn contract_view_copies_execution_profile_and_workspace_root() {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Async,
            isolation: ConstrainedSubagentIsolation::Worktree,
            depth: 1,
            max_depth: 3,
            active_children: 0,
            max_active_children: 4,
            timeout_seconds: 90,
            allow_shell_in_child: true,
            child_tool_allowlist: vec!["file.read".to_owned(), "web.fetch".to_owned()],
            workspace_root: Some(PathBuf::from("/tmp/delegate-worktree")),
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: Some(ConstrainedSubagentIdentity {
                nickname: Some("child".to_owned()),
                specialization: Some("reviewer".to_owned()),
            }),
            profile: Some(ConstrainedSubagentProfile::for_child_depth(1, 3)),
        };

        let contract = execution.contract_view();

        assert_eq!(
            contract.profile,
            Some(ConstrainedSubagentProfile::Orchestrator)
        );
        assert_eq!(
            contract.isolation,
            Some(ConstrainedSubagentIsolation::Worktree)
        );
        assert_eq!(
            contract.workspace_root,
            Some(PathBuf::from("/tmp/delegate-worktree"))
        );
        assert!(contract.allows_child_delegation());
    }
}
