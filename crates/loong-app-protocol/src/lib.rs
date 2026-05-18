#![forbid(unsafe_code)]

//! Transitional Phase 2 app-facing protocol spine.
//! Delete overlapping task/session orchestration inside legacy CLI surfaces once
//! Phase 3 retargets a real user path through this protocol layer.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use loong_runtime::WorkspaceContext as AppProtocolWorkspaceContext;
use loong_runtime::{
    ChildBudgetPolicy, ExecutionArtifact, ExecutionArtifactKind, RuntimeExecutorRequest,
    RuntimeExecutorResult, RuntimeInteractiveExecution, RuntimeInteractiveExecutor,
    RuntimeInteractiveExecutorRequest, RuntimeInteractiveExecutorResult, RuntimeInteractiveRequest,
    RuntimeOneshotExecution, RuntimeOneshotExecutor, RuntimeSessionDescriptor,
    RuntimeTaskDescriptor, RuntimeTaskStatusExecution, RuntimeTaskStatusExecutor,
    RuntimeTaskStatusExecutorResult, RuntimeTaskStatusRequest, Session, SessionBudgetOverlay,
    SessionEvent, Task, TaskBudget, TaskEvent, TaskExecutionMode, TaskLifecycle, TurnStatus,
    WorkspaceContext,
};
pub use loong_runtime::{
    RuntimeExecutorRequest as AppProtocolRuntimeExecutorRequest,
    RuntimeExecutorResult as AppProtocolRuntimeExecutorResult,
    RuntimeInteractiveExecutorRequest as AppProtocolRuntimeInteractiveExecutorRequest,
    RuntimeInteractiveExecutorResult as AppProtocolRuntimeInteractiveExecutorResult,
    RuntimeTaskStatusExecutorResult as AppProtocolRuntimeTaskStatusExecutorResult,
    RuntimeTaskStatusRequest as AppProtocolRuntimeTaskStatusRequest,
};

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
    OneshotTurn(RuntimeOneshotExecution),
    InteractiveShell(RuntimeInteractiveExecution),
    TaskStatus(RuntimeTaskStatusExecution),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OneshotTurnRequest {
    pub config_path: Option<String>,
    pub session_hint: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutedOneshotTurnRequest {
    pub config_path: Option<String>,
    pub session_hint: Option<String>,
    pub message: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
    pub working_directory: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub acp_requested: bool,
    pub acp_event_stream: bool,
}

#[async_trait]
pub trait AppProtocolOneshotExecutor: Send + Sync {
    async fn execute(
        &self,
        request: RuntimeExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String>;
}

pub fn build_runtime_oneshot_request(
    request: &OneshotTurnRequest,
    workspace: WorkspaceContext,
) -> Result<loong_runtime::RuntimeOneshotRequest, String> {
    if request.message.trim().is_empty() {
        return Err("turn message must not be empty".to_owned());
    }

    Ok(loong_runtime::RuntimeOneshotRequest {
        session_hint: request.session_hint.clone(),
        message: request.message.clone(),
        workspace,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRoutedExecutorRequest {
    pub session_hint: Option<String>,
    pub message: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
    pub working_directory: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub acp_requested: bool,
    pub acp_event_stream: bool,
}

#[async_trait]
pub trait AppProtocolRoutedOneshotExecutor: Send + Sync {
    async fn execute_routed(
        &self,
        request: RuntimeRoutedExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String>;
}

pub fn build_runtime_routed_oneshot_request(
    request: &RoutedOneshotTurnRequest,
    workspace: WorkspaceContext,
) -> Result<loong_runtime::RuntimeOneshotRequest, String> {
    if request.message.trim().is_empty() {
        return Err("turn message must not be empty".to_owned());
    }

    Ok(loong_runtime::RuntimeOneshotRequest {
        session_hint: request.session_hint.clone(),
        message: request.message.clone(),
        workspace,
    })
}

pub async fn execute_routed_oneshot_turn<E: AppProtocolRoutedOneshotExecutor>(
    request: &RoutedOneshotTurnRequest,
    workspace: WorkspaceContext,
    executor: &E,
) -> Result<RuntimeOneshotExecution, String> {
    if request.message.trim().is_empty() {
        return Err("turn message must not be empty".to_owned());
    }

    let executor_result = executor
        .execute_routed(RuntimeRoutedExecutorRequest {
            session_hint: request.session_hint.clone(),
            message: request.message.clone(),
            channel_id: request.channel_id.clone(),
            account_id: request.account_id.clone(),
            conversation_id: request.conversation_id.clone(),
            participant_id: request.participant_id.clone(),
            thread_id: request.thread_id.clone(),
            working_directory: request.working_directory.clone(),
            metadata: request.metadata.clone(),
            acp_requested: request.acp_requested,
            acp_event_stream: request.acp_event_stream,
        })
        .await?;

    let session_id = executor_result.session_id.clone();
    let task_id = format!("turn-run:{session_id}");
    let mut session = Session::new(
        session_id.clone(),
        workspace,
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
    task.begin_turn(
        "turn-1",
        "execute routed one-shot turn through the additive spine",
    )
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

struct RuntimeExecutorAdapter<'a, E> {
    inner: &'a E,
}

#[async_trait]
impl<E> RuntimeOneshotExecutor for RuntimeExecutorAdapter<'_, E>
where
    E: AppProtocolOneshotExecutor,
{
    async fn execute(
        &self,
        request: RuntimeExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String> {
        self.inner.execute(request).await
    }
}

pub async fn execute_oneshot_turn<E: AppProtocolOneshotExecutor>(
    request: &OneshotTurnRequest,
    workspace: WorkspaceContext,
    executor: &E,
) -> Result<RuntimeOneshotExecution, String> {
    let runtime_request = build_runtime_oneshot_request(request, workspace)?;
    let adapter = RuntimeExecutorAdapter { inner: executor };
    loong_runtime::execute_oneshot_turn(&runtime_request, &adapter).await
}

pub fn render_oneshot_turn_output(execution: &RuntimeOneshotExecution) -> &str {
    execution.output_text.as_str()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractiveShellRequest {
    pub config_path: Option<String>,
    pub session_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStatusRequest {
    pub current_session_id: String,
    pub task_id: String,
}

#[async_trait]
pub trait AppProtocolInteractiveExecutor: Send + Sync {
    async fn run_interactive(
        &self,
        request: RuntimeInteractiveExecutorRequest,
    ) -> Result<RuntimeInteractiveExecutorResult, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeExecutorConfig {
    pub requested_config_path: Option<String>,
    pub resolved_config_path: PathBuf,
    pub runtime_workspace_root: Option<PathBuf>,
    pub latest_session_selector: Option<String>,
}

impl RuntimeExecutorConfig {
    pub fn preserve_requested_path_for_interactive_missing_config(
        requested_config_path: Option<String>,
        resolved_config_path: PathBuf,
    ) -> Self {
        Self {
            requested_config_path,
            resolved_config_path,
            runtime_workspace_root: None,
            latest_session_selector: None,
        }
    }
}

#[async_trait]
pub trait AppProtocolRuntimeHost: Send + Sync {
    async fn execute_oneshot_request(
        &self,
        config: &RuntimeExecutorConfig,
        request: RuntimeExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String>;

    async fn execute_routed_oneshot_request(
        &self,
        config: &RuntimeExecutorConfig,
        request: RuntimeRoutedExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String>;

    async fn execute_interactive_request(
        &self,
        config: &RuntimeExecutorConfig,
        request: RuntimeInteractiveExecutorRequest,
    ) -> Result<RuntimeInteractiveExecutorResult, String>;

    async fn resolve_latest_root_session_id(
        &self,
        config: &RuntimeExecutorConfig,
    ) -> Result<Option<String>, String>;
}

pub struct ProductionOneshotExecutor<'a, H> {
    host: &'a H,
    config: RuntimeExecutorConfig,
}

impl<'a, H> ProductionOneshotExecutor<'a, H> {
    pub fn new(host: &'a H, config: RuntimeExecutorConfig) -> Self {
        Self { host, config }
    }
}

pub struct ProductionRoutedOneshotExecutor<'a, H> {
    host: &'a H,
    config: RuntimeExecutorConfig,
}

impl<'a, H> ProductionRoutedOneshotExecutor<'a, H> {
    pub fn new(host: &'a H, config: RuntimeExecutorConfig) -> Self {
        Self { host, config }
    }
}

#[async_trait]
impl<H> AppProtocolOneshotExecutor for ProductionOneshotExecutor<'_, H>
where
    H: AppProtocolRuntimeHost,
{
    async fn execute(
        &self,
        request: RuntimeExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String> {
        self.host
            .execute_oneshot_request(&self.config, request)
            .await
    }
}

#[async_trait]
impl<H> AppProtocolRoutedOneshotExecutor for ProductionRoutedOneshotExecutor<'_, H>
where
    H: AppProtocolRuntimeHost,
{
    async fn execute_routed(
        &self,
        request: RuntimeRoutedExecutorRequest,
    ) -> Result<RuntimeExecutorResult, String> {
        self.host
            .execute_routed_oneshot_request(&self.config, request)
            .await
    }
}

pub struct ProductionInteractiveExecutor<'a, H> {
    host: &'a H,
    config: RuntimeExecutorConfig,
}

impl<'a, H> ProductionInteractiveExecutor<'a, H> {
    pub fn new(host: &'a H, config: RuntimeExecutorConfig) -> Self {
        Self { host, config }
    }
}

#[async_trait]
impl<H> AppProtocolInteractiveExecutor for ProductionInteractiveExecutor<'_, H>
where
    H: AppProtocolRuntimeHost,
{
    async fn run_interactive(
        &self,
        request: RuntimeInteractiveExecutorRequest,
    ) -> Result<RuntimeInteractiveExecutorResult, String> {
        self.host
            .execute_interactive_request(&self.config, request)
            .await
    }
}

pub fn load_runtime_executor_config(
    requested_config_path: Option<&str>,
) -> Result<RuntimeExecutorConfig, String> {
    let resolved_config_path = requested_config_path
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    resolved_config_path.try_exists().map_err(|error| {
        format!(
            "failed to access config path {}: {error}",
            resolved_config_path.display()
        )
    })?;
    let requested_config_path = requested_config_path.map(ToOwned::to_owned);
    let requested_path_for_interactive = requested_config_path;

    Ok(RuntimeExecutorConfig {
        requested_config_path: requested_path_for_interactive,
        resolved_config_path,
        runtime_workspace_root: Some(current_runtime_workspace_root()),
        latest_session_selector: Some("latest".to_owned()),
    })
}

fn current_runtime_workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn default_config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".loong").join("config.toml")
}

pub fn executor_requested_config_path(config: &RuntimeExecutorConfig) -> Option<&str> {
    config.requested_config_path.as_deref()
}

pub fn executor_resolved_config_path(config: &RuntimeExecutorConfig) -> &Path {
    config.resolved_config_path.as_path()
}

struct RuntimeInteractiveExecutorAdapter<'a, E> {
    inner: &'a E,
}

#[async_trait]
impl<E> RuntimeInteractiveExecutor for RuntimeInteractiveExecutorAdapter<'_, E>
where
    E: AppProtocolInteractiveExecutor,
{
    async fn run_interactive(
        &self,
        request: RuntimeInteractiveExecutorRequest,
    ) -> Result<RuntimeInteractiveExecutorResult, String> {
        self.inner.run_interactive(request).await
    }
}

pub fn build_runtime_interactive_request(
    request: &InteractiveShellRequest,
    workspace: WorkspaceContext,
) -> RuntimeInteractiveRequest {
    RuntimeInteractiveRequest {
        session_hint: request.session_hint.clone(),
        workspace,
    }
}

pub async fn execute_interactive_shell<E: AppProtocolInteractiveExecutor>(
    request: &InteractiveShellRequest,
    workspace: WorkspaceContext,
    executor: &E,
) -> Result<RuntimeInteractiveExecution, String> {
    let runtime_request = build_runtime_interactive_request(request, workspace);
    let adapter = RuntimeInteractiveExecutorAdapter { inner: executor };
    loong_runtime::execute_interactive_shell(&runtime_request, &adapter).await
}

#[async_trait]
pub trait AppProtocolTaskStatusExecutor: Send + Sync {
    async fn load_task_status(
        &self,
        request: RuntimeTaskStatusRequest,
    ) -> Result<RuntimeTaskStatusExecutorResult, String>;
}

struct RuntimeTaskStatusExecutorAdapter<'a, E> {
    inner: &'a E,
}

#[async_trait]
impl<E> RuntimeTaskStatusExecutor for RuntimeTaskStatusExecutorAdapter<'_, E>
where
    E: AppProtocolTaskStatusExecutor,
{
    async fn load_task_status(
        &self,
        request: RuntimeTaskStatusRequest,
    ) -> Result<RuntimeTaskStatusExecutorResult, String> {
        self.inner.load_task_status(request).await
    }
}

pub async fn execute_task_status<E: AppProtocolTaskStatusExecutor>(
    request: &TaskStatusRequest,
    workspace: WorkspaceContext,
    executor: &E,
) -> Result<RuntimeTaskStatusExecution, String> {
    let adapter = RuntimeTaskStatusExecutorAdapter { inner: executor };
    loong_runtime::execute_task_status(
        &RuntimeTaskStatusRequest {
            current_session_id: request.current_session_id.clone(),
            task_id: request.task_id.clone(),
            workspace,
        },
        &adapter,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockExecutor;

    #[async_trait]
    impl AppProtocolOneshotExecutor for MockExecutor {
        async fn execute(
            &self,
            request: RuntimeExecutorRequest,
        ) -> Result<RuntimeExecutorResult, String> {
            assert_eq!(request.session_hint.as_deref(), Some("latest"));
            assert_eq!(request.message, "summarize via protocol");
            Ok(RuntimeExecutorResult {
                session_id: "resolved-session".to_owned(),
                output_text: "protocol result".to_owned(),
                state: Some("ok".to_owned()),
                stop_reason: Some("completed".to_owned()),
                usage: None,
                event_count: 1,
            })
        }
    }

    struct MockInteractiveExecutor;

    #[async_trait]
    impl AppProtocolInteractiveExecutor for MockInteractiveExecutor {
        async fn run_interactive(
            &self,
            request: RuntimeInteractiveExecutorRequest,
        ) -> Result<RuntimeInteractiveExecutorResult, String> {
            assert_eq!(request.session_hint.as_deref(), Some("latest"));
            Ok(RuntimeInteractiveExecutorResult {
                session_id: "interactive-session".to_owned(),
                exit_state: "completed".to_owned(),
            })
        }
    }

    struct MockTaskStatusExecutor;

    #[async_trait]
    impl AppProtocolTaskStatusExecutor for MockTaskStatusExecutor {
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
    async fn protocol_oneshot_turn_routes_into_runtime_execution() {
        let request = OneshotTurnRequest {
            config_path: None,
            session_hint: Some("latest".to_owned()),
            message: "summarize via protocol".to_owned(),
        };
        let workspace = WorkspaceContext::new(
            "/tmp/project",
            "/tmp/project",
            "/tmp/project",
            "/tmp/project",
            "feature/phase3".to_owned(),
        );

        let execution = execute_oneshot_turn(&request, workspace.clone(), &MockExecutor)
            .await
            .expect("execute protocol oneshot turn");

        assert_eq!(execution.session.session_id, "resolved-session");
        assert_eq!(execution.session.workspace, workspace);
        assert_eq!(execution.output_text, "protocol result");
        assert_eq!(execution.task.task_id, "turn-run:resolved-session");
        assert_eq!(execution.task.lifecycle, TaskLifecycle::Completed);
    }

    #[tokio::test]
    async fn protocol_interactive_shell_routes_into_runtime_execution() {
        let request = InteractiveShellRequest {
            config_path: None,
            session_hint: Some("latest".to_owned()),
        };
        let workspace = WorkspaceContext::new(
            "/tmp/chat",
            "/tmp/chat",
            "/tmp/chat",
            "/tmp/chat",
            "feature/chat".to_owned(),
        );

        let execution =
            execute_interactive_shell(&request, workspace.clone(), &MockInteractiveExecutor)
                .await
                .expect("execute interactive shell");

        assert_eq!(execution.session.session_id, "interactive-session");
        assert_eq!(execution.session.workspace, workspace);
        assert_eq!(execution.task.task_id, "chat-shell:interactive-session");
        assert_eq!(execution.task.lifecycle, TaskLifecycle::Completed);
        assert_eq!(execution.exit_state, "completed");
    }

    #[tokio::test]
    async fn protocol_task_status_routes_into_runtime_execution() {
        let workspace = WorkspaceContext::new(
            "/tmp/tasks",
            "/tmp/tasks",
            "/tmp/tasks",
            "/tmp/tasks",
            "feature/tasks".to_owned(),
        );

        let execution = execute_task_status(
            &TaskStatusRequest {
                current_session_id: "ops-root".to_owned(),
                task_id: "delegate:task-1".to_owned(),
            },
            workspace,
            &MockTaskStatusExecutor,
        )
        .await
        .expect("execute task status");

        assert_eq!(execution.session.session_id, "delegate:task-1");
        assert_eq!(execution.task.task_id, "delegate:task-1");
        assert_eq!(execution.task.lifecycle, TaskLifecycle::WaitingForApproval);
    }
}
