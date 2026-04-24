use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedSessionMode {
    AdvisoryOnly,
    MutatingCapable,
}

impl GovernedSessionMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdvisoryOnly => "advisory_only",
            Self::MutatingCapable => "mutating_capable",
        }
    }
}

impl fmt::Display for GovernedSessionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOperationKind {
    Plan,
    Task,
    Worktree,
    Approval,
}

impl WorkflowOperationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Task => "task",
            Self::Worktree => "worktree",
            Self::Approval => "approval",
        }
    }
}

impl fmt::Display for WorkflowOperationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOperationScope {
    Session,
    Task,
    Worktree,
    Approval,
}

impl WorkflowOperationScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Task => "task",
            Self::Worktree => "worktree",
            Self::Approval => "approval",
        }
    }
}

impl fmt::Display for WorkflowOperationScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedWorkflowPhase {
    Plan,
    Spec,
    Execute,
    Verify,
    Fix,
    Complete,
    Failed,
    Cancelled,
}

impl GovernedWorkflowPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Spec => "spec",
            Self::Execute => "execute",
            Self::Verify => "verify",
            Self::Fix => "fix",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for GovernedWorkflowPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskScopeDescriptor {
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeBindingDescriptor {
    pub worktree_id: String,
    pub workspace_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernedSessionBindingDescriptor {
    pub session_id: String,
    pub task_scope: TaskScopeDescriptor,
    pub task_session_id: String,
    pub turn_id: String,
    pub worktree: WorktreeBindingDescriptor,
    pub policy_snapshot: String,
    pub audit_correlation_id: String,
    pub execution_surface: String,
    pub mode: GovernedSessionMode,
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WorkflowOperationHolder {
        #[serde(rename = "kind")]
        _kind: WorkflowOperationKind,
    }

    #[test]
    fn governed_workflow_contract_governed_session_binding_serializes_with_stable_shape() {
        let descriptor = GovernedSessionBindingDescriptor {
            session_id: "session-001".to_owned(),
            task_scope: TaskScopeDescriptor {
                task_id: "task-001".to_owned(),
            },
            task_session_id: "task-session-001".to_owned(),
            turn_id: "turn-001".to_owned(),
            worktree: WorktreeBindingDescriptor {
                worktree_id: "worktree-001".to_owned(),
                workspace_root: "/repo/.worktrees/worktree-001".to_owned(),
            },
            policy_snapshot: "policy-snapshot-001".to_owned(),
            audit_correlation_id: "audit-001".to_owned(),
            execution_surface: "conversation_turn".to_owned(),
            mode: GovernedSessionMode::MutatingCapable,
        };

        let serialized = serde_json::to_value(&descriptor).expect("binding descriptor serializes");

        assert_eq!(
            serialized,
            json!({
                "session_id": "session-001",
                "task_scope": {
                    "task_id": "task-001",
                },
                "task_session_id": "task-session-001",
                "turn_id": "turn-001",
                "worktree": {
                    "worktree_id": "worktree-001",
                    "workspace_root": "/repo/.worktrees/worktree-001",
                },
                "policy_snapshot": "policy-snapshot-001",
                "audit_correlation_id": "audit-001",
                "execution_surface": "conversation_turn",
                "mode": "mutating_capable",
            })
        );
    }

    #[test]
    fn governed_workflow_contract_session_modes_are_distinguishable() {
        let advisory =
            serde_json::to_value(GovernedSessionMode::AdvisoryOnly).expect("advisory serializes");
        let mutating = serde_json::to_value(GovernedSessionMode::MutatingCapable)
            .expect("mutating serializes");

        assert_eq!(advisory, json!("advisory_only"));
        assert_eq!(mutating, json!("mutating_capable"));
        assert_ne!(advisory, mutating);
    }

    #[test]
    fn governed_workflow_contract_binding_keeps_task_identity_distinct_from_session_identity() {
        let binding = serde_json::from_value::<GovernedSessionBindingDescriptor>(json!({
            "session_id": "binding-session-001",
            "task_scope": {
                "task_id": "task-001",
            },
            "task_session_id": "task-session-001",
            "turn_id": "turn-001",
            "worktree": {
                "worktree_id": "worktree-001",
                "workspace_root": "/repo/.worktrees/worktree-001",
            },
            "policy_snapshot": "policy-snapshot-001",
            "audit_correlation_id": "audit-001",
            "execution_surface": "conversation_turn",
            "mode": "mutating_capable",
        }))
        .expect("binding parses");

        assert_eq!(binding.task_scope.task_id, "task-001");
        assert_eq!(binding.task_session_id, "task-session-001");
        assert_eq!(binding.session_id, "binding-session-001");
        assert_ne!(binding.task_scope.task_id, binding.task_session_id);
        assert_ne!(binding.task_scope.task_id, binding.session_id);
        assert_ne!(binding.task_session_id, binding.session_id);
    }

    #[test]
    fn governed_workflow_contract_operation_kind_and_scope_serialize_deterministically() {
        let kind = serde_json::to_value(WorkflowOperationKind::Worktree).expect("kind serializes");
        let scope =
            serde_json::to_value(WorkflowOperationScope::Worktree).expect("scope serializes");
        let phase = serde_json::to_value(GovernedWorkflowPhase::Execute).expect("phase serializes");

        assert_eq!(kind, json!("worktree"));
        assert_eq!(scope, json!("worktree"));
        assert_eq!(phase, json!("execute"));
    }

    #[test]
    fn governed_workflow_contract_operation_kind_rejects_missing_or_unknown_values() {
        let missing_kind = serde_json::from_value::<WorkflowOperationHolder>(json!({}))
            .expect_err("missing kind should fail closed");
        assert!(missing_kind.to_string().contains("missing field `kind`"));

        let unknown_kind = serde_json::from_value::<WorkflowOperationHolder>(json!({
            "kind": "shell",
        }))
        .expect_err("unknown kind should fail closed");
        assert!(unknown_kind.to_string().contains("unknown variant"));
    }

    #[test]
    fn governed_workflow_contract_binding_rejects_missing_required_fields() {
        let error = serde_json::from_value::<GovernedSessionBindingDescriptor>(json!({
            "session_id": "session-001",
            "task_scope": {
                "task_id": "task-001",
            },
            "task_session_id": "task-session-001",
            "turn_id": "turn-001",
            "worktree": {
                "worktree_id": "worktree-001",
                "workspace_root": "/repo/.worktrees/worktree-001",
            },
            "policy_snapshot": "policy-snapshot-001",
            "audit_correlation_id": "audit-001",
            "execution_surface": "conversation_turn"
        }))
        .expect_err("missing mode should fail closed");

        assert!(error.to_string().contains("missing field `mode`"));
    }

    #[test]
    fn governed_workflow_contract_binding_requires_explicit_task_session_mapping() {
        let error = serde_json::from_value::<GovernedSessionBindingDescriptor>(json!({
            "session_id": "session-001",
            "task_scope": {
                "task_id": "task-001",
            },
            "turn_id": "turn-001",
            "worktree": {
                "worktree_id": "worktree-001",
                "workspace_root": "/repo/.worktrees/worktree-001",
            },
            "policy_snapshot": "policy-snapshot-001",
            "audit_correlation_id": "audit-001",
            "execution_surface": "conversation_turn",
            "mode": "mutating_capable"
        }))
        .expect_err("missing task_session_id should fail closed");

        assert!(
            error
                .to_string()
                .contains("missing field `task_session_id`")
        );
    }
}
