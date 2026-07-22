#![forbid(unsafe_code)]

//! Application command and response contracts shared by Loong hosts.

use loong_core::{
    ChildBudgetPolicy, ExecutionArtifact, Session, SessionEvent, Task, TaskBudget, TaskEvent,
    TaskExecutionMode, TaskLifecycle, WorkspaceContext,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartTaskRequest {
    pub session_id: String,
    pub objective: String,
    pub budget: TaskBudget,
    pub execution_mode: TaskExecutionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkTaskRequest {
    pub session_id: String,
    pub parent_task_id: String,
    pub objective: String,
    pub execution_mode: TaskExecutionMode,
    pub budget_policy: ChildBudgetPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendInputRequest {
    pub session_id: String,
    pub task_id: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveRequest {
    pub session_id: String,
    pub task_id: String,
    pub checkpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptRequest {
    pub session_id: String,
    pub task_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchWorktreeRequest {
    pub session_id: String,
    pub workspace: WorkspaceContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadArtifactsRequest {
    pub session_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamEventsRequest {
    pub session_id: String,
    pub task_id: Option<String>,
    pub from_event_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppCommand {
    StartTask(StartTaskRequest),
    ResumeTask { session_id: String, task_id: String },
    ForkTask(ForkTaskRequest),
    SendInput(SendInputRequest),
    Approve(ApproveRequest),
    Interrupt(InterruptRequest),
    SwitchWorktree(SwitchWorktreeRequest),
    ReadArtifacts(ReadArtifactsRequest),
    ListTasks { session_id: String },
    ListSessions,
    StreamEvents(StreamEventsRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskProjection {
    pub task_id: String,
    pub parent_task_id: Option<String>,
    pub objective: String,
    pub lifecycle: TaskLifecycle,
    pub execution_mode: TaskExecutionMode,
    pub current_turn_id: Option<String>,
    pub artifact_count: usize,
    pub subtask_count: usize,
}

impl From<&Task> for TaskProjection {
    fn from(task: &Task) -> Self {
        Self {
            task_id: task.task_id.clone(),
            parent_task_id: task.parent_task_id.clone(),
            objective: task.objective.clone(),
            lifecycle: task.lifecycle().clone(),
            execution_mode: task.execution_mode.clone(),
            current_turn_id: task.current_turn().map(|turn| turn.turn_id.clone()),
            artifact_count: task.artifacts().len(),
            subtask_count: task.subtasks().len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProjection {
    pub session_id: String,
    pub workspace: WorkspaceContext,
    pub tracked_task_count: usize,
    pub active_task_count: usize,
}

impl From<&Session> for SessionProjection {
    fn from(session: &Session) -> Self {
        let tracked_task_count = session.tasks().count();
        let active_task_count = session
            .tasks()
            .filter(|task| !task.lifecycle().is_terminal())
            .count();
        Self {
            session_id: session.session_id.clone(),
            workspace: session.workspace().clone(),
            tracked_task_count,
            active_task_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AppResponse {
    Ack,
    Task(TaskProjection),
    Tasks(Vec<TaskProjection>),
    Session(SessionProjection),
    Sessions(Vec<SessionProjection>),
    SessionEvents(Vec<SessionEvent>),
    Events(Vec<TaskEvent>),
    Artifacts(Vec<ExecutionArtifact>),
}
