use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    ChildBudgetPolicy, CoreModelError, ExecutionArtifact, ExecutionArtifacts, SessionBudgetOverlay,
    SessionEvent, Task, TaskBudget, TaskEvent, TaskExecutionMode, WorkspaceContext,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    workspace: WorkspaceContext,
    pub overlay_budget: SessionBudgetOverlay,
    tasks: BTreeMap<String, Task>,
    artifacts: ExecutionArtifacts,
    events: Vec<SessionEvent>,
}

impl Session {
    fn push_event(&mut self, event: SessionEvent) {
        self.events.push(event);
        self.enforce_event_retention();
    }

    fn enforce_event_retention(&mut self) {
        let Some(max_records) = self.overlay_budget.max_total_events else {
            return;
        };
        if self.events.len() <= max_records {
            return;
        }
        let drop_count = self.events.len() - max_records;
        self.events.drain(0..drop_count);
    }

    fn record_shared_artifact_bounded(&mut self, artifact: ExecutionArtifact) {
        self.artifacts
            .record_bounded(artifact, self.overlay_budget.max_total_artifacts);
    }

    fn active_task_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| !task.lifecycle().is_terminal())
            .count()
    }

    fn active_child_task_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| task.parent_task_id.is_some() && !task.lifecycle().is_terminal())
            .count()
    }

    fn active_child_task_count_for_parent(&self, parent_task_id: &str) -> usize {
        self.tasks
            .values()
            .filter(|task| {
                task.parent_task_id.as_deref() == Some(parent_task_id)
                    && !task.lifecycle().is_terminal()
            })
            .count()
    }

    pub fn new(
        session_id: impl Into<String>,
        workspace: WorkspaceContext,
        overlay_budget: SessionBudgetOverlay,
    ) -> Self {
        let session_id = session_id.into();
        Self {
            session_id: session_id.clone(),
            workspace: workspace.clone(),
            overlay_budget,
            tasks: BTreeMap::new(),
            artifacts: ExecutionArtifacts::default(),
            events: vec![SessionEvent::SessionCreated {
                session_id,
                workspace,
            }],
        }
    }

    pub fn workspace(&self) -> &WorkspaceContext {
        &self.workspace
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub fn task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    pub fn task_mut(&mut self, task_id: &str) -> Option<&mut Task> {
        self.tasks.get_mut(task_id)
    }

    pub fn session_events(&self) -> &[SessionEvent] {
        &self.events
    }

    pub fn artifacts(&self) -> &ExecutionArtifacts {
        &self.artifacts
    }

    pub fn record_artifact(&mut self, artifact: ExecutionArtifact) -> SessionEvent {
        let event = SessionEvent::ArtifactRecorded {
            session_id: self.session_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            kind: artifact.kind.clone(),
            durability: artifact.durability.clone(),
        };
        self.record_shared_artifact_bounded(artifact);
        self.push_event(event.clone());
        event
    }

    pub fn spawn_task(
        &mut self,
        task_id: impl Into<String>,
        objective: impl Into<String>,
        budget: TaskBudget,
        execution_mode: TaskExecutionMode,
    ) -> Result<(), CoreModelError> {
        let task_id = task_id.into();
        if self.tasks.contains_key(&task_id) {
            return Err(CoreModelError::DuplicateTask {
                session_id: self.session_id.clone(),
                task_id,
            });
        }
        if self.active_task_count() >= self.overlay_budget.max_parallel_tasks {
            return Err(CoreModelError::SessionParallelTaskBudgetExceeded {
                session_id: self.session_id.clone(),
                limit: self.overlay_budget.max_parallel_tasks,
            });
        }
        let task = Task::new(
            task_id.clone(),
            objective,
            self.workspace.clone(),
            budget,
            execution_mode,
        );
        self.tasks.insert(task_id, task);
        Ok(())
    }

    pub fn spawn_subtask(
        &mut self,
        parent_task_id: &str,
        child_task_id: impl Into<String>,
        objective: impl Into<String>,
        execution_mode: TaskExecutionMode,
        budget_policy: ChildBudgetPolicy,
        workspace_override: Option<WorkspaceContext>,
    ) -> Result<(), CoreModelError> {
        let child_task_id = child_task_id.into();
        if self.tasks.contains_key(&child_task_id) {
            return Err(CoreModelError::DuplicateTask {
                session_id: self.session_id.clone(),
                task_id: child_task_id,
            });
        }
        if self.active_child_task_count() >= self.overlay_budget.max_parallel_child_tasks {
            return Err(CoreModelError::SessionParallelChildTaskBudgetExceeded {
                session_id: self.session_id.clone(),
                limit: self.overlay_budget.max_parallel_child_tasks,
            });
        }
        let parent_child_limit = self
            .tasks
            .get(parent_task_id)
            .ok_or_else(|| CoreModelError::UnknownTask {
                session_id: self.session_id.clone(),
                task_id: parent_task_id.to_owned(),
            })?
            .budget()
            .max_child_tasks;
        if self.active_child_task_count_for_parent(parent_task_id) >= parent_child_limit {
            return Err(CoreModelError::TaskChildBudgetExceeded {
                task_id: parent_task_id.to_owned(),
                limit: parent_child_limit,
            });
        }

        let child = {
            let parent =
                self.tasks
                    .get_mut(parent_task_id)
                    .ok_or_else(|| CoreModelError::UnknownTask {
                        session_id: self.session_id.clone(),
                        task_id: parent_task_id.to_owned(),
                    })?;
            parent.spawn_subtask(
                child_task_id.clone(),
                objective,
                execution_mode,
                budget_policy,
                workspace_override,
            )
        };

        self.tasks.insert(child_task_id, child);
        Ok(())
    }

    pub fn switch_worktree(&mut self, workspace: WorkspaceContext) -> Result<(), CoreModelError> {
        if self.workspace.repo_root != workspace.repo_root {
            return Err(CoreModelError::RepositoryMismatch {
                expected: self.workspace.repo_root.clone(),
                actual: workspace.repo_root,
            });
        }
        let previous_workspace = self.workspace.clone();
        self.workspace = workspace.clone();
        self.push_event(SessionEvent::WorktreeSwitched {
            session_id: self.session_id.clone(),
            previous_workspace,
            next_workspace: workspace,
        });
        Ok(())
    }

    pub fn fact_events(&self) -> Vec<&TaskEvent> {
        self.tasks
            .values()
            .flat_map(|task| task.fact_events().iter())
            .collect()
    }
}
