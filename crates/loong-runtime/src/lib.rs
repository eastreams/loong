#![forbid(unsafe_code)]

//! Transitional runtime spine.
//! Delete the temporary legacy turn executor adapter after later phases move
//! the live one-shot and interactive runtime execution fully under this crate.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use loong_core::{
    ApprovalState, ArtifactDurabilityClass, ChildBudgetPolicy, DiagnosticSeverity,
    ExecutionArtifact, ExecutionArtifactKind, Session, SessionBudgetOverlay, SessionEvent, Task,
    TaskBudget, TaskEvent, TaskExecutionMode, TaskLifecycle, TurnStatus, WorkspaceContext,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeSurface {
    ProviderAdapters,
    DefaultCodingTools,
    BrowserAdapter,
    SessionStorage,
    ArtifactStorage,
    ProjectionCompaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSurfaceBinding {
    pub surface: RuntimeSurface,
    pub ownership_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSpine {
    pub core_contract: String,
    pub detached_execution_supported: bool,
    pub surfaces: Vec<RuntimeSurfaceBinding>,
}

impl Default for RuntimeSpine {
    fn default() -> Self {
        Self {
            core_contract: "loong-core".to_owned(),
            detached_execution_supported: true,
            surfaces: vec![
                RuntimeSurfaceBinding {
                    surface: RuntimeSurface::ProviderAdapters,
                    ownership_summary: "Official model/provider adapters stay above loong-core"
                        .to_owned(),
                },
                RuntimeSurfaceBinding {
                    surface: RuntimeSurface::DefaultCodingTools,
                    ownership_summary: "Default shell/file/browser tools live in the runtime layer"
                        .to_owned(),
                },
                RuntimeSurfaceBinding {
                    surface: RuntimeSurface::BrowserAdapter,
                    ownership_summary:
                        "Browser automation stays a runtime adapter, not a kernel concern"
                            .to_owned(),
                },
                RuntimeSurfaceBinding {
                    surface: RuntimeSurface::SessionStorage,
                    ownership_summary:
                        "Session truth storage binds durable facts without redefining the kernel"
                            .to_owned(),
                },
                RuntimeSurfaceBinding {
                    surface: RuntimeSurface::ArtifactStorage,
                    ownership_summary:
                        "Artifact references and durability storage are runtime-owned adapters"
                            .to_owned(),
                },
                RuntimeSurfaceBinding {
                    surface: RuntimeSurface::ProjectionCompaction,
                    ownership_summary:
                        "Projection compaction is optional runtime optimization only".to_owned(),
                },
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeOneshotRequest {
    pub session_hint: Option<String>,
    pub message: String,
    pub workspace: WorkspaceContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeExecutorRequest {
    pub session_hint: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeExecutorResult {
    pub session_id: String,
    pub output_text: String,
    pub state: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub event_count: usize,
}

#[async_trait]
pub trait RuntimeOneshotExecutor: Send + Sync {
    async fn execute(
        &self,
        request: RuntimeExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSessionDescriptor {
    pub session_id: String,
    pub workspace: WorkspaceContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTaskDescriptor {
    pub task_id: String,
    pub objective: String,
    pub lifecycle: TaskLifecycle,
    pub execution_mode: TaskExecutionMode,
    pub current_turn_id: Option<String>,
    pub artifact_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeOneshotExecution {
    pub session: RuntimeSessionDescriptor,
    pub task: RuntimeTaskDescriptor,
    pub session_events: Vec<SessionEvent>,
    pub task_events: Vec<TaskEvent>,
    pub output_text: String,
    pub state: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub event_count: usize,
}

pub async fn execute_oneshot_turn<E: RuntimeOneshotExecutor>(
    request: &RuntimeOneshotRequest,
    executor: &E,
) -> Result<RuntimeOneshotExecution, String> {
    if request.message.trim().is_empty() {
        return Err("turn message must not be empty".to_owned());
    }

    let executor_result = executor
        .execute(RuntimeExecutorRequest {
            session_hint: request.session_hint.clone(),
            message: request.message.clone(),
        })
        .await?;

    let session_id = executor_result.session_id.clone();
    let task_id = format!("turn-run:{session_id}");
    let mut session = Session::new(
        session_id.clone(),
        request.workspace.clone(),
        SessionBudgetOverlay {
            max_total_artifacts: Some(1),
            ..SessionBudgetOverlay::default()
        },
    );
    session.record_artifact(ExecutionArtifact::new(
        "session-output-text",
        ExecutionArtifactKind::AssistantTextOutput {
            text: executor_result.output_text.clone(),
        },
    ));
    session
        .spawn_task(
            task_id.as_str(),
            request.message.as_str(),
            TaskBudget::default(),
            TaskExecutionMode::RemoteControlledAttached,
        )
        .map_err(|error| error.to_string())?;
    let task = session
        .task_mut(task_id.as_str())
        .ok_or_else(|| "migrated runtime task missing after spawn".to_owned())?;
    task.transition_to(TaskLifecycle::Running)
        .map_err(|error| error.to_string())?;
    task.begin_turn("turn-1", "execute one-shot turn through the additive spine")
        .map_err(|error| error.to_string())?;
    task.record_artifact(ExecutionArtifact::new(
        "artifact-output-text",
        ExecutionArtifactKind::AssistantTextOutput {
            text: executor_result.output_text.clone(),
        },
    ));
    task.finish_current_turn(TurnStatus::Completed);
    task.transition_to(TaskLifecycle::Completed)
        .map_err(|error| error.to_string())?;

    let task = session
        .task(task_id.as_str())
        .ok_or_else(|| "migrated runtime task missing after completion".to_owned())?;

    Ok(RuntimeOneshotExecution {
        session: RuntimeSessionDescriptor {
            session_id,
            workspace: session.workspace().clone(),
        },
        task: RuntimeTaskDescriptor {
            task_id: task.task_id.clone(),
            objective: task.objective.clone(),
            lifecycle: task.lifecycle().clone(),
            execution_mode: task.execution_mode.clone(),
            current_turn_id: task.current_turn().map(|turn| turn.turn_id.clone()),
            artifact_count: task.artifacts().len(),
        },
        session_events: session.session_events().to_vec(),
        task_events: task.fact_events().to_vec(),
        output_text: executor_result.output_text,
        state: executor_result.state,
        stop_reason: executor_result.stop_reason,
        usage: executor_result.usage,
        event_count: executor_result.event_count,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractiveRequest {
    pub session_hint: Option<String>,
    pub workspace: WorkspaceContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractiveExecutorRequest {
    pub session_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractiveExecutorResult {
    pub session_id: String,
    pub exit_state: String,
}

#[async_trait]
pub trait RuntimeInteractiveExecutor: Send + Sync {
    async fn run_interactive(
        &self,
        request: RuntimeInteractiveExecutorRequest,
    ) -> Result<RuntimeInteractiveExecutorResult, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractiveExecution {
    pub session: RuntimeSessionDescriptor,
    pub task: RuntimeTaskDescriptor,
    pub session_events: Vec<SessionEvent>,
    pub task_events: Vec<TaskEvent>,
    pub exit_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTaskStatusRequest {
    pub current_session_id: String,
    pub task_id: String,
    pub workspace: WorkspaceContext,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeTaskStatusExecutorResult {
    pub detail: Value,
}

#[async_trait]
pub trait RuntimeTaskStatusExecutor: Send + Sync {
    async fn load_task_status(
        &self,
        request: RuntimeTaskStatusRequest,
    ) -> Result<RuntimeTaskStatusExecutorResult, String>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeTaskStatusExecution {
    pub session: RuntimeSessionDescriptor,
    pub task: RuntimeTaskDescriptor,
    pub detail: Value,
}

pub async fn execute_interactive_shell<E: RuntimeInteractiveExecutor>(
    request: &RuntimeInteractiveRequest,
    executor: &E,
) -> Result<RuntimeInteractiveExecution, String> {
    let executor_result = executor
        .run_interactive(RuntimeInteractiveExecutorRequest {
            session_hint: request.session_hint.clone(),
        })
        .await?;

    let session_id = executor_result.session_id.clone();
    let task_id = format!("chat-shell:{session_id}");
    let mut session = Session::new(
        session_id.clone(),
        request.workspace.clone(),
        SessionBudgetOverlay::default(),
    );
    session
        .spawn_task(
            task_id.as_str(),
            "interactive chat shell",
            TaskBudget::default(),
            TaskExecutionMode::InteractiveAttached,
        )
        .map_err(|error| error.to_string())?;
    let task = session
        .task_mut(task_id.as_str())
        .ok_or_else(|| "interactive shell task missing after spawn".to_owned())?;
    task.transition_to(TaskLifecycle::Running)
        .map_err(|error| error.to_string())?;
    task.begin_turn(
        "turn-1",
        "run interactive chat shell through the additive spine",
    )
    .map_err(|error| error.to_string())?;
    task.finish_current_turn(TurnStatus::Completed);
    task.transition_to(TaskLifecycle::Completed)
        .map_err(|error| error.to_string())?;

    let task = session
        .task(task_id.as_str())
        .ok_or_else(|| "interactive shell task missing after completion".to_owned())?;

    Ok(RuntimeInteractiveExecution {
        session: RuntimeSessionDescriptor {
            session_id,
            workspace: session.workspace().clone(),
        },
        task: RuntimeTaskDescriptor {
            task_id: task.task_id.clone(),
            objective: task.objective.clone(),
            lifecycle: task.lifecycle().clone(),
            execution_mode: task.execution_mode.clone(),
            current_turn_id: task.current_turn().map(|turn| turn.turn_id.clone()),
            artifact_count: task.artifacts().len(),
        },
        session_events: session.session_events().to_vec(),
        task_events: task.fact_events().to_vec(),
        exit_state: executor_result.exit_state,
    })
}

pub async fn execute_task_status<E: RuntimeTaskStatusExecutor>(
    request: &RuntimeTaskStatusRequest,
    executor: &E,
) -> Result<RuntimeTaskStatusExecution, String> {
    let executor_result = executor.load_task_status(request.clone()).await?;
    let detail = executor_result.detail;
    let task_id = required_string_field(&detail, "task_id", "runtime task status detail")?;
    let owner_session_id = detail
        .get("owner_session_id")
        .and_then(Value::as_str)
        .unwrap_or(task_id.as_str())
        .to_owned();
    let label = detail
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or(task_id.as_str())
        .to_owned();
    let task_status_kind = detail
        .get("task_status")
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let session_workspace = task_workspace_context(&detail, &request.workspace);

    let mut session = Session::new(
        owner_session_id.clone(),
        session_workspace.clone(),
        SessionBudgetOverlay::default(),
    );
    session
        .spawn_task(
            task_id.as_str(),
            label,
            TaskBudget::default(),
            task_execution_mode_from_detail(&detail),
        )
        .map_err(|error| error.to_string())?;
    let task = session
        .task_mut(task_id.as_str())
        .ok_or_else(|| "runtime task status task missing after spawn".to_owned())?;
    apply_task_lifecycle(task, task_status_kind)?;

    let task = session
        .task(task_id.as_str())
        .ok_or_else(|| "runtime task status task missing after lifecycle mapping".to_owned())?;

    Ok(RuntimeTaskStatusExecution {
        session: RuntimeSessionDescriptor {
            session_id: owner_session_id,
            workspace: session_workspace,
        },
        task: RuntimeTaskDescriptor {
            task_id: task.task_id.clone(),
            objective: task.objective.clone(),
            lifecycle: task.lifecycle().clone(),
            execution_mode: task.execution_mode.clone(),
            current_turn_id: task.current_turn().map(|turn| turn.turn_id.clone()),
            artifact_count: task.artifacts().len(),
        },
        detail,
    })
}

fn apply_task_lifecycle(task: &mut Task, task_status_kind: &str) -> Result<(), String> {
    match task_status_kind {
        "queued" => Ok(()),
        "running" | "cancel_requested" => task
            .transition_to(TaskLifecycle::Running)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        "approval_pending" => {
            task.transition_to(TaskLifecycle::Running)
                .map_err(|error| error.to_string())?;
            task.transition_to(TaskLifecycle::WaitingForApproval)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        "overdue" => {
            task.transition_to(TaskLifecycle::Running)
                .map_err(|error| error.to_string())?;
            task.transition_to(TaskLifecycle::Blocked)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        "completed" => {
            task.transition_to(TaskLifecycle::Running)
                .map_err(|error| error.to_string())?;
            task.transition_to(TaskLifecycle::Completed)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        "failed" | "timed_out" => {
            task.transition_to(TaskLifecycle::Running)
                .map_err(|error| error.to_string())?;
            task.transition_to(TaskLifecycle::Failed)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        _ => Ok(()),
    }
}

fn task_execution_mode_from_detail(detail: &Value) -> TaskExecutionMode {
    let execution_surface = detail
        .get("workflow")
        .and_then(|value| value.get("binding"))
        .and_then(|value| value.get("execution_surface"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let session_kind = detail
        .get("session")
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("");

    if execution_surface == "delegate.async" || session_kind == "delegate_child" {
        return TaskExecutionMode::DetachedBackground;
    }

    TaskExecutionMode::RemoteControlledAttached
}

fn task_workspace_context(detail: &Value, fallback: &WorkspaceContext) -> WorkspaceContext {
    let workflow_binding = detail
        .get("workflow")
        .and_then(|value| value.get("binding"));
    let worktree = workflow_binding.and_then(|value| value.get("worktree"));
    let workspace_root = worktree
        .and_then(|value| value.get("workspace_root"))
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fallback.workspace_root.clone());
    let worktree_root = workspace_root.clone();
    let repo_root = fallback.repo_root.clone();
    let cwd = fallback.cwd.clone();
    let branch_identity = fallback.branch_identity.clone();
    WorkspaceContext::new(
        workspace_root,
        repo_root,
        worktree_root,
        cwd,
        branch_identity,
    )
}

fn required_string_field(value: &Value, field: &str, context: &str) -> Result<String, String> {
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context} missing string field `{field}`"))?;
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockExecutor {
        session_id: String,
        output_text: String,
    }

    #[async_trait]
    impl RuntimeOneshotExecutor for MockExecutor {
        async fn execute(
            &self,
            _request: RuntimeExecutorRequest,
        ) -> Result<RuntimeExecutorResult, String> {
            Ok(RuntimeExecutorResult {
                session_id: self.session_id.clone(),
                output_text: self.output_text.clone(),
                state: Some("ok".to_owned()),
                stop_reason: Some("completed".to_owned()),
                usage: None,
                event_count: 0,
            })
        }
    }

    #[tokio::test]
    async fn runtime_oneshot_execution_builds_visible_core_truth() {
        let workspace = WorkspaceContext::new(
            "/tmp/phase3",
            "/tmp/phase3",
            "/tmp/phase3",
            "/tmp/phase3",
            "feature/phase3".to_owned(),
        );
        let request = RuntimeOneshotRequest {
            session_hint: Some("latest".to_owned()),
            message: "Summarize this repository.".to_owned(),
            workspace: workspace.clone(),
        };
        let executor = MockExecutor {
            session_id: "resolved-session".to_owned(),
            output_text: "phase3 migrated reply".to_owned(),
        };

        let execution = execute_oneshot_turn(&request, &executor)
            .await
            .expect("execute oneshot turn");

        assert_eq!(execution.session.session_id, "resolved-session");
        assert_eq!(execution.session.workspace, workspace);
        assert_eq!(execution.task.task_id, "turn-run:resolved-session");
        assert_eq!(
            execution.task.execution_mode,
            TaskExecutionMode::RemoteControlledAttached
        );
        assert_eq!(execution.task.lifecycle, TaskLifecycle::Completed);
        assert_eq!(execution.task.current_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(execution.task.artifact_count, 1);
        assert_eq!(execution.output_text, "phase3 migrated reply");
        assert_eq!(execution.usage, None);
        assert!(matches!(
            execution.session_events.as_slice(),
            [
                SessionEvent::SessionCreated { .. },
                SessionEvent::ArtifactRecorded { .. }
            ]
        ));
        assert!(
            execution
                .task_events
                .iter()
                .any(|event| matches!(event, TaskEvent::ArtifactRecorded { .. })),
            "task events should include artifact truth"
        );
    }

    struct MockInteractiveExecutor {
        session_id: String,
    }

    #[async_trait]
    impl RuntimeInteractiveExecutor for MockInteractiveExecutor {
        async fn run_interactive(
            &self,
            _request: RuntimeInteractiveExecutorRequest,
        ) -> Result<RuntimeInteractiveExecutorResult, String> {
            Ok(RuntimeInteractiveExecutorResult {
                session_id: self.session_id.clone(),
                exit_state: "completed".to_owned(),
            })
        }
    }

    #[tokio::test]
    async fn runtime_interactive_execution_builds_visible_core_truth() {
        let workspace = WorkspaceContext::new(
            "/tmp/phase3-chat",
            "/tmp/phase3-chat",
            "/tmp/phase3-chat",
            "/tmp/phase3-chat",
            "feature/phase3-chat".to_owned(),
        );
        let request = RuntimeInteractiveRequest {
            session_hint: Some("latest".to_owned()),
            workspace: workspace.clone(),
        };
        let executor = MockInteractiveExecutor {
            session_id: "chat-session".to_owned(),
        };

        let execution = execute_interactive_shell(&request, &executor)
            .await
            .expect("execute interactive shell");

        assert_eq!(execution.session.session_id, "chat-session");
        assert_eq!(execution.session.workspace, workspace);
        assert_eq!(execution.task.task_id, "chat-shell:chat-session");
        assert_eq!(
            execution.task.execution_mode,
            TaskExecutionMode::InteractiveAttached
        );
        assert_eq!(execution.task.lifecycle, TaskLifecycle::Completed);
        assert_eq!(execution.task.current_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(execution.exit_state, "completed");
    }

    struct MockTaskStatusExecutor;

    #[async_trait]
    impl RuntimeTaskStatusExecutor for MockTaskStatusExecutor {
        async fn load_task_status(
            &self,
            request: RuntimeTaskStatusRequest,
        ) -> Result<RuntimeTaskStatusExecutorResult, String> {
            assert_eq!(request.current_session_id, "ops-root");
            assert_eq!(request.task_id, "delegate:task-1");
            Ok(RuntimeTaskStatusExecutorResult {
                detail: serde_json::json!({
                    "task_id": "delegate:task-1",
                    "owner_session_id": "delegate:task-1",
                    "label": "Release Check",
                    "task_status": { "kind": "approval_pending" },
                    "workflow": {
                        "binding": {
                            "execution_surface": "delegate.async",
                            "worktree": {
                                "worktree_id": "delegate:task-1",
                                "workspace_root": "/tmp/loong/tasks-cli/delegate:task-1"
                            }
                        }
                    },
                    "session": {
                        "kind": "delegate_child"
                    }
                }),
            })
        }
    }

    #[tokio::test]
    async fn runtime_task_status_execution_builds_visible_core_truth() {
        let workspace = WorkspaceContext::new(
            "/tmp/ops-root",
            "/tmp/ops-root",
            "/tmp/ops-root",
            "/tmp/ops-root",
            "feature/tasks".to_owned(),
        );
        let request = RuntimeTaskStatusRequest {
            current_session_id: "ops-root".to_owned(),
            task_id: "delegate:task-1".to_owned(),
            workspace,
        };

        let execution = execute_task_status(&request, &MockTaskStatusExecutor)
            .await
            .expect("execute task status");

        assert_eq!(execution.session.session_id, "delegate:task-1");
        assert_eq!(execution.task.task_id, "delegate:task-1");
        assert_eq!(execution.task.objective, "Release Check");
        assert_eq!(execution.task.lifecycle, TaskLifecycle::WaitingForApproval);
        assert_eq!(
            execution.task.execution_mode,
            TaskExecutionMode::DetachedBackground
        );
        assert_eq!(execution.detail["task_id"], "delegate:task-1");
    }
}
