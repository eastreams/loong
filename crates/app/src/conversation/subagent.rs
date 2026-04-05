use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::runtime_config::ToolRuntimeNarrowing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedSubagentMode {
    Inline,
    Async,
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
        self.nickname.is_none() && self.specialization.is_none()
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
        Self {
            session_id: session_id.into(),
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
        if let Some(identity) = identity.filter(|identity| !identity.is_empty()) {
            self.identity = Some(identity);
        }
        self
    }

    pub fn with_contract(mut self, contract: Option<ConstrainedSubagentContractView>) -> Self {
        if let Some(contract) = contract.filter(|contract| !contract.is_empty()) {
            if self.identity.is_none() {
                self.identity = contract.resolved_identity().cloned();
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
        self.identity.as_ref().or_else(|| {
            self.contract
                .as_ref()
                .and_then(ConstrainedSubagentContractView::resolved_identity)
        })
    }

    pub fn resolved_profile(&self) -> Option<ConstrainedSubagentProfile> {
        self.contract
            .as_ref()
            .and_then(ConstrainedSubagentContractView::resolved_profile)
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
        Self {
            kind,
            tool_name: kind.tool_name().to_owned(),
        }
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
        actions.push(ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::Archive,
        ));
        return actions;
    }

    actions.push(ConstrainedSubagentCoordinationAction::from_kind(
        ConstrainedSubagentCoordinationActionKind::Wait,
    ));

    let async_mode = matches!(mode, Some(ConstrainedSubagentMode::Async));
    let can_cancel = async_mode && matches!(phase, Some("queued" | "running"));
    if can_cancel {
        actions.push(ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::Cancel,
        ));
    }

    if overdue {
        actions.push(ConstrainedSubagentCoordinationAction::from_kind(
            ConstrainedSubagentCoordinationActionKind::Recover,
        ));
    }

    actions
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentProfile {
    pub role: ConstrainedSubagentRole,
    pub control_scope: ConstrainedSubagentControlScope,
}

impl ConstrainedSubagentProfile {
    pub fn for_child_depth(depth: usize, max_depth: usize) -> Self {
        if depth < max_depth {
            Self {
                role: ConstrainedSubagentRole::Orchestrator,
                control_scope: ConstrainedSubagentControlScope::Children,
            }
        } else {
            Self {
                role: ConstrainedSubagentRole::Leaf,
                control_scope: ConstrainedSubagentControlScope::None,
            }
        }
    }

    pub fn allows_child_delegation(self) -> bool {
        self.control_scope == ConstrainedSubagentControlScope::Children
    }
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
    pub depth_budget: Option<ConstrainedSubagentBudgetSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_child_budget: Option<ConstrainedSubagentBudgetSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_shell_in_child: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_tool_allowlist: Vec<String>,
    #[serde(default, skip_serializing_if = "ToolRuntimeNarrowing::is_empty")]
    pub runtime_narrowing: ToolRuntimeNarrowing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_binding: Option<ConstrainedSubagentRuntimeBinding>,
}

impl ConstrainedSubagentContractView {
    pub fn from_execution(execution: &ConstrainedSubagentExecution) -> Self {
        Self {
            mode: Some(execution.mode),
            identity: execution.identity.clone(),
            profile: Some(execution.resolved_profile()),
            depth_budget: Some(ConstrainedSubagentBudgetSnapshot {
                current: execution.depth,
                max: execution.max_depth,
            }),
            active_child_budget: Some(ConstrainedSubagentBudgetSnapshot {
                current: execution.active_children,
                max: execution.max_active_children,
            }),
            timeout_seconds: Some(execution.timeout_seconds),
            allow_shell_in_child: Some(execution.allow_shell_in_child),
            child_tool_allowlist: execution.child_tool_allowlist.clone(),
            runtime_narrowing: execution.runtime_narrowing.clone(),
            runtime_binding: Some(if execution.kernel_bound {
                ConstrainedSubagentRuntimeBinding::KernelBound
            } else {
                ConstrainedSubagentRuntimeBinding::Direct
            }),
        }
    }

    pub fn from_profile(profile: ConstrainedSubagentProfile) -> Self {
        Self {
            profile: Some(profile),
            ..Self::default()
        }
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

    pub fn resolved_profile(&self) -> Option<ConstrainedSubagentProfile> {
        self.profile
    }

    pub fn resolved_identity(&self) -> Option<&ConstrainedSubagentIdentity> {
        self.identity.as_ref()
    }

    pub fn allows_child_delegation(&self) -> bool {
        self.profile
            .map(ConstrainedSubagentProfile::allows_child_delegation)
            .unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.identity.is_none()
            && self.profile.is_none()
            && self.depth_budget.is_none()
            && self.active_child_budget.is_none()
            && self.timeout_seconds.is_none()
            && self.allow_shell_in_child.is_none()
            && self.child_tool_allowlist.is_empty()
            && self.runtime_narrowing.is_empty()
            && self.runtime_binding.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstrainedSubagentExecution {
    pub mode: ConstrainedSubagentMode,
    pub depth: usize,
    pub max_depth: usize,
    pub active_children: usize,
    pub max_active_children: usize,
    pub timeout_seconds: u64,
    pub allow_shell_in_child: bool,
    pub child_tool_allowlist: Vec<String>,
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
        self.profile.unwrap_or_else(|| {
            ConstrainedSubagentProfile::for_child_depth(self.depth, self.max_depth)
        })
    }

    pub fn with_resolved_profile(mut self) -> Self {
        if self.profile.is_none() {
            self.profile = Some(self.resolved_profile());
        }
        self
    }

    pub fn allows_nested_delegate_children(&self) -> bool {
        self.resolved_profile().allows_child_delegation() && self.depth < self.max_depth
    }

    pub fn contract_view(&self) -> ConstrainedSubagentContractView {
        ConstrainedSubagentContractView::from_execution(self)
    }

    pub fn spawn_payload(&self, task: &str, label: Option<&str>) -> Value {
        self.spawn_payload_with_runtime_self_continuity(task, label, None)
    }

    pub(crate) fn spawn_payload_with_runtime_self_continuity(
        &self,
        task: &str,
        label: Option<&str>,
        runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    ) -> Value {
        json!(ConstrainedSubagentSpawnEventPayload {
            task: task.to_owned(),
            label: label.map(ToOwned::to_owned),
            execution: self.clone(),
            runtime_self_continuity: runtime_self_continuity.cloned(),
        })
    }

    pub fn terminal_payload(
        &self,
        terminal_reason: ConstrainedSubagentTerminalReason,
        duration_ms: u64,
        turn_count: Option<usize>,
        error: Option<&str>,
    ) -> Value {
        json!(ConstrainedSubagentTerminalEventPayload {
            terminal_reason,
            execution: self.clone(),
            duration_ms,
            turn_count,
            error: error.map(ToOwned::to_owned),
        })
    }

    pub fn from_event_payload(payload: &Value) -> Option<Self> {
        let execution = payload.get("execution")?.clone();
        serde_json::from_value(execution).ok()
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
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: true,
            identity: None,
            profile: Some(ConstrainedSubagentProfile::for_child_depth(1, 2)),
        };

        let payload = execution.spawn_payload("research", Some("child"));
        assert_eq!(
            ConstrainedSubagentExecution::from_event_payload(&payload),
            Some(execution)
        );
    }

    #[test]
    fn constrained_subagent_execution_preserves_runtime_self_continuity_in_spawn_payload() {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Inline,
            depth: 1,
            max_depth: 2,
            active_children: 0,
            max_active_children: 2,
            timeout_seconds: 30,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["web.fetch".to_owned()],
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

        let payload = execution.spawn_payload_with_runtime_self_continuity(
            "research",
            Some("child"),
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
    fn constrained_subagent_execution_derives_legacy_profile_from_depth_budget() {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Async,
            depth: 1,
            max_depth: 3,
            active_children: 0,
            max_active_children: 3,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["file.read".to_owned()],
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: None,
            profile: None,
        };

        assert_eq!(
            execution.resolved_profile(),
            ConstrainedSubagentProfile {
                role: ConstrainedSubagentRole::Orchestrator,
                control_scope: ConstrainedSubagentControlScope::Children,
            }
        );
        assert!(execution.allows_nested_delegate_children());
    }

    #[test]
    fn constrained_subagent_execution_contract_view_normalizes_execution_semantics() {
        let runtime_narrowing = ToolRuntimeNarrowing {
            web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
                allow_private_hosts: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Inline,
            depth: 2,
            max_depth: 3,
            active_children: 1,
            max_active_children: 4,
            timeout_seconds: 45,
            allow_shell_in_child: true,
            child_tool_allowlist: vec!["file.read".to_owned(), "shell.exec".to_owned()],
            runtime_narrowing: runtime_narrowing.clone(),
            kernel_bound: true,
            identity: Some(ConstrainedSubagentIdentity {
                nickname: Some("child-researcher".to_owned()),
                specialization: Some("reviewer".to_owned()),
            }),
            profile: Some(ConstrainedSubagentProfile::for_child_depth(2, 3)),
        };

        assert_eq!(
            execution.contract_view(),
            ConstrainedSubagentContractView {
                mode: Some(ConstrainedSubagentMode::Inline),
                identity: Some(ConstrainedSubagentIdentity {
                    nickname: Some("child-researcher".to_owned()),
                    specialization: Some("reviewer".to_owned()),
                }),
                profile: Some(ConstrainedSubagentProfile::for_child_depth(2, 3)),
                depth_budget: Some(ConstrainedSubagentBudgetSnapshot { current: 2, max: 3 }),
                active_child_budget: Some(ConstrainedSubagentBudgetSnapshot { current: 1, max: 4 }),
                timeout_seconds: Some(45),
                allow_shell_in_child: Some(true),
                child_tool_allowlist: vec!["file.read".to_owned(), "shell.exec".to_owned()],
                runtime_narrowing,
                runtime_binding: Some(ConstrainedSubagentRuntimeBinding::KernelBound),
            }
        );
    }

    #[test]
    fn subagent_surface_fields_derive_identity_and_profile_from_handle_contract() {
        let handle = ConstrainedSubagentHandle::new("child-session").with_contract(Some(
            ConstrainedSubagentContractView::from_identity(ConstrainedSubagentIdentity {
                nickname: Some("child".to_owned()),
                specialization: Some("reviewer".to_owned()),
            })
            .with_profile(ConstrainedSubagentProfile {
                role: ConstrainedSubagentRole::Leaf,
                control_scope: ConstrainedSubagentControlScope::None,
            }),
        ));
        let fields = subagent_surface_fields(Some(&handle));

        assert_eq!(fields["subagent_identity"]["nickname"], "child");
        assert_eq!(fields["subagent_identity"]["specialization"], "reviewer");
        assert_eq!(fields["subagent_profile"]["role"], "leaf");
        assert_eq!(fields["subagent"]["session_id"], "child-session");
    }
}
