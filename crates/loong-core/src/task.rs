use serde::{Deserialize, Serialize};

use crate::{
    CancellationPolicy, ChildBudgetPolicy, CoreModelError, ExecutionArtifact, ExecutionArtifacts,
    SessionBudgetOverlay, Subtask, TaskBudget, TaskEvent, TaskExecutionMode, TaskLifecycle, Turn,
    TurnStatus, WorkspaceContext,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub task_id: String,
    pub parent_task_id: Option<String>,
    pub objective: String,
    lifecycle: TaskLifecycle,
    pub execution_mode: TaskExecutionMode,
    workspace: WorkspaceContext,
    budget: TaskBudget,
    turns: Vec<Turn>,
    subtasks: Vec<Subtask>,
    artifacts: ExecutionArtifacts,
    events: Vec<TaskEvent>,
}

impl Task {
    fn push_event(&mut self, event: TaskEvent) {
        self.events.push(event);
        self.enforce_event_retention();
    }

    fn enforce_event_retention(&mut self) {
        let Some(max_records) = self.budget.event_retention.max_records else {
            return;
        };
        if self.events.len() <= max_records {
            return;
        }
        let drop_count = self.events.len() - max_records;
        self.events.drain(0..drop_count);
    }

    pub fn new(
        task_id: impl Into<String>,
        objective: impl Into<String>,
        workspace: WorkspaceContext,
        budget: TaskBudget,
        execution_mode: TaskExecutionMode,
    ) -> Self {
        Self::new_with_parent(
            task_id,
            None,
            objective,
            workspace,
            budget,
            execution_mode,
            false,
        )
    }

    fn new_with_parent(
        task_id: impl Into<String>,
        parent_task_id: Option<String>,
        objective: impl Into<String>,
        workspace: WorkspaceContext,
        budget: TaskBudget,
        execution_mode: TaskExecutionMode,
        inherited_workspace: bool,
    ) -> Self {
        let task_id = task_id.into();
        let objective = objective.into();
        let events = vec![
            TaskEvent::TaskCreated {
                task_id: task_id.clone(),
                objective: objective.clone(),
                execution_mode: execution_mode.clone(),
            },
            TaskEvent::WorkspaceBound {
                task_id: task_id.clone(),
                workspace: workspace.clone(),
                inherited: inherited_workspace,
            },
        ];

        let mut task = Self {
            task_id,
            parent_task_id,
            objective,
            lifecycle: TaskLifecycle::Queued,
            execution_mode,
            workspace,
            budget,
            turns: Vec::new(),
            subtasks: Vec::new(),
            artifacts: ExecutionArtifacts::default(),
            events,
        };
        task.enforce_event_retention();
        task
    }

    pub fn lifecycle(&self) -> &TaskLifecycle {
        &self.lifecycle
    }

    pub fn workspace(&self) -> &WorkspaceContext {
        &self.workspace
    }

    pub fn budget(&self) -> &TaskBudget {
        &self.budget
    }

    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    pub fn current_turn(&self) -> Option<&Turn> {
        self.turns.last()
    }

    pub fn subtasks(&self) -> &[Subtask] {
        &self.subtasks
    }

    pub fn artifacts(&self) -> &ExecutionArtifacts {
        &self.artifacts
    }

    pub fn fact_events(&self) -> &[TaskEvent] {
        &self.events
    }

    pub fn session_overlay_scope<'a>(
        &self,
        overlay: &'a SessionBudgetOverlay,
    ) -> &'a SessionBudgetOverlay {
        overlay
    }

    pub fn transition_to(&mut self, next: TaskLifecycle) -> Result<TaskEvent, CoreModelError> {
        if !self.lifecycle.can_transition_to(&next) {
            return Err(CoreModelError::InvalidLifecycleTransition {
                from: self.lifecycle.clone(),
                to: next,
            });
        }

        let event = TaskEvent::LifecycleChanged {
            task_id: self.task_id.clone(),
            from: self.lifecycle.clone(),
            to: next.clone(),
        };
        self.lifecycle = next;
        self.push_event(event.clone());
        Ok(event)
    }

    pub fn begin_turn(
        &mut self,
        turn_id: impl Into<String>,
        summary: impl Into<String>,
    ) -> Result<TaskEvent, CoreModelError> {
        if self
            .current_turn()
            .is_some_and(|turn| !turn.status.is_terminal())
        {
            return Err(CoreModelError::ActiveTurnAlreadyOpen {
                task_id: self.task_id.clone(),
            });
        }

        let turn = Turn::new(turn_id, (self.turns.len() as u32) + 1, summary);
        let event = TaskEvent::TurnUpdated {
            task_id: self.task_id.clone(),
            turn_id: turn.turn_id.clone(),
            status: turn.status.clone(),
        };
        self.turns.push(turn);
        self.push_event(event.clone());
        Ok(event)
    }

    pub fn finish_current_turn(&mut self, status: TurnStatus) -> Option<TaskEvent> {
        let turn = self.turns.last_mut()?;
        turn.status = status.clone();
        let event = TaskEvent::TurnUpdated {
            task_id: self.task_id.clone(),
            turn_id: turn.turn_id.clone(),
            status,
        };
        self.push_event(event.clone());
        Some(event)
    }

    pub fn record_artifact(&mut self, artifact: ExecutionArtifact) -> TaskEvent {
        let event = TaskEvent::ArtifactRecorded {
            task_id: self.task_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            kind: artifact.kind.clone(),
            durability: artifact.durability.clone(),
        };
        self.artifacts
            .record_bounded(artifact, self.budget.artifact_retention.max_records);
        self.push_event(event.clone());
        event
    }

    pub fn spawn_subtask(
        &mut self,
        child_task_id: impl Into<String>,
        objective: impl Into<String>,
        execution_mode: TaskExecutionMode,
        budget_policy: ChildBudgetPolicy,
        workspace_override: Option<WorkspaceContext>,
    ) -> Task {
        let child_task_id = child_task_id.into();
        let inherited_workspace = workspace_override.is_none();
        let workspace = workspace_override.unwrap_or_else(|| self.workspace.clone());
        let lineage = Subtask {
            parent_task_id: self.task_id.clone(),
            task_id: child_task_id.clone(),
            budget_policy: budget_policy.clone(),
            cancellation: CancellationPolicy::CascadeFromParent,
        };
        self.subtasks.push(lineage);
        self.push_event(TaskEvent::SubtaskRegistered {
            parent_task_id: self.task_id.clone(),
            child_task_id: child_task_id.clone(),
            budget_policy: budget_policy.clone(),
            cancellation: CancellationPolicy::CascadeFromParent,
            inherited_workspace,
        });
        Task::new_with_parent(
            child_task_id,
            Some(self.task_id.clone()),
            objective,
            workspace,
            budget_policy.resolve(&self.budget),
            execution_mode,
            inherited_workspace,
        )
    }
}
