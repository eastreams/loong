use serde::{Deserialize, Serialize};

use crate::{
    ArtifactDurabilityClass, CancellationPolicy, ChildBudgetPolicy, ExecutionArtifactKind,
    TaskExecutionMode, TaskLifecycle, TurnStatus, WorkspaceContext,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEvent {
    SessionCreated {
        session_id: String,
        workspace: WorkspaceContext,
    },
    ArtifactRecorded {
        session_id: String,
        artifact_id: String,
        kind: ExecutionArtifactKind,
        durability: ArtifactDurabilityClass,
    },
    WorktreeSwitched {
        session_id: String,
        previous_workspace: WorkspaceContext,
        next_workspace: WorkspaceContext,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskEvent {
    TaskCreated {
        task_id: String,
        objective: String,
        execution_mode: TaskExecutionMode,
    },
    WorkspaceBound {
        task_id: String,
        workspace: WorkspaceContext,
        inherited: bool,
    },
    LifecycleChanged {
        task_id: String,
        from: TaskLifecycle,
        to: TaskLifecycle,
    },
    TurnUpdated {
        task_id: String,
        turn_id: String,
        status: TurnStatus,
    },
    ArtifactRecorded {
        task_id: String,
        artifact_id: String,
        kind: ExecutionArtifactKind,
        durability: ArtifactDurabilityClass,
    },
    SubtaskRegistered {
        parent_task_id: String,
        child_task_id: String,
        budget_policy: ChildBudgetPolicy,
        cancellation: CancellationPolicy,
        inherited_workspace: bool,
    },
}

impl TaskEvent {
    pub fn task_id(&self) -> &str {
        match self {
            Self::TaskCreated { task_id, .. }
            | Self::WorkspaceBound { task_id, .. }
            | Self::LifecycleChanged { task_id, .. }
            | Self::TurnUpdated { task_id, .. }
            | Self::ArtifactRecorded { task_id, .. } => task_id,
            Self::SubtaskRegistered { parent_task_id, .. } => parent_task_id,
        }
    }
}
