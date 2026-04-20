use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use clap::Subcommand;
use kernel::ToolCoreRequest;
use loong_app as mvp;
use loong_contracts::ToolCoreOutcome;
use loong_spec::CliResult;
use serde_json::{Value, json};

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
    let task_id = required_string_field(&queued.payload, "child_session_id", "tasks create")?;
    let (task_detail, task_lookup_error) =
        build_best_effort_task_detail(memory_config, tool_config, current_session_id, &task_id)
            .await;
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
    let task = build_task_detail(memory_config, tool_config, current_session_id, task_id).await?;
    let payload = json!({
        "command": "status",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "task": task,
    });
    Ok(payload)
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
    let _ = build_task_detail(memory_config, tool_config, current_session_id, task_id).await?;
    let event_limit = limit.clamp(1, 200);
    let payload = json!({
        "session_id": task_id,
        "after_id": after_id,
        "limit": event_limit,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "session_events",
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
        "task_id": task_id,
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
    let _ = build_task_detail(memory_config, tool_config, current_session_id, task_id).await?;
    let payload = json!({
        "session_id": task_id,
        "after_id": after_id,
        "timeout_ms": timeout_ms.clamp(1, 30_000),
    });
    let outcome = mvp::tools::wait_for_session_with_config(
        payload,
        current_session_id,
        memory_config,
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
        "task_id": task_id,
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
    validate_background_task_target(memory_config, tool_config, current_session_id, task_id)?;
    let payload = json!({
        "session_id": task_id,
        "dry_run": dry_run,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "session_cancel",
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
        .unwrap_or(Value::Null);
    let message = mutation_result
        .as_ref()
        .and_then(|value| value.get("message"))
        .cloned()
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
    validate_background_task_target(memory_config, tool_config, current_session_id, task_id)?;
    let payload = json!({
        "session_id": task_id,
        "dry_run": dry_run,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "session_recover",
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
        .unwrap_or(Value::Null);
    let message = mutation_result
        .as_ref()
        .and_then(|value| value.get("message"))
        .cloned()
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

    let latest_session_id = mvp::session::latest_resumable_root_session_id(memory_config)?;
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
    let outcome = mvp::tools::execute_app_tool_with_config(
        request,
        current_session_id,
        memory_config,
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
    let repo = mvp::session::repository::SessionRepository::new(memory_config)?;
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
    let status_payload =
        load_task_status_payload(memory_config, tool_config, current_session_id, task_id)?;
    ensure_background_task_status_payload(&status_payload, task_id)?;
    let (approvals_payload, approval_lookup_error) = load_best_effort_task_approvals_payload(
        memory_config,
        tool_config,
        current_session_id,
        task_id,
    );
    let (tool_policy_payload, tool_policy_lookup_error) = load_best_effort_task_tool_policy_payload(
        memory_config,
        tool_config,
        current_session_id,
        task_id,
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
        &approval_requests,
        &approval_attention_summary,
        &tool_policy,
        &recent_events,
    );
    let prompt_frame =
        crate::session_prompt_frame_cli::load_session_prompt_frame_payload(memory_config, task_id)
            .await;
    let safe_lane =
        crate::session_runtime_truth_cli::load_session_safe_lane_payload(memory_config, task_id)
            .await;
    let turn_checkpoint = crate::session_runtime_truth_cli::load_session_turn_checkpoint_payload(
        memory_config,
        task_id,
    )
    .await;

    let detail = compose_task_detail_payload(
        current_session_id,
        task_id,
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
        "session_id": task_id,
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

fn validate_background_task_target(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> CliResult<()> {
    let status_payload =
        load_task_status_payload(memory_config, tool_config, current_session_id, task_id)?;
    ensure_background_task_status_payload(&status_payload, task_id)
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

fn summarize_task_status_payload(status_payload: &Value) -> CliResult<TaskStatusSummary> {
    let session = status_payload
        .get("session")
        .ok_or_else(|| "task status payload missing session object".to_owned())?;
    let delegate = status_payload
        .get("delegate_lifecycle")
        .cloned()
        .unwrap_or(Value::Null);
    let session_kind = session.get("kind").and_then(Value::as_str).unwrap_or("");
    let delegate_mode = delegate.get("mode").and_then(Value::as_str).unwrap_or("");
    let staleness_state = delegate
        .get("staleness")
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let is_background_task = session_kind == "delegate_child" && delegate_mode == "async";
    let is_overdue = staleness_state == "overdue";

    Ok(TaskStatusSummary {
        is_background_task,
        is_overdue,
    })
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

#[derive(Debug, Clone, Copy)]
struct TaskStatusSummary {
    is_background_task: bool,
    is_overdue: bool,
}

fn build_task_status_payload(
    session: &Value,
    delegate: &Value,
    approval_requests: &Value,
    approval_attention_summary: &Value,
    tool_policy: &Value,
    recent_events: &Value,
) -> Value {
    let session_state = session
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let phase = delegate
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let staleness_state = delegate
        .get("staleness")
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str);
    let cancellation_state = delegate
        .get("cancellation")
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str);
    let approval_attention_count = approval_attention_summary
        .get("needs_attention_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let has_approval_attention = approval_attention_count > 0;
    let approval_primary_action = primary_approval_action(approval_requests).map(ToOwned::to_owned);
    let recovered = recent_events_contains_kind(recent_events, "delegate_recovery_applied");
    let tool_narrowing_active = task_tool_narrowing_active(tool_policy);
    let kind = derive_task_status_kind(
        session_state,
        phase,
        staleness_state,
        cancellation_state,
        has_approval_attention,
    );
    let display = render_task_status_display(kind, recovered);
    let blocked = task_status_is_blocked(kind);
    let terminal = task_status_is_terminal(kind);
    let status = kind;
    let needs_attention = task_status_needs_attention(kind, approval_primary_action.as_deref());
    let next_action = task_status_next_action(kind, approval_primary_action.as_deref());
    let signals = build_task_status_signals(
        kind,
        recovered,
        tool_narrowing_active,
        has_approval_attention,
        staleness_state,
        cancellation_state,
    );

    json!({
        "status": status,
        "kind": kind,
        "display": display,
        "blocked": blocked,
        "terminal": terminal,
        "needs_attention": needs_attention,
        "next_action": next_action,
        "approval_primary_action": approval_primary_action,
        "recovered": recovered,
        "tool_narrowing_active": tool_narrowing_active,
        "signals": signals,
    })
}

fn unknown_task_status_payload() -> Value {
    json!({
        "status": "unknown",
        "kind": "unknown",
        "display": "unknown",
        "blocked": false,
        "terminal": false,
        "needs_attention": false,
        "next_action": "status",
        "approval_primary_action": Value::Null,
        "recovered": false,
        "tool_narrowing_active": false,
        "signals": [],
    })
}

fn derive_task_status_kind(
    session_state: &str,
    phase: &str,
    staleness_state: Option<&str>,
    cancellation_state: Option<&str>,
    has_approval_attention: bool,
) -> &'static str {
    if session_state == "completed" {
        return "completed";
    }

    if session_state == "failed" {
        return "failed";
    }

    if session_state == "timed_out" {
        return "timed_out";
    }

    let is_overdue = staleness_state == Some("overdue");
    if is_overdue {
        return "overdue";
    }

    let cancel_requested = cancellation_state == Some("requested");
    if cancel_requested {
        return "cancel_requested";
    }

    if has_approval_attention {
        return "approval_pending";
    }

    if session_state == "running" {
        return "running";
    }

    let queued_state = session_state == "ready";
    let queued_phase = phase == "queued";
    if queued_state || queued_phase {
        return "queued";
    }

    "unknown"
}

fn render_task_status_display(kind: &str, recovered: bool) -> String {
    let base = kind.to_owned();
    if !recovered {
        return base;
    }

    let display = format!("{base} (recovered)");
    display
}

fn task_status_is_blocked(kind: &str) -> bool {
    matches!(kind, "approval_pending" | "overdue")
}

fn task_status_is_terminal(kind: &str) -> bool {
    matches!(kind, "completed" | "failed" | "timed_out")
}

fn task_status_needs_attention(kind: &str, approval_primary_action: Option<&str>) -> bool {
    let status_requires_attention = matches!(
        kind,
        "approval_pending" | "overdue" | "failed" | "timed_out"
    );
    if status_requires_attention {
        return true;
    }

    approval_primary_action.is_some()
}

fn task_status_next_action(kind: &str, approval_primary_action: Option<&str>) -> String {
    if let Some(approval_primary_action) = approval_primary_action {
        let next_action = approval_primary_action.to_owned();
        return next_action;
    }

    match kind {
        "approval_pending" => "status".to_owned(),
        "overdue" => "recover".to_owned(),
        "queued" => "wait".to_owned(),
        "running" => "wait".to_owned(),
        "cancel_requested" => "wait".to_owned(),
        "completed" => "events".to_owned(),
        "failed" => "events".to_owned(),
        "timed_out" => "events".to_owned(),
        _ => "status".to_owned(),
    }
}

fn build_task_status_signals(
    kind: &str,
    recovered: bool,
    tool_narrowing_active: bool,
    has_approval_attention: bool,
    staleness_state: Option<&str>,
    cancellation_state: Option<&str>,
) -> Vec<String> {
    let mut signals = Vec::new();

    if has_approval_attention {
        signals.push("approval_pending".to_owned());
    }

    if staleness_state == Some("overdue") {
        signals.push("overdue".to_owned());
    }

    if cancellation_state == Some("requested") {
        signals.push("cancel_requested".to_owned());
    }

    if recovered {
        signals.push("recovered".to_owned());
    }

    if tool_narrowing_active {
        signals.push("tool_narrowing_active".to_owned());
    }

    let terminal = task_status_is_terminal(kind);
    if terminal {
        signals.push("terminal".to_owned());
    }

    signals
}

fn recent_events_contains_kind(recent_events: &Value, expected_kind: &str) -> bool {
    let Some(events) = recent_events.as_array() else {
        return false;
    };

    for event in events {
        let event_kind = event
            .get("event_kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        let matches_kind = event_kind == expected_kind;
        if matches_kind {
            return true;
        }
    }

    false
}

fn primary_approval_action(approval_requests: &Value) -> Option<&str> {
    let requests = approval_requests.as_array()?;

    for request in requests {
        let action = request
            .get("attention")
            .and_then(|value| value.get("primary_action"))
            .and_then(Value::as_str);
        if action.is_some() {
            return action;
        }
    }

    None
}

fn task_tool_narrowing_active(tool_policy: &Value) -> bool {
    let effective_tool_ids = tool_policy
        .get("effective_tool_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let base_tool_ids = tool_policy
        .get("base_tool_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let runtime_narrowing = tool_policy.get("effective_runtime_narrowing");
    let runtime_narrowing = runtime_narrowing.cloned().unwrap_or(Value::Null);
    let tool_ids_changed = effective_tool_ids != base_tool_ids;
    let runtime_narrowing_active = !runtime_narrowing.is_null();

    tool_ids_changed || runtime_narrowing_active
}

fn load_task_status_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> CliResult<Value> {
    let payload = json!({
        "session_id": task_id,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "session_status",
        payload,
    )?;
    Ok(outcome.payload)
}

fn load_task_approvals_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> CliResult<Value> {
    let payload = json!({
        "session_id": task_id,
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
    task_id: &str,
) -> (Value, Value) {
    let result =
        load_task_approvals_payload(memory_config, tool_config, current_session_id, task_id);
    let fallback_payload = fallback_task_approvals_payload();
    best_effort_task_lookup_payload(result, fallback_payload)
}

fn load_task_tool_policy_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> CliResult<Value> {
    let payload = json!({
        "session_id": task_id,
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
    task_id: &str,
) -> (Value, Value) {
    let result =
        load_task_tool_policy_payload(memory_config, tool_config, current_session_id, task_id);
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
    task_id: &str,
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
        "task_id": task_id,
        "session_id": task_id,
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
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context} missing string field `{field}`"))?;
    Ok(text.to_owned())
}

pub fn render_tasks_cli_text(execution: &TasksCommandExecution) -> CliResult<String> {
    let command = execution
        .payload
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "tasks CLI payload missing command".to_owned())?;

    let rendered = match command {
        "create" => render_tasks_create_text(&execution.payload)?,
        "list" => render_tasks_list_text(&execution.payload)?,
        "status" => render_tasks_status_text(&execution.payload)?,
        "events" => render_tasks_events_text(&execution.payload)?,
        "wait" => render_tasks_wait_text(&execution.payload)?,
        "cancel" | "recover" => render_tasks_mutation_text(&execution.payload)?,
        other => {
            return Err(format!("unknown tasks CLI render command `{other}`"));
        }
    };
    Ok(rendered)
}

fn render_tasks_create_text(payload: &Value) -> CliResult<String> {
    let task = payload
        .get("task")
        .ok_or_else(|| "tasks create payload missing task".to_owned())?;
    let recipes = payload
        .get("recipes")
        .and_then(Value::as_array)
        .ok_or_else(|| "tasks create payload missing recipes".to_owned())?;
    let next_steps = payload
        .get("next_steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "tasks create payload missing next_steps".to_owned())?;
    let scope = payload
        .get("current_session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let mut lines = Vec::new();
    let sanitized_scope = crate::sessions_cli::sanitize_terminal_text(scope);
    lines.push(format!(
        "background task queued in session `{sanitized_scope}`"
    ));
    lines.extend(render_task_detail_lines(task)?);
    append_task_lookup_error_line(payload, &mut lines);

    if !recipes.is_empty() {
        for recipe in recipes {
            let text = recipe.as_str().unwrap_or("");
            let sanitized_text = crate::sessions_cli::sanitize_terminal_text(text);
            lines.push(format!("- {sanitized_text}"));
        }
    }

    let mut next_lines = Vec::new();
    if !next_steps.is_empty() {
        for step in next_steps {
            let text = step.as_str().unwrap_or("");
            let sanitized_text = crate::sessions_cli::sanitize_terminal_text(text);
            next_lines.push(format!("- {sanitized_text}"));
        }
    }

    let mut sections = Vec::new();
    if !next_lines.is_empty() {
        sections.push(("next steps", next_lines));
    }
    sections.push(("queued task", lines));
    Ok(render_tasks_surface(
        "task queued",
        "background tasks",
        Vec::new(),
        sections,
        vec![
            "Use the next-step commands to inspect, wait on, or cancel the queued task.".to_owned(),
        ],
    ))
}

fn render_tasks_list_text(payload: &Value) -> CliResult<String> {
    let tasks = payload
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or_else(|| "tasks list payload missing tasks array".to_owned())?;
    let matched_count = payload
        .get("matched_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let returned_count = payload
        .get("returned_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let scope = payload
        .get("current_session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let mut lines = Vec::new();
    let sanitized_scope = crate::sessions_cli::sanitize_terminal_text(scope);
    lines.push(format!(
        "visible background tasks from session `{sanitized_scope}`: {returned_count}/{matched_count}"
    ));
    if tasks.is_empty() {
        lines.push("No async background tasks are currently visible.".to_owned());
        return Ok(render_tasks_surface(
            "visible tasks",
            "background tasks",
            Vec::new(),
            vec![("tasks", lines)],
            vec![
                "Use `tasks create` to queue a new background delegate from the current session."
                    .to_owned(),
            ],
        ));
    }

    for task in tasks {
        let line = render_task_brief_line(task)?;
        lines.push(format!("- {line}"));
    }

    Ok(render_tasks_surface(
        "visible tasks",
        "background tasks",
        Vec::new(),
        vec![("tasks", lines)],
        vec![
            "Use `tasks status <id>` for one task or `tasks wait <id>` to follow it incrementally."
                .to_owned(),
        ],
    ))
}

fn render_tasks_status_text(payload: &Value) -> CliResult<String> {
    let task = payload
        .get("task")
        .ok_or_else(|| "tasks status payload missing task".to_owned())?;
    let lines = render_task_detail_lines(task)?;
    Ok(render_tasks_surface(
        "task detail",
        "background tasks",
        Vec::new(),
        vec![("task", lines)],
        vec![
            "Use `tasks events <id>` or `tasks wait <id>` to keep inspecting the task lifecycle."
                .to_owned(),
        ],
    ))
}

fn render_tasks_events_text(payload: &Value) -> CliResult<String> {
    let task_id = payload
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let events = payload
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| "tasks events payload missing events array".to_owned())?;
    let next_after_id = payload
        .get("next_after_id")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let mut lines = Vec::new();
    let sanitized_task_id = crate::sessions_cli::sanitize_terminal_text(task_id);
    lines.push(format!(
        "events for `{sanitized_task_id}` (next_after_id={next_after_id})"
    ));
    if events.is_empty() {
        lines.push("No newer events.".to_owned());
        return Ok(render_tasks_surface(
            "task events",
            "background tasks",
            Vec::new(),
            vec![("events", lines)],
            vec!["Use `tasks wait <id>` to continue following this task.".to_owned()],
        ));
    }

    for event in events {
        let event_id = event.get("id").and_then(Value::as_i64).unwrap_or_default();
        let event_kind = event
            .get("event_kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let ts = event.get("ts").and_then(Value::as_i64).unwrap_or_default();
        let sanitized_event_kind = crate::sessions_cli::sanitize_terminal_text(event_kind);
        lines.push(format!("- #{event_id} {sanitized_event_kind} ts={ts}"));
    }

    Ok(render_tasks_surface(
        "task events",
        "background tasks",
        Vec::new(),
        vec![("events", lines)],
        vec!["Use `tasks wait <id>` to continue following this task.".to_owned()],
    ))
}

fn render_tasks_wait_text(payload: &Value) -> CliResult<String> {
    let wait_status = payload
        .get("wait_status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let task = payload
        .get("task")
        .ok_or_else(|| "tasks wait payload missing task".to_owned())?;
    let events = payload
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| "tasks wait payload missing events array".to_owned())?;
    let next_after_id = payload
        .get("next_after_id")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let mut lines = Vec::new();
    lines.push(format!(
        "wait result: {wait_status} (next_after_id={next_after_id})"
    ));
    lines.extend(render_task_detail_lines(task)?);
    if !events.is_empty() {
        lines.push("observed events:".to_owned());
        for event in events {
            let event_id = event.get("id").and_then(Value::as_i64).unwrap_or_default();
            let event_kind = event
                .get("event_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let sanitized_event_kind = crate::sessions_cli::sanitize_terminal_text(event_kind);
            lines.push(format!("- #{event_id} {sanitized_event_kind}"));
        }
    }

    Ok(render_tasks_surface(
        "task wait",
        "background tasks",
        Vec::new(),
        vec![("result", lines)],
        vec!["Re-run `tasks wait` with the returned cursor when you need more updates.".to_owned()],
    ))
}

fn render_tasks_mutation_text(payload: &Value) -> CliResult<String> {
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let task = payload
        .get("task")
        .ok_or_else(|| "tasks mutation payload missing task".to_owned())?;
    let action = payload.get("action").cloned().unwrap_or(Value::Null);
    let dry_run = payload
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result = payload.get("result").and_then(Value::as_str);
    let message = payload.get("message").and_then(Value::as_str);

    let mut lines = Vec::new();
    lines.push(format!("{command} dry_run={dry_run}"));
    if let Some(result) = result {
        let sanitized_result = crate::sessions_cli::sanitize_terminal_text(result);
        lines.push(format!("result: {sanitized_result}"));
    }
    if let Some(message) = message {
        let sanitized_message = crate::sessions_cli::sanitize_terminal_text(message);
        lines.push(format!("message: {sanitized_message}"));
    }
    if !action.is_null() {
        let rendered_action = serde_json::to_string_pretty(&action)
            .map_err(|error| format!("render action failed: {error}"))?;
        lines.push("action:".to_owned());
        lines.push(rendered_action);
    }
    lines.extend(render_task_detail_lines(task)?);
    append_task_lookup_error_line(payload, &mut lines);
    Ok(render_tasks_surface(
        "task action",
        "background tasks",
        Vec::new(),
        vec![("action result", lines)],
        vec!["Use `tasks status <id>` to verify the task state after the action.".to_owned()],
    ))
}

fn render_tasks_surface(
    title: &str,
    subtitle: &str,
    intro_lines: Vec<String>,
    sections: Vec<(&str, Vec<String>)>,
    footer_lines: Vec<String>,
) -> String {
    let sections = sections
        .into_iter()
        .map(
            |(section_title, lines)| mvp::tui_surface::TuiSectionSpec::Narrative {
                title: Some(section_title.to_owned()),
                lines,
            },
        )
        .collect();
    let screen = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some(subtitle.to_owned()),
        title: Some(title.to_owned()),
        progress_line: None,
        intro_lines,
        sections,
        choices: Vec::new(),
        footer_lines,
    };
    mvp::tui_surface::render_tui_screen_spec_ratatui(
        &screen,
        mvp::presentation::detect_render_width(),
        false,
    )
    .join("\n")
}

fn render_task_brief_line(task: &Value) -> CliResult<String> {
    let task_id = required_string_field(task, "task_id", "task summary")?;
    let state = task
        .get("session_state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let phase = task
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let task_status = task
        .get("task_status")
        .cloned()
        .unwrap_or_else(unknown_task_status_payload);
    let status_display = task_status
        .get("display")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let blocked = task_status
        .get("blocked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let workflow_phase = task
        .get("workflow")
        .and_then(|value| value.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let label = task.get("label").and_then(Value::as_str).unwrap_or("-");
    let approval_attention = task
        .get("approval")
        .and_then(|value| value.get("attention_summary"))
        .and_then(|value| value.get("needs_attention_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let owner_kind = task
        .get("owner_kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let signals = task_status
        .get("signals")
        .and_then(Value::as_array)
        .map(|values| render_string_array(values))
        .unwrap_or_else(|| "-".to_owned());
    let sanitized_task_id = crate::sessions_cli::sanitize_terminal_text(task_id.as_str());
    let sanitized_label = crate::sessions_cli::sanitize_terminal_text(label);
    let sanitized_owner_kind = crate::sessions_cli::sanitize_terminal_text(owner_kind);
    let line = format!(
        "{sanitized_task_id} status={status_display} blocked={blocked} state={state} workflow_phase={workflow_phase} delegate_phase={phase} label={sanitized_label} owner_kind={sanitized_owner_kind} approval_attention={approval_attention} signals={signals}"
    );
    Ok(line)
}

fn render_task_detail_lines(task: &Value) -> CliResult<Vec<String>> {
    let task_id = required_string_field(task, "task_id", "task detail")?;
    let scope_session_id = task
        .get("scope_session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let task_status = task
        .get("task_status")
        .cloned()
        .unwrap_or_else(unknown_task_status_payload);
    let task_status_display = task_status
        .get("display")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let blocked = task_status
        .get("blocked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let needs_attention = task_status
        .get("needs_attention")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let next_action = task_status
        .get("next_action")
        .and_then(Value::as_str)
        .unwrap_or("status");
    let task_signals = task_status
        .get("signals")
        .and_then(Value::as_array)
        .map(|values| render_string_array(values))
        .unwrap_or_else(|| "-".to_owned());
    let label = task.get("label").and_then(Value::as_str).unwrap_or("-");
    let state = task
        .get("session_state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let phase = task
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let owner_kind = task
        .get("owner_kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let workflow_id = task
        .get("workflow")
        .and_then(|value| value.get("workflow_id"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_phase = task
        .get("workflow")
        .and_then(|value| value.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_operation_kind = task
        .get("workflow")
        .and_then(|value| value.get("operation_kind"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_operation_scope = task
        .get("workflow")
        .and_then(|value| value.get("operation_scope"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_task_session_id = task
        .get("workflow")
        .and_then(|value| value.get("task_session_id"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_binding_mode = task
        .get("workflow")
        .and_then(|value| value.get("binding"))
        .and_then(|value| value.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_execution_surface = task
        .get("workflow")
        .and_then(|value| value.get("binding"))
        .and_then(|value| value.get("execution_surface"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_worktree_id = task
        .get("workflow")
        .and_then(|value| value.get("binding"))
        .and_then(|value| value.get("worktree"))
        .and_then(|value| value.get("worktree_id"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_workspace_root = task
        .get("workflow")
        .and_then(|value| value.get("binding"))
        .and_then(|value| value.get("worktree"))
        .and_then(|value| value.get("workspace_root"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let timeout_seconds = task
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let last_error = task
        .get("last_error")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let approval_total = task
        .get("approval")
        .and_then(|value| value.get("matched_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let approval_attention = task
        .get("approval")
        .and_then(|value| value.get("attention_summary"))
        .and_then(|value| value.get("needs_attention_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let requested_tool_ids =
        render_task_tool_policy_tool_ids(task, "visible_requested_tool_ids", "requested_tool_ids");
    let effective_tool_ids =
        render_task_tool_policy_tool_ids(task, "visible_effective_tool_ids", "effective_tool_ids");
    let effective_runtime_narrowing = task
        .get("tool_policy")
        .and_then(|value| value.get("effective_runtime_narrowing"))
        .cloned()
        .unwrap_or(Value::Null);
    let prompt_frame_summary =
        crate::session_prompt_frame_cli::render_prompt_frame_summary(task.get("prompt_frame"));
    let safe_lane_summary =
        crate::session_runtime_truth_cli::render_safe_lane_summary(task.get("safe_lane"));
    let turn_checkpoint_summary = crate::session_runtime_truth_cli::render_turn_checkpoint_summary(
        task.get("turn_checkpoint"),
    );
    let rendered_runtime_narrowing = if effective_runtime_narrowing.is_null() {
        "-".to_owned()
    } else {
        serde_json::to_string(&effective_runtime_narrowing)
            .map_err(|error| format!("render runtime narrowing failed: {error}"))?
    };
    let sanitized_task_id = crate::sessions_cli::sanitize_terminal_text(task_id.as_str());
    let sanitized_scope_session_id = crate::sessions_cli::sanitize_terminal_text(scope_session_id);
    let sanitized_label = crate::sessions_cli::sanitize_terminal_text(label);
    let sanitized_last_error = crate::sessions_cli::sanitize_terminal_text(last_error);
    let sanitized_requested_tool_ids =
        crate::sessions_cli::sanitize_terminal_text(requested_tool_ids.as_str());
    let sanitized_effective_tool_ids =
        crate::sessions_cli::sanitize_terminal_text(effective_tool_ids.as_str());
    let sanitized_runtime_narrowing =
        crate::sessions_cli::sanitize_terminal_text(rendered_runtime_narrowing.as_str());
    let sanitized_prompt_frame_summary =
        crate::sessions_cli::sanitize_terminal_text(prompt_frame_summary.as_str());
    let sanitized_safe_lane_summary =
        crate::sessions_cli::sanitize_terminal_text(safe_lane_summary.as_str());
    let sanitized_turn_checkpoint_summary =
        crate::sessions_cli::sanitize_terminal_text(turn_checkpoint_summary.as_str());
    let approval_lookup_error = task
        .get("approval_lookup_error")
        .and_then(Value::as_str)
        .map(crate::sessions_cli::sanitize_terminal_text);
    let tool_policy_lookup_error = task
        .get("tool_policy_lookup_error")
        .and_then(Value::as_str)
        .map(crate::sessions_cli::sanitize_terminal_text);

    let mut lines = Vec::new();
    lines.push(format!("task_id: {sanitized_task_id}"));
    lines.push(format!("scope_session_id: {sanitized_scope_session_id}"));
    lines.push(format!("label: {sanitized_label}"));
    lines.push(format!("task_status: {task_status_display}"));
    lines.push(format!("task_blocked: {blocked}"));
    lines.push(format!("task_needs_attention: {needs_attention}"));
    lines.push(format!("task_next_action: {next_action}"));
    lines.push(format!("task_signals: {task_signals}"));
    lines.push(format!("state: {state}"));
    lines.push(format!("workflow_id: {workflow_id}"));
    lines.push(format!("workflow_phase: {workflow_phase}"));
    lines.push(format!(
        "workflow_operation_kind: {workflow_operation_kind}"
    ));
    lines.push(format!(
        "workflow_operation_scope: {workflow_operation_scope}"
    ));
    lines.push(format!(
        "workflow_task_session_id: {workflow_task_session_id}"
    ));
    lines.push(format!("workflow_binding_mode: {workflow_binding_mode}"));
    lines.push(format!(
        "workflow_execution_surface: {workflow_execution_surface}"
    ));
    lines.push(format!("workflow_worktree_id: {workflow_worktree_id}"));
    lines.push(format!(
        "workflow_workspace_root: {workflow_workspace_root}"
    ));
    lines.push(format!("phase: {phase}"));
    lines.push(format!("owner_kind: {owner_kind}"));
    lines.push(format!("timeout_seconds: {timeout_seconds}"));
    lines.push(format!("last_error: {sanitized_last_error}"));
    lines.push(format!("approval_requests: {approval_total}"));
    lines.push(format!("approval_attention: {approval_attention}"));
    if requested_tool_ids != "-" {
        lines.push(format!(
            "requested_tool_ids: {sanitized_requested_tool_ids}"
        ));
    }
    lines.push(format!(
        "effective_tool_ids: {sanitized_effective_tool_ids}"
    ));
    lines.push(format!(
        "effective_runtime_narrowing: {sanitized_runtime_narrowing}"
    ));
    lines.push(format!("prompt_frame: {sanitized_prompt_frame_summary}"));
    lines.push(format!("safe_lane: {sanitized_safe_lane_summary}"));
    lines.push(format!(
        "turn_checkpoint: {sanitized_turn_checkpoint_summary}"
    ));
    if let Some(approval_lookup_error) = approval_lookup_error {
        lines.push(format!("approval_lookup_error: {approval_lookup_error}"));
    }
    if let Some(tool_policy_lookup_error) = tool_policy_lookup_error {
        lines.push(format!(
            "tool_policy_lookup_error: {tool_policy_lookup_error}"
        ));
    }
    Ok(lines)
}

fn append_task_lookup_error_line(payload: &Value, lines: &mut Vec<String>) {
    let Some(task_lookup_error) = payload.get("task_lookup_error").and_then(Value::as_str) else {
        return;
    };
    let sanitized_task_lookup_error =
        crate::sessions_cli::sanitize_terminal_text(task_lookup_error);
    lines.push(format!("task_lookup_error: {sanitized_task_lookup_error}"));
}

fn render_task_tool_policy_tool_ids(task: &Value, visible_field: &str, raw_field: &str) -> String {
    task.get("tool_policy")
        .and_then(|value| value.get(visible_field))
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .map(|values| render_string_array(values))
        .or_else(|| {
            task.get("tool_policy")
                .and_then(|value| value.get(raw_field))
                .and_then(Value::as_array)
                .filter(|values| !values.is_empty())
                .map(|values| render_string_array(values))
        })
        .unwrap_or_else(|| "-".to_owned())
}

fn render_string_array(values: &[Value]) -> String {
    let mut items = Vec::new();
    for value in values {
        if let Some(text) = value.as_str() {
            items.push(text.to_owned());
        }
    }
    if items.is_empty() {
        return "-".to_owned();
    }
    items.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mvp;

    fn build_task_payload(
        session_state: &str,
        phase: &str,
        approval_primary_action: Option<&str>,
        tool_narrowing_active: bool,
        recovered: bool,
        staleness_state: Option<&str>,
    ) -> Value {
        let approval_requests = approval_primary_action
            .map(|primary_action| {
                vec![json!({
                    "attention": {
                        "primary_action": primary_action,
                    },
                })]
            })
            .unwrap_or_default();
        let approval_summary = json!({
            "needs_attention_count": u64::from(approval_primary_action.is_some()),
        });
        let tool_policy = if tool_narrowing_active {
            json!({
                "base_tool_ids": ["file.read", "web.fetch"],
                "effective_tool_ids": ["file.read"],
                "effective_runtime_narrowing": {
                    "web_fetch": {
                        "allowed_domains": ["docs.example.com"],
                    },
                },
            })
        } else {
            json!({
                "base_tool_ids": ["file.read"],
                "effective_tool_ids": ["file.read"],
                "effective_runtime_narrowing": Value::Null,
            })
        };
        let recent_events = if recovered {
            json!([
                {
                    "event_kind": "delegate_recovery_applied",
                }
            ])
        } else {
            json!([])
        };
        let delegate = json!({
            "phase": phase,
            "staleness": staleness_state.map(|value| {
                json!({
                    "state": value,
                })
            }),
            "cancellation": Value::Null,
        });
        let session = json!({
            "state": session_state,
        });
        let task_status = build_task_status_payload(
            &session,
            &delegate,
            &json!(approval_requests),
            &approval_summary,
            &tool_policy,
            &recent_events,
        );

        json!({
            "task_id": "delegate:task-1",
            "scope_session_id": "ops-root",
            "label": "Release Check",
            "session_state": session_state,
            "phase": phase,
            "timeout_seconds": 60,
            "last_error": Value::Null,
            "approval": {
                "matched_count": approval_requests.len(),
                "attention_summary": approval_summary,
            },
            "tool_policy": tool_policy,
            "task_status": task_status,
        })
    }

    #[test]
    fn build_task_status_payload_uses_approval_action_and_tool_narrowing_signal() {
        let task = build_task_payload(
            "ready",
            "queued",
            Some("resolve_request"),
            true,
            false,
            None,
        );
        let task_status = &task["task_status"];

        assert_eq!(task_status["kind"], "approval_pending");
        assert_eq!(task_status["blocked"], true);
        assert_eq!(task_status["status"], "approval_pending");
        assert_eq!(task_status["needs_attention"], true);
        assert_eq!(task_status["next_action"], "resolve_request");
        assert_eq!(task_status["tool_narrowing_active"], true);
        assert!(
            task_status["signals"]
                .as_array()
                .expect("signals array")
                .iter()
                .any(|value| value == "tool_narrowing_active"),
            "signals should include narrowing"
        );
    }

    #[test]
    fn build_task_status_payload_marks_failed_task_as_recovered_when_event_present() {
        let task = build_task_payload("failed", "failed", None, false, true, None);
        let task_status = &task["task_status"];

        assert_eq!(task_status["status"], "failed");
        assert_eq!(task_status["kind"], "failed");
        assert_eq!(task_status["display"], "failed (recovered)");
        assert_eq!(task_status["needs_attention"], true);
        assert_eq!(task_status["recovered"], true);
        assert_eq!(task_status["next_action"], "events");
    }

    #[test]
    fn build_task_status_payload_marks_overdue_task_recoverable() {
        let task = build_task_payload("running", "running", None, false, false, Some("overdue"));
        let task_status = &task["task_status"];

        assert_eq!(task_status["kind"], "overdue");
        assert_eq!(task_status["blocked"], true);
        assert_eq!(task_status["status"], "overdue");
        assert_eq!(task_status["needs_attention"], true);
        assert_eq!(task_status["next_action"], "recover");
    }

    #[test]
    fn render_task_detail_lines_surface_task_status_summary() {
        let task = build_task_payload(
            "ready",
            "queued",
            Some("resolve_request"),
            true,
            false,
            None,
        );
        let rendered = render_task_detail_lines(&task).expect("render task detail");
        let joined = rendered.join("\n");

        assert!(joined.contains("task_status: approval_pending"));
        assert!(joined.contains("task_blocked: true"));
        assert!(joined.contains("task_needs_attention: true"));
        assert!(joined.contains("task_next_action: resolve_request"));
        assert!(joined.contains("task_signals: approval_pending, tool_narrowing_active"));
    }

    #[test]
    fn render_task_brief_line_prefers_derived_task_status_summary() {
        let task = build_task_payload(
            "ready",
            "queued",
            Some("resolve_request"),
            false,
            false,
            None,
        );
        let rendered = render_task_brief_line(&task).expect("render task brief");

        assert!(rendered.contains("status=approval_pending"));
        assert!(rendered.contains("blocked=true"));
        assert!(rendered.contains("signals=approval_pending"));
    }

    #[test]
    fn best_effort_task_approvals_payload_falls_back_when_session_tools_are_disabled() {
        let memory_config = mvp::memory::runtime_config::MemoryRuntimeConfig::default();
        let mut tool_config = mvp::config::ToolConfig::default();
        tool_config.sessions.enabled = false;

        let (payload, lookup_error) = load_best_effort_task_approvals_payload(
            &memory_config,
            &tool_config,
            "ops-root",
            "delegate:task-1",
        );

        assert_eq!(payload["matched_count"], 0);
        assert_eq!(payload["returned_count"], 0);
        assert_eq!(payload["requests"], json!([]));
        assert!(
            lookup_error
                .as_str()
                .expect("lookup error")
                .contains("session tools are disabled"),
            "expected degraded approval lookup error, got: {lookup_error:?}"
        );
    }

    #[test]
    fn best_effort_task_tool_policy_payload_falls_back_when_session_tools_are_disabled() {
        let memory_config = mvp::memory::runtime_config::MemoryRuntimeConfig::default();
        let mut tool_config = mvp::config::ToolConfig::default();
        tool_config.sessions.enabled = false;

        let (payload, lookup_error) = load_best_effort_task_tool_policy_payload(
            &memory_config,
            &tool_config,
            "ops-root",
            "delegate:task-1",
        );

        assert!(
            payload.is_null(),
            "expected null fallback payload, got: {payload:?}"
        );
        assert!(
            lookup_error
                .as_str()
                .expect("lookup error")
                .contains("session tools are disabled"),
            "expected degraded tool-policy lookup error, got: {lookup_error:?}"
        );
    }

    #[test]
    fn bootstrap_tasks_runtime_kernel_provides_kernel_bound_binding() {
        let mut config = mvp::config::LoongConfig::default();
        config.audit.mode = mvp::config::AuditMode::InMemory;

        let runtime_kernel =
            bootstrap_tasks_runtime_kernel(&config).expect("bootstrap tasks runtime kernel");
        let binding = runtime_kernel.conversation_binding();

        assert!(binding.is_kernel_bound());
        assert_eq!(runtime_kernel.kernel_context().agent_id(), "cli-tasks");
    }

    #[test]
    fn compose_task_detail_payload_keeps_core_status_truth_when_secondary_lookups_degrade() {
        let session = json!({
            "session_id": "delegate:task-1",
            "kind": "delegate_child",
            "state": "running",
            "created_at": 10,
            "updated_at": 20,
            "archived": false,
            "label": "Release Check",
            "last_error": Value::Null,
        });
        let delegate = json!({
            "mode": "async",
            "phase": "running",
            "execution": {
                "owner_kind": "background_task_host"
            },
            "timeout_seconds": 60
        });
        let detail = compose_task_detail_payload(
            "ops-root",
            "delegate:task-1",
            session.clone(),
            delegate.clone(),
            json!("Release Check"),
            json!("running"),
            json!("running"),
            json!("async"),
            json!("background_task_host"),
            json!(60),
            Value::Null,
            json!(10),
            json!(20),
            json!(false),
            Value::Null,
            json!([]),
            Value::Null,
            json!(0),
            json!(0),
            json!("approval lookup failed"),
            Value::Null,
            json!("tool policy lookup failed"),
            unknown_task_status_payload(),
            json!("missing"),
            json!("not_terminal"),
            Value::Null,
            Value::Null,
            json!([]),
            Value::Null,
            Value::Null,
            Value::Null,
        );

        assert_eq!(detail["session"], session);
        assert_eq!(detail["delegate"], delegate);
        assert_eq!(detail["task_id"], "delegate:task-1");
        assert_eq!(detail["approval_lookup_error"], "approval lookup failed");
        assert_eq!(
            detail["tool_policy_lookup_error"],
            "tool policy lookup failed"
        );
        assert_eq!(detail["tool_policy"], Value::Null);
        assert_eq!(detail["approval"]["matched_count"], 0);
        assert_eq!(detail["terminal_outcome_state"], "missing");
    }
}
