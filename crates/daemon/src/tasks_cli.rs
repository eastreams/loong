use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use clap::Subcommand;
use kernel::ToolCoreRequest;
use loong_app as mvp;
use loong_app_protocol::{
    AppProtocolRuntimeTaskStatusExecutorResult, AppProtocolRuntimeTaskStatusRequest,
    AppProtocolTaskStatusExecutor, AppProtocolWorkspaceContext, TaskStatusRequest,
    execute_task_status,
};
use loong_contracts::ToolCoreOutcome;
use loong_spec::CliResult;
use serde_json::{Value, json};
use std::path::PathBuf;

#[path = "tasks_cli_render.rs"]
mod render_support;
#[path = "tasks_cli_status.rs"]
mod status_support;

use self::render_support::render_tasks_cli_text;
use self::status_support::{
    TaskStatusSummary, build_task_status_payload, summarize_task_status_payload,
    unknown_task_status_payload,
};

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TasksCommands {
    /// Queue one async background task on top of the current session runtime
    Create {
        task: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        timeout_seconds: Option<u64>,
    },
    /// List visible async background tasks for the scoped session
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = false)]
        overdue_only: bool,
        #[arg(long, default_value_t = false)]
        include_archived: bool,
    },
    /// Inspect one visible async background task
    #[command(visible_alias = "info")]
    Status { task_id: String },
    /// Show recent lifecycle events for one visible async background task
    Events {
        task_id: String,
        #[arg(long)]
        after_id: Option<i64>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Wait on one visible async background task and return incremental events
    Wait {
        task_id: String,
        #[arg(long)]
        after_id: Option<i64>,
        #[arg(long, default_value_t = 1_000)]
        timeout_ms: u64,
    },
    /// Cancel one visible async background task
    Cancel {
        task_id: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Recover one visible overdue async background task
    Recover {
        task_id: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone)]
pub struct TasksCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub session: String,
    pub command: TasksCommands,
}

#[derive(Debug, Clone)]
pub struct TasksCommandExecution {
    pub resolved_config_path: String,
    pub current_session_id: String,
    pub payload: Value,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
struct DetachedTasksSpawner;

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl mvp::conversation::AsyncDelegateSpawner for DetachedTasksSpawner {
    async fn spawn(
        &self,
        request: mvp::conversation::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        crate::delegate_child_cli::spawn_detached_delegate_child_process(&request)?;
        Ok(())
    }
}

pub async fn run_tasks_cli(options: TasksCommandOptions) -> CliResult<()> {
    let as_json = options.json;
    let execution = execute_tasks_command(options).await?;
    if as_json {
        let pretty = serde_json::to_string_pretty(&execution.payload)
            .map_err(|error| format!("serialize tasks CLI output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = render_tasks_cli_text(&execution)?;
    println!("{rendered}");
    Ok(())
}

pub async fn execute_tasks_command(
    options: TasksCommandOptions,
) -> CliResult<TasksCommandExecution> {
    let TasksCommandOptions {
        config,
        json: _,
        session,
        command,
    } = options;
    let (resolved_path, config) = mvp::config::load(config.as_deref())?;
    mvp::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));

    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let current_session_id = resolve_session_scope(&session, &memory_config)?;
    let tool_config = &config.tools;

    let payload = match command {
        TasksCommands::Create {
            task,
            label,
            timeout_seconds,
        } => {
            execute_create_command(
                &resolved_path.display().to_string(),
                &config,
                &current_session_id,
                &memory_config,
                tool_config,
                &task,
                label,
                timeout_seconds,
            )
            .await?
        }
        TasksCommands::List {
            limit,
            state,
            overdue_only,
            include_archived,
        } => {
            execute_list_command(
                &resolved_path.display().to_string(),
                &current_session_id,
                &memory_config,
                tool_config,
                limit,
                state.as_deref(),
                overdue_only,
                include_archived,
            )
            .await?
        }
        TasksCommands::Status { task_id } => {
            execute_status_command(
                &resolved_path.display().to_string(),
                &current_session_id,
                &memory_config,
                tool_config,
                &task_id,
            )
            .await?
        }
        TasksCommands::Events {
            task_id,
            after_id,
            limit,
        } => {
            execute_events_command(
                &resolved_path.display().to_string(),
                &current_session_id,
                &memory_config,
                tool_config,
                &task_id,
                after_id,
                limit,
            )
            .await?
        }
        TasksCommands::Wait {
            task_id,
            after_id,
            timeout_ms,
        } => {
            execute_wait_command(
                &resolved_path.display().to_string(),
                &current_session_id,
                &memory_config,
                tool_config,
                &task_id,
                after_id,
                timeout_ms,
            )
            .await?
        }
        TasksCommands::Cancel { task_id, dry_run } => {
            execute_cancel_command(
                &resolved_path.display().to_string(),
                &current_session_id,
                &memory_config,
                tool_config,
                &task_id,
                dry_run,
            )
            .await?
        }
        TasksCommands::Recover { task_id, dry_run } => {
            execute_recover_command(
                &resolved_path.display().to_string(),
                &current_session_id,
                &memory_config,
                tool_config,
                &task_id,
                dry_run,
            )
            .await?
        }
    };

    Ok(TasksCommandExecution {
        resolved_config_path: resolved_path.display().to_string(),
        current_session_id,
        payload,
    })
}

async fn execute_create_command(
    resolved_config_path: &str,
    config: &mvp::config::LoongConfig,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task: &str,
    label: Option<String>,
    timeout_seconds: Option<u64>,
) -> CliResult<Value> {
    let runtime = build_tasks_create_runtime(config)?;
    let runtime_kernel = bootstrap_tasks_runtime_kernel(config)?;
    let binding = runtime_kernel.conversation_binding();
    let queued = mvp::conversation::spawn_background_delegate_with_runtime(
        config,
        &runtime,
        current_session_id,
        task,
        label,
        None,
        timeout_seconds,
        binding,
    )
    .await?;
    let task_session_id =
        required_string_field(&queued.payload, "child_session_id", "tasks create")?;
    let (task_detail, task_lookup_error) = build_best_effort_task_detail(
        memory_config,
        tool_config,
        current_session_id,
        &task_session_id,
    )
    .await;
    let task_id = task_detail
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or(task_session_id.as_str())
        .to_owned();
    let recipes = build_task_recipes(resolved_config_path, current_session_id, &task_id);
    let next_steps = build_task_next_steps();
    let payload = json!({
        "command": "create",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "queued_outcome": queued.payload,
        "task": task_detail,
        "task_lookup_error": task_lookup_error,
        "recipes": recipes,
        "next_steps": next_steps,
    });
    Ok(payload)
}

fn bootstrap_tasks_runtime_kernel(
    config: &mvp::config::LoongConfig,
) -> CliResult<mvp::runtime_bridge::RuntimeKernelOwner> {
    let agent_id = "cli-tasks";
    let runtime_kernel = mvp::runtime_bridge::RuntimeKernelOwner::bootstrap(agent_id, config)?;
    Ok(runtime_kernel)
}

fn build_tasks_create_runtime(
    config: &mvp::config::LoongConfig,
) -> CliResult<impl mvp::conversation::ConversationRuntime> {
    // Background task creation prefers the detached sqlite-backed runtime when
    // available so delegated child sessions can survive outside the foreground
    // CLI process. Non-sqlite builds fall back to the default in-process
    // conversation runtime.
    #[cfg(feature = "memory-sqlite")]
    {
        let background_task_spawner = Arc::new(DetachedTasksSpawner);
        let runtime = mvp::conversation::load_hosted_default_conversation_runtime(config)?
            .with_background_task_spawner(background_task_spawner);
        Ok(runtime)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let runtime = mvp::conversation::load_default_conversation_runtime(config)?;
        Ok(runtime)
    }
}

async fn execute_list_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    limit: usize,
    state: Option<&str>,
    overdue_only: bool,
    include_archived: bool,
) -> CliResult<Value> {
    let raw_limit = limit.clamp(1, 200);
    let session_ids = load_visible_background_task_ids(
        memory_config,
        tool_config,
        current_session_id,
        state,
        overdue_only,
        include_archived,
    )?;
    let matched_count = session_ids.len();

    let mut tasks = Vec::new();
    for session_id in session_ids {
        if tasks.len() >= raw_limit {
            break;
        }
        let task =
            build_task_detail(memory_config, tool_config, current_session_id, &session_id).await?;
        tasks.push(task);
    }

    let returned_count = tasks.len();
    let payload = json!({
        "command": "list",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "filters": {
            "limit": raw_limit,
            "state": state,
            "overdue_only": overdue_only,
            "include_archived": include_archived,
        },
        "matched_count": matched_count,
        "returned_count": returned_count,
        "tasks": tasks,
    });
    Ok(payload)
}

async fn execute_status_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task_id: &str,
) -> CliResult<Value> {
    let executor = LegacyTaskStatusExecutor::new(
        memory_config.clone(),
        tool_config.clone(),
        current_session_id.to_owned(),
    );
    let workspace = task_status_workspace_context(tool_config)?;
    let execution = execute_task_status(
        &TaskStatusRequest {
            current_session_id: current_session_id.to_owned(),
            task_id: task_id.to_owned(),
        },
        workspace,
        &executor,
    )
    .await?;
    let mut task = execution.detail;
    if let Some(task_object) = task.as_object_mut() {
        task_object.insert(
            "spine".to_owned(),
            json!({
                "session_id": execution.session.session_id,
                "workspace": execution.session.workspace,
                "task_id": execution.task.task_id,
                "objective": execution.task.objective,
                "lifecycle": execution.task.lifecycle,
                "execution_mode": execution.task.execution_mode,
                "current_turn_id": execution.task.current_turn_id,
            }),
        );
    }
    let payload = json!({
        "command": "status",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "task": task,
    });
    Ok(payload)
}

struct LegacyTaskStatusExecutor {
    memory_config: mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: mvp::config::ToolConfig,
    current_session_id: String,
}

impl LegacyTaskStatusExecutor {
    fn new(
        memory_config: mvp::memory::runtime_config::MemoryRuntimeConfig,
        tool_config: mvp::config::ToolConfig,
        current_session_id: String,
    ) -> Self {
        Self {
            memory_config,
            tool_config,
            current_session_id,
        }
    }
}

#[async_trait]
impl AppProtocolTaskStatusExecutor for LegacyTaskStatusExecutor {
    async fn load_task_status(
        &self,
        request: AppProtocolRuntimeTaskStatusRequest,
    ) -> Result<AppProtocolRuntimeTaskStatusExecutorResult, String> {
        let detail = build_task_detail(
            &self.memory_config,
            &self.tool_config,
            self.current_session_id.as_str(),
            request.task_id.as_str(),
        )
        .await?;
        Ok(AppProtocolRuntimeTaskStatusExecutorResult { detail })
    }
}

fn task_status_workspace_context(
    tool_config: &mvp::config::ToolConfig,
) -> CliResult<AppProtocolWorkspaceContext> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let workspace_root = tool_config
        .configured_runtime_workspace_root()
        .or_else(|| tool_config.configured_file_root())
        .unwrap_or_else(|| cwd.clone());
    let workspace_root = dunce::canonicalize(&workspace_root).unwrap_or(workspace_root);
    let repo_root =
        resolve_git_repo_root(workspace_root.as_path()).unwrap_or_else(|_| workspace_root.clone());
    let worktree_root = workspace_root.clone();
    Ok(AppProtocolWorkspaceContext::new(
        workspace_root.clone(),
        repo_root,
        worktree_root,
        cwd,
        current_branch_identity(&workspace_root),
    ))
}

fn current_branch_identity(workspace_root: &std::path::Path) -> String {
    std::process::Command::new("git")
        .args(["-C"])
        .arg(workspace_root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            } else {
                None
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn resolve_git_repo_root(base_root: &std::path::Path) -> Result<PathBuf, String> {
    let output = std::process::Command::new("git")
        .args(["-C"])
        .arg(base_root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("spawn git command failed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let display_path = base_root.display();
        return Err(format!(
            "resolve git repo root from `{display_path}` failed: {stderr}"
        ));
    }

    let raw_stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed_stdout = raw_stdout.trim();
    if trimmed_stdout.is_empty() {
        let display_path = base_root.display();
        return Err(format!(
            "resolve git repo root from `{display_path}` returned empty output"
        ));
    }

    Ok(PathBuf::from(trimmed_stdout))
}

async fn execute_events_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task_id: &str,
    after_id: Option<i64>,
    limit: usize,
) -> CliResult<Value> {
    let task_target = resolve_cli_task_target(memory_config, current_session_id, task_id)?;
    let event_limit = limit.clamp(1, 200);
    let payload = json!({
        "task_id": task_target.task_id,
        "after_id": after_id,
        "limit": event_limit,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "task_events",
        payload,
    )?;
    let next_after_id = outcome
        .payload
        .get("next_after_id")
        .cloned()
        .unwrap_or(Value::Null);
    let events = outcome
        .payload
        .get("events")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let output = json!({
        "command": "events",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "task_id": task_target.task_id,
        "after_id": after_id,
        "next_after_id": next_after_id,
        "events": events,
    });
    Ok(output)
}

async fn execute_wait_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task_id: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
) -> CliResult<Value> {
    let task_target = resolve_cli_task_target(memory_config, current_session_id, task_id)?;
    let payload = json!({
        "task_id": task_target.task_id,
        "after_id": after_id,
        "timeout_ms": timeout_ms.clamp(1, 30_000),
    });
    let session_store_config = mvp::session::store::SessionStoreConfig::from(memory_config);
    let outcome = mvp::tools::wait_for_task_with_config(
        payload,
        current_session_id,
        &session_store_config,
        tool_config,
    )
    .await?;
    let task = build_task_detail(memory_config, tool_config, current_session_id, task_id).await?;
    let wait_payload = outcome.payload;
    let next_after_id = wait_payload
        .get("next_after_id")
        .cloned()
        .unwrap_or(Value::Null);
    let events = wait_payload
        .get("events")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let output = json!({
        "command": "wait",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "task_id": task_target.task_id,
        "wait_status": outcome.status,
        "after_id": after_id,
        "timeout_ms": timeout_ms.clamp(1, 30_000),
        "next_after_id": next_after_id,
        "events": events,
        "task": task,
    });
    Ok(output)
}

async fn execute_cancel_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task_id: &str,
    dry_run: bool,
) -> CliResult<Value> {
    let task_target = resolve_cli_task_target(memory_config, current_session_id, task_id)?;
    let status_payload =
        load_task_status_payload(memory_config, tool_config, current_session_id, &task_target)?;
    ensure_background_task_status_payload(&status_payload, task_id)?;
    let payload = json!({
        "task_id": task_target.task_id,
        "dry_run": dry_run,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "task_cancel",
        payload,
    )?;
    let (task, task_lookup_error) =
        build_best_effort_task_detail(memory_config, tool_config, current_session_id, task_id)
            .await;
    let mutation_result = extract_single_mutation_result(&outcome.payload);
    let result = mutation_result
        .as_ref()
        .and_then(|value| value.get("result"))
        .cloned()
        .or_else(|| outcome.payload.get("result").cloned())
        .unwrap_or(Value::Null);
    let message = mutation_result
        .as_ref()
        .and_then(|value| value.get("message"))
        .cloned()
        .or_else(|| outcome.payload.get("message").cloned())
        .unwrap_or(Value::Null);
    let action = outcome
        .payload
        .get("cancel_action")
        .cloned()
        .or_else(|| {
            mutation_result
                .as_ref()
                .and_then(|value| value.get("action"))
                .cloned()
        })
        .or_else(|| outcome.payload.get("action").cloned())
        .unwrap_or(Value::Null);
    let output = json!({
        "command": "cancel",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "dry_run": dry_run,
        "result": result,
        "message": message,
        "action": action,
        "task": task,
        "task_lookup_error": task_lookup_error,
    });
    Ok(output)
}

async fn execute_recover_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task_id: &str,
    dry_run: bool,
) -> CliResult<Value> {
    let task_target = resolve_cli_task_target(memory_config, current_session_id, task_id)?;
    let status_payload =
        load_task_status_payload(memory_config, tool_config, current_session_id, &task_target)?;
    ensure_background_task_status_payload(&status_payload, task_id)?;
    let payload = json!({
        "task_id": task_target.task_id,
        "dry_run": dry_run,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "task_recover",
        payload,
    )?;
    let (task, task_lookup_error) =
        build_best_effort_task_detail(memory_config, tool_config, current_session_id, task_id)
            .await;
    let mutation_result = extract_single_mutation_result(&outcome.payload);
    let result = mutation_result
        .as_ref()
        .and_then(|value| value.get("result"))
        .cloned()
        .or_else(|| outcome.payload.get("result").cloned())
        .unwrap_or(Value::Null);
    let message = mutation_result
        .as_ref()
        .and_then(|value| value.get("message"))
        .cloned()
        .or_else(|| outcome.payload.get("message").cloned())
        .unwrap_or(Value::Null);
    let action = outcome
        .payload
        .get("recovery_action")
        .cloned()
        .or_else(|| {
            mutation_result
                .as_ref()
                .and_then(|value| value.get("action"))
                .cloned()
        })
        .or_else(|| outcome.payload.get("action").cloned())
        .unwrap_or(Value::Null);
    let output = json!({
        "command": "recover",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "dry_run": dry_run,
        "result": result,
        "message": message,
        "action": action,
        "task": task,
        "task_lookup_error": task_lookup_error,
    });
    Ok(output)
}

fn extract_single_mutation_result(payload: &Value) -> Option<Value> {
    let results = payload.get("results")?.as_array()?;
    if results.len() != 1 {
        return None;
    }
    results.first().cloned()
}

fn normalize_session_scope(raw: &str) -> CliResult<String> {
    let session = raw.trim();
    if session.is_empty() {
        return Err("tasks CLI requires a non-empty session scope".to_owned());
    }
    Ok(session.to_owned())
}

fn resolve_session_scope(
    raw: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
) -> CliResult<String> {
    let session = normalize_session_scope(raw)?;
    let should_resolve_latest = session == mvp::session::LATEST_SESSION_SELECTOR;
    if !should_resolve_latest {
        return Ok(session);
    }

    let session_store_config = mvp::session::store::SessionStoreConfig::from(memory_config);
    let latest_session_id = mvp::session::latest_resumable_root_session_id(&session_store_config)?;
    let latest_session_id = latest_session_id.ok_or_else(|| {
        "tasks CLI session selector `latest` did not find any resumable root session".to_owned()
    })?;

    Ok(latest_session_id)
}

fn execute_app_tool_request(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    tool_name: &str,
    payload: Value,
) -> CliResult<ToolCoreOutcome> {
    let request = ToolCoreRequest {
        tool_name: tool_name.to_owned(),
        payload,
    };
    let session_store_config = mvp::session::store::SessionStoreConfig::from(memory_config);
    let outcome = mvp::tools::execute_app_tool_with_config(
        request,
        current_session_id,
        &session_store_config,
        tool_config,
    )?;
    Ok(outcome)
}

fn load_visible_background_task_ids(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    state: Option<&str>,
    overdue_only: bool,
    include_archived: bool,
) -> CliResult<Vec<String>> {
    let session_store_config = mvp::session::store::SessionStoreConfig::from(memory_config);
    let repo = mvp::session::repository::SessionRepository::new(&session_store_config)?;
    let mut sessions = repo.list_visible_sessions(current_session_id)?;
    if tool_config.sessions.visibility == mvp::config::SessionVisibility::SelfOnly {
        sessions.retain(|session| session.session_id == current_session_id);
    }
    if let Some(raw_state) = state {
        let required_state = parse_task_state_filter(raw_state)?;
        sessions.retain(|session| session.state == required_state);
    }
    sessions.retain(|session| session.kind == mvp::session::repository::SessionKind::DelegateChild);
    if !include_archived {
        sessions.retain(|session| session.archived_at.is_none());
    }

    let mut task_ids = Vec::new();
    for session in sessions {
        let status_summary = summarize_visible_background_task(&repo, &session)?;
        if !status_summary.is_background_task {
            continue;
        }
        if overdue_only && !status_summary.is_overdue {
            continue;
        }
        let task_id = session.session_id;
        task_ids.push(task_id);
    }
    Ok(task_ids)
}

fn summarize_visible_background_task(
    repo: &mvp::session::repository::SessionRepository,
    session: &mvp::session::repository::SessionSummaryRecord,
) -> CliResult<TaskStatusSummary> {
    let delegate_kind = mvp::session::repository::SessionKind::DelegateChild;
    if session.kind != delegate_kind {
        return Ok(TaskStatusSummary {
            is_background_task: false,
            is_overdue: false,
        });
    }

    let delegate_events = repo.list_delegate_lifecycle_events(&session.session_id)?;
    let mut queued_at = None;
    let mut started_at = None;
    let mut queued_timeout_seconds = None;
    let mut started_timeout_seconds = None;
    let mut execution_mode = None;

    for event in delegate_events {
        let event_kind = event.event_kind.as_str();
        let execution = mvp::conversation::ConstrainedSubagentExecution::from_event_payload(
            &event.payload_json,
        );
        let event_mode = execution.as_ref().map(|value| value.mode);
        let event_timeout_seconds = event
            .payload_json
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .or_else(|| execution.as_ref().map(|value| value.timeout_seconds));

        match event_kind {
            "delegate_queued" => {
                queued_at = Some(event.ts);
                if execution_mode.is_none() {
                    execution_mode = event_mode;
                }
                if queued_timeout_seconds.is_none() {
                    queued_timeout_seconds = event_timeout_seconds;
                }
            }
            "delegate_started" => {
                started_at = Some(event.ts);
                if execution_mode.is_none() {
                    execution_mode = event_mode;
                }
                if started_timeout_seconds.is_none() {
                    started_timeout_seconds = event_timeout_seconds;
                }
            }
            _ => {}
        }
    }

    let async_mode = mvp::conversation::ConstrainedSubagentMode::Async;
    let inline_mode = mvp::conversation::ConstrainedSubagentMode::Inline;
    let effective_mode = execution_mode.unwrap_or_else(|| {
        if queued_at.is_some() || session.state == mvp::session::repository::SessionState::Ready {
            async_mode
        } else {
            inline_mode
        }
    });
    let timeout_seconds = started_timeout_seconds.or(queued_timeout_seconds);
    let reference_at = match session.state {
        mvp::session::repository::SessionState::Ready => queued_at,
        mvp::session::repository::SessionState::Running => started_at.or(queued_at),
        mvp::session::repository::SessionState::Completed => None,
        mvp::session::repository::SessionState::Failed => None,
        mvp::session::repository::SessionState::TimedOut => None,
    };
    let now_ts = current_unix_timestamp();
    let is_overdue = match (reference_at, timeout_seconds) {
        (Some(reference_at), Some(timeout_seconds)) => {
            let elapsed_seconds = now_ts.saturating_sub(reference_at).max(0) as u64;
            elapsed_seconds > timeout_seconds
        }
        _ => false,
    };
    let is_background_task = effective_mode == async_mode;

    Ok(TaskStatusSummary {
        is_background_task,
        is_overdue,
    })
}

fn current_unix_timestamp() -> i64 {
    let now = SystemTime::now();
    let duration = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    duration.as_secs().min(i64::MAX as u64) as i64
}

async fn build_task_detail(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> CliResult<Value> {
    let task_target = resolve_cli_task_target(memory_config, current_session_id, task_id)?;
    let status_payload =
        load_task_status_payload(memory_config, tool_config, current_session_id, &task_target)?;
    ensure_background_task_status_payload(&status_payload, task_id)?;
    let (approvals_payload, approval_lookup_error) = load_best_effort_task_approvals_payload(
        memory_config,
        tool_config,
        current_session_id,
        &task_target,
    );
    let (tool_policy_payload, tool_policy_lookup_error) = load_best_effort_task_tool_policy_payload(
        memory_config,
        tool_config,
        current_session_id,
        &task_target,
    );

    let session = status_payload
        .get("session")
        .cloned()
        .ok_or_else(|| "task status payload missing session object".to_owned())?;
    let delegate = status_payload
        .get("delegate_lifecycle")
        .cloned()
        .unwrap_or(Value::Null);
    let label = session.get("label").cloned().unwrap_or(Value::Null);
    let session_state = session.get("state").cloned().unwrap_or(Value::Null);
    let phase = delegate.get("phase").cloned().unwrap_or(Value::Null);
    let mode = delegate.get("mode").cloned().unwrap_or(Value::Null);
    let owner_kind = delegate
        .get("execution")
        .and_then(|value| value.get("owner_kind"))
        .cloned()
        .unwrap_or(Value::Null);
    let timeout_seconds = delegate
        .get("timeout_seconds")
        .cloned()
        .unwrap_or(Value::Null);
    let workflow = status_payload
        .get("workflow")
        .cloned()
        .unwrap_or(Value::Null);
    let created_at = session.get("created_at").cloned().unwrap_or(Value::Null);
    let updated_at = session.get("updated_at").cloned().unwrap_or(Value::Null);
    let archived = session.get("archived").cloned().unwrap_or(Value::Null);
    let last_error = session.get("last_error").cloned().unwrap_or(Value::Null);
    let approval_requests = approvals_payload
        .get("requests")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let approval_attention_summary = approvals_payload
        .get("attention_summary")
        .cloned()
        .unwrap_or(Value::Null);
    let approval_matched_count = approvals_payload
        .get("matched_count")
        .cloned()
        .unwrap_or_else(|| json!(0));
    let approval_returned_count = approvals_payload
        .get("returned_count")
        .cloned()
        .unwrap_or_else(|| json!(0));
    let tool_policy = tool_policy_payload
        .get("policy")
        .cloned()
        .unwrap_or(Value::Null);
    let terminal_outcome_state = status_payload
        .get("terminal_outcome_state")
        .cloned()
        .unwrap_or(Value::Null);
    let terminal_outcome_missing_reason = status_payload
        .get("terminal_outcome_missing_reason")
        .cloned()
        .unwrap_or(Value::Null);
    let recovery = status_payload
        .get("recovery")
        .cloned()
        .unwrap_or(Value::Null);
    let terminal_outcome = status_payload
        .get("terminal_outcome")
        .cloned()
        .unwrap_or(Value::Null);
    let recent_events = status_payload
        .get("recent_events")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let task_status = build_task_status_payload(
        &session,
        &delegate,
        workflow.get("task_progress").unwrap_or(&Value::Null),
        &terminal_outcome_state,
        &recovery,
        &approval_requests,
        &approval_attention_summary,
        &tool_policy,
        &recent_events,
    );
    let prompt_frame = crate::session_prompt_frame_cli::load_session_prompt_frame_payload(
        memory_config,
        task_target.owner_session_id.as_str(),
    )
    .await;
    let safe_lane = crate::session_runtime_truth_cli::load_session_safe_lane_payload(
        memory_config,
        task_target.owner_session_id.as_str(),
    )
    .await;
    let turn_checkpoint = crate::session_runtime_truth_cli::load_session_turn_checkpoint_payload(
        memory_config,
        task_target.owner_session_id.as_str(),
    )
    .await;

    let detail = compose_task_detail_payload(
        current_session_id,
        &task_target,
        session,
        delegate,
        label,
        session_state,
        phase,
        mode,
        owner_kind,
        timeout_seconds,
        workflow,
        created_at,
        updated_at,
        archived,
        last_error,
        approval_requests,
        approval_attention_summary,
        approval_matched_count,
        approval_returned_count,
        approval_lookup_error,
        tool_policy,
        tool_policy_lookup_error,
        task_status,
        terminal_outcome_state,
        terminal_outcome_missing_reason,
        recovery,
        terminal_outcome,
        recent_events,
        prompt_frame,
        safe_lane,
        turn_checkpoint,
    );
    Ok(detail)
}

async fn build_best_effort_task_detail(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> (Value, Value) {
    let detail_result =
        build_task_detail(memory_config, tool_config, current_session_id, task_id).await;
    match detail_result {
        Ok(task_detail) => (task_detail, Value::Null),
        Err(error) => {
            let fallback_task = fallback_task_detail(current_session_id, task_id);
            let lookup_error = Value::String(error);
            (fallback_task, lookup_error)
        }
    }
}

fn fallback_task_detail(current_session_id: &str, task_id: &str) -> Value {
    let task_status = unknown_task_status_payload();
    json!({
        "task_id": task_id,
        "task_session_id": task_id,
        "owner_session_id": task_id,
        "scope_session_id": current_session_id,
        "label": Value::Null,
        "session_state": Value::Null,
        "phase": Value::Null,
        "mode": Value::Null,
        "owner_kind": Value::Null,
        "timeout_seconds": Value::Null,
        "workflow": Value::Null,
        "created_at": Value::Null,
        "updated_at": Value::Null,
        "archived": Value::Null,
        "last_error": Value::Null,
        "approval": {
            "matched_count": 0,
            "returned_count": 0,
            "attention_summary": Value::Null,
            "requests": [],
        },
        "approval_lookup_error": Value::Null,
        "tool_policy": Value::Null,
        "tool_policy_lookup_error": Value::Null,
        "task_status": task_status,
        "session": Value::Null,
        "delegate": Value::Null,
        "terminal_outcome_state": Value::Null,
        "terminal_outcome_missing_reason": Value::Null,
        "recovery": Value::Null,
        "terminal_outcome": Value::Null,
        "recent_events": [],
        "prompt_frame": Value::Null,
        "safe_lane": Value::Null,
        "turn_checkpoint": Value::Null,
    })
}

fn ensure_background_task_status_payload(status_payload: &Value, task_id: &str) -> CliResult<()> {
    let status_summary = summarize_task_status_payload(status_payload)?;
    if !status_summary.is_background_task {
        return Err(format!(
            "tasks_cli_not_background_task: session `{task_id}` is not an async delegate child"
        ));
    }
    Ok(())
}

fn parse_task_state_filter(raw_state: &str) -> CliResult<mvp::session::repository::SessionState> {
    match raw_state {
        "ready" => Ok(mvp::session::repository::SessionState::Ready),
        "running" => Ok(mvp::session::repository::SessionState::Running),
        "completed" => Ok(mvp::session::repository::SessionState::Completed),
        "failed" => Ok(mvp::session::repository::SessionState::Failed),
        "timed_out" => Ok(mvp::session::repository::SessionState::TimedOut),
        _ => Err(format!("invalid session tool payload.state: `{raw_state}`")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedCliTaskTarget {
    task_id: String,
    owner_session_id: String,
    task_session_id: String,
}

fn load_task_status_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_target: &ResolvedCliTaskTarget,
) -> CliResult<Value> {
    let payload = json!({
        "task_id": task_target.task_id,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "task_status",
        payload,
    )?;
    Ok(outcome.payload)
}

fn load_task_approvals_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_target: &ResolvedCliTaskTarget,
) -> CliResult<Value> {
    let payload = json!({
        "session_id": task_target.owner_session_id,
        "limit": 20,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "approval_requests_list",
        payload,
    )?;
    Ok(outcome.payload)
}

fn load_best_effort_task_approvals_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_target: &ResolvedCliTaskTarget,
) -> (Value, Value) {
    let result =
        load_task_approvals_payload(memory_config, tool_config, current_session_id, task_target);
    let fallback_payload = fallback_task_approvals_payload();
    best_effort_task_lookup_payload(result, fallback_payload)
}

fn load_task_tool_policy_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_target: &ResolvedCliTaskTarget,
) -> CliResult<Value> {
    let payload = json!({
        "session_id": task_target.owner_session_id,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "session_tool_policy_status",
        payload,
    )?;
    Ok(outcome.payload)
}

fn load_best_effort_task_tool_policy_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_target: &ResolvedCliTaskTarget,
) -> (Value, Value) {
    let result =
        load_task_tool_policy_payload(memory_config, tool_config, current_session_id, task_target);
    let fallback_payload = Value::Null;
    best_effort_task_lookup_payload(result, fallback_payload)
}

fn best_effort_task_lookup_payload(
    result: CliResult<Value>,
    fallback_payload: Value,
) -> (Value, Value) {
    match result {
        Ok(payload) => (payload, Value::Null),
        Err(error) => (fallback_payload, Value::String(error)),
    }
}

fn fallback_task_approvals_payload() -> Value {
    json!({
        "matched_count": 0,
        "returned_count": 0,
        "attention_summary": Value::Null,
        "requests": [],
    })
}

#[allow(clippy::too_many_arguments)]
fn compose_task_detail_payload(
    current_session_id: &str,
    task_target: &ResolvedCliTaskTarget,
    session: Value,
    delegate: Value,
    label: Value,
    session_state: Value,
    phase: Value,
    mode: Value,
    owner_kind: Value,
    timeout_seconds: Value,
    workflow: Value,
    created_at: Value,
    updated_at: Value,
    archived: Value,
    last_error: Value,
    approval_requests: Value,
    approval_attention_summary: Value,
    approval_matched_count: Value,
    approval_returned_count: Value,
    approval_lookup_error: Value,
    tool_policy: Value,
    tool_policy_lookup_error: Value,
    task_status: Value,
    terminal_outcome_state: Value,
    terminal_outcome_missing_reason: Value,
    recovery: Value,
    terminal_outcome: Value,
    recent_events: Value,
    prompt_frame: Value,
    safe_lane: Value,
    turn_checkpoint: Value,
) -> Value {
    json!({
        "task_id": task_target.task_id,
        "task_session_id": task_target.task_session_id,
        "owner_session_id": task_target.owner_session_id,
        "scope_session_id": current_session_id,
        "label": label,
        "session_state": session_state,
        "phase": phase,
        "mode": mode,
        "owner_kind": owner_kind,
        "timeout_seconds": timeout_seconds,
        "workflow": workflow,
        "created_at": created_at,
        "updated_at": updated_at,
        "archived": archived,
        "last_error": last_error,
        "approval": {
            "matched_count": approval_matched_count,
            "returned_count": approval_returned_count,
            "attention_summary": approval_attention_summary,
            "requests": approval_requests,
        },
        "approval_lookup_error": approval_lookup_error,
        "tool_policy": tool_policy,
        "tool_policy_lookup_error": tool_policy_lookup_error,
        "task_status": task_status,
        "session": session,
        "delegate": delegate,
        "terminal_outcome_state": terminal_outcome_state,
        "terminal_outcome_missing_reason": terminal_outcome_missing_reason,
        "recovery": recovery,
        "terminal_outcome": terminal_outcome,
        "recent_events": recent_events,
        "prompt_frame": prompt_frame,
        "safe_lane": safe_lane,
        "turn_checkpoint": turn_checkpoint,
    })
}

fn resolve_cli_task_target(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    current_session_id: &str,
    requested_task_id: &str,
) -> CliResult<ResolvedCliTaskTarget> {
    let session_store_config = mvp::session::store::SessionStoreConfig::from(memory_config);
    let repo = mvp::session::repository::SessionRepository::new(&session_store_config)?;
    let visible_sessions = repo.list_visible_sessions(current_session_id)?;

    for session in &visible_sessions {
        let task_identity =
            mvp::task_progress::resolve_task_identity_for_session(&repo, &session.session_id);
        if task_identity.task_id == requested_task_id {
            return Ok(ResolvedCliTaskTarget {
                task_id: task_identity.task_id,
                owner_session_id: task_identity.task_session_id.clone(),
                task_session_id: task_identity.task_session_id,
            });
        }
    }

    let fallback_session = repo
        .load_session_summary_with_legacy_fallback(requested_task_id)?
        .ok_or_else(|| format!("task_not_found: `{requested_task_id}`"))?;
    if !visible_sessions
        .iter()
        .any(|session| session.session_id == fallback_session.session_id)
    {
        return Err(format!(
            "visibility_denied: session `{}` is not visible from `{current_session_id}`",
            fallback_session.session_id
        ));
    }

    let task_identity =
        mvp::task_progress::resolve_task_identity_for_session(&repo, &fallback_session.session_id);
    let task_id = if task_identity.task_id.trim().is_empty() {
        requested_task_id.to_owned()
    } else {
        task_identity.task_id
    };
    Ok(ResolvedCliTaskTarget {
        task_id,
        owner_session_id: fallback_session.session_id,
        task_session_id: task_identity.task_session_id,
    })
}

fn build_task_recipes(
    resolved_config_path: &str,
    current_session_id: &str,
    task_id: &str,
) -> Vec<String> {
    let command_name = crate::active_cli_command_name();
    let config_arg = crate::cli_handoff::shell_quote_argument(resolved_config_path);
    let session_arg = crate::cli_handoff::shell_quote_argument(current_session_id);
    let task_arg = crate::cli_handoff::shell_quote_argument(task_id);

    let status_recipe = format!(
        "{command_name} tasks status --config {config_arg} --session {session_arg} {task_arg}"
    );
    let wait_recipe = format!(
        "{command_name} tasks wait --config {config_arg} --session {session_arg} {task_arg}"
    );
    let events_recipe = format!(
        "{command_name} tasks events --config {config_arg} --session {session_arg} {task_arg}"
    );

    vec![status_recipe, wait_recipe, events_recipe]
}

fn build_task_next_steps() -> Vec<String> {
    let step_one =
        "Use `tasks status` to inspect approval, policy narrowing, and lifecycle state.".to_owned();
    let step_two =
        "Use `tasks wait` for bounded progress checks or `tasks events` for raw lifecycle history."
            .to_owned();
    let step_three =
        "Use `tasks cancel` or `tasks recover` only after the task state confirms that the action is valid."
            .to_owned();
    vec![step_one, step_two, step_three]
}

fn required_string_field(value: &Value, field: &str, context: &str) -> CliResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{context} missing string field `{field}`"))
}

#[cfg(test)]
#[path = "tasks_cli_tests.rs"]
mod tests;

