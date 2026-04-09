use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use clap::Subcommand;
use kernel::ToolCoreRequest;
use loongclaw_app as mvp;
use loongclaw_contracts::ToolCoreOutcome;
use loongclaw_spec::CliResult;
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
struct DetachedTasksRuntime {
    inner: mvp::conversation::DefaultConversationRuntime<
        Box<dyn mvp::conversation::ConversationContextEngine>,
    >,
    background_task_spawner: Arc<dyn mvp::conversation::AsyncDelegateSpawner>,
}

#[cfg(feature = "memory-sqlite")]
impl DetachedTasksRuntime {
    fn from_config(config: &mvp::config::LoongClawConfig) -> CliResult<Self> {
        let inner = mvp::conversation::DefaultConversationRuntime::from_config_or_env(config)?;
        let background_task_spawner = Arc::new(DetachedTasksSpawner);

        Ok(Self {
            inner,
            background_task_spawner,
        })
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl mvp::conversation::ConversationRuntime for DetachedTasksRuntime {
    fn session_context(
        &self,
        config: &mvp::config::LoongClawConfig,
        session_id: &str,
        binding: mvp::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<mvp::conversation::SessionContext> {
        self.inner.session_context(config, session_id, binding)
    }

    fn tool_view(
        &self,
        config: &mvp::config::LoongClawConfig,
        session_id: &str,
        binding: mvp::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<mvp::tools::ToolView> {
        self.inner.tool_view(config, session_id, binding)
    }

    fn background_task_spawner(
        &self,
        _config: &mvp::config::LoongClawConfig,
    ) -> Option<Arc<dyn mvp::conversation::AsyncDelegateSpawner>> {
        Some(self.background_task_spawner.clone())
    }

    async fn build_messages(
        &self,
        config: &mvp::config::LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &mvp::tools::ToolView,
        binding: mvp::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        self.inner
            .build_messages(
                config,
                session_id,
                include_system_prompt,
                tool_view,
                binding,
            )
            .await
    }

    async fn request_completion(
        &self,
        config: &mvp::config::LoongClawConfig,
        messages: &[Value],
        binding: mvp::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.inner
            .request_completion(config, messages, binding)
            .await
    }

    async fn request_turn(
        &self,
        config: &mvp::config::LoongClawConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &mvp::tools::ToolView,
        binding: mvp::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<mvp::conversation::ProviderTurn> {
        self.inner
            .request_turn(config, session_id, turn_id, messages, tool_view, binding)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &mvp::config::LoongClawConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &mvp::tools::ToolView,
        binding: mvp::conversation::ConversationRuntimeBinding<'_>,
        on_token: mvp::provider::StreamingTokenCallback,
    ) -> CliResult<mvp::conversation::ProviderTurn> {
        self.inner
            .request_turn_streaming(
                config, session_id, turn_id, messages, tool_view, binding, on_token,
            )
            .await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        binding: mvp::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        self.inner
            .persist_turn(session_id, role, content, binding)
            .await
    }
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
        } => execute_list_command(
            &resolved_path.display().to_string(),
            &current_session_id,
            &memory_config,
            tool_config,
            limit,
            state.as_deref(),
            overdue_only,
            include_archived,
        )?,
        TasksCommands::Status { task_id } => execute_status_command(
            &resolved_path.display().to_string(),
            &current_session_id,
            &memory_config,
            tool_config,
            &task_id,
        )?,
        TasksCommands::Events {
            task_id,
            after_id,
            limit,
        } => execute_events_command(
            &resolved_path.display().to_string(),
            &current_session_id,
            &memory_config,
            tool_config,
            &task_id,
            after_id,
            limit,
        )?,
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
        TasksCommands::Cancel { task_id, dry_run } => execute_cancel_command(
            &resolved_path.display().to_string(),
            &current_session_id,
            &memory_config,
            tool_config,
            &task_id,
            dry_run,
        )?,
        TasksCommands::Recover { task_id, dry_run } => execute_recover_command(
            &resolved_path.display().to_string(),
            &current_session_id,
            &memory_config,
            tool_config,
            &task_id,
            dry_run,
        )?,
    };

    Ok(TasksCommandExecution {
        resolved_config_path: resolved_path.display().to_string(),
        current_session_id,
        payload,
    })
}

async fn execute_create_command(
    resolved_config_path: &str,
    config: &mvp::config::LoongClawConfig,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task: &str,
    label: Option<String>,
    timeout_seconds: Option<u64>,
) -> CliResult<Value> {
    let runtime = build_tasks_create_runtime(config)?;
    let kernel_context = mvp::context::bootstrap_kernel_context_with_config(
        "cli-tasks",
        mvp::context::DEFAULT_TOKEN_TTL_S,
        config,
    )?;
    let binding = mvp::conversation::ConversationRuntimeBinding::kernel(&kernel_context);
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
        build_best_effort_task_detail(memory_config, tool_config, current_session_id, &task_id);
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

fn build_tasks_create_runtime(
    config: &mvp::config::LoongClawConfig,
) -> CliResult<impl mvp::conversation::ConversationRuntime> {
    #[cfg(feature = "memory-sqlite")]
    {
        let runtime = DetachedTasksRuntime::from_config(config)?;
        Ok(runtime)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let runtime = mvp::conversation::DefaultConversationRuntime::from_config_or_env(config)?;
        Ok(runtime)
    }
}

fn execute_list_command(
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
        let task = build_task_detail(memory_config, tool_config, current_session_id, &session_id)?;
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

fn execute_status_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task_id: &str,
) -> CliResult<Value> {
    let task = build_task_detail(memory_config, tool_config, current_session_id, task_id)?;
    let payload = json!({
        "command": "status",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "task": task,
    });
    Ok(payload)
}

fn execute_events_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    task_id: &str,
    after_id: Option<i64>,
    limit: usize,
) -> CliResult<Value> {
    let _ = build_task_detail(memory_config, tool_config, current_session_id, task_id)?;
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
    let _ = build_task_detail(memory_config, tool_config, current_session_id, task_id)?;
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
    let task = build_task_detail(memory_config, tool_config, current_session_id, task_id)?;
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

fn execute_cancel_command(
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
        build_best_effort_task_detail(memory_config, tool_config, current_session_id, task_id);
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

fn execute_recover_command(
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
        build_best_effort_task_detail(memory_config, tool_config, current_session_id, task_id);
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

fn build_task_detail(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> CliResult<Value> {
    let status_payload =
        load_task_status_payload(memory_config, tool_config, current_session_id, task_id)?;
    ensure_background_task_status_payload(&status_payload, task_id)?;
    let approvals_payload =
        load_task_approvals_payload(memory_config, tool_config, current_session_id, task_id)?;
    let tool_policy_payload =
        load_task_tool_policy_payload(memory_config, tool_config, current_session_id, task_id)?;

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
    let timeout_seconds = delegate
        .get("timeout_seconds")
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

    let detail = json!({
        "task_id": task_id,
        "session_id": task_id,
        "scope_session_id": current_session_id,
        "label": label,
        "session_state": session_state,
        "phase": phase,
        "mode": mode,
        "timeout_seconds": timeout_seconds,
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
        "tool_policy": tool_policy,
        "session": session,
        "delegate": delegate,
        "terminal_outcome_state": terminal_outcome_state,
        "terminal_outcome_missing_reason": terminal_outcome_missing_reason,
        "recovery": recovery,
        "terminal_outcome": terminal_outcome,
        "recent_events": recent_events,
    });
    Ok(detail)
}

fn build_best_effort_task_detail(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> (Value, Value) {
    let detail_result = build_task_detail(memory_config, tool_config, current_session_id, task_id);
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
    json!({
        "task_id": task_id,
        "session_id": task_id,
        "scope_session_id": current_session_id,
        "label": Value::Null,
        "session_state": Value::Null,
        "phase": Value::Null,
        "timeout_seconds": Value::Null,
        "last_error": Value::Null,
        "approval": {
            "matched_count": 0,
            "returned_count": 0,
            "attention_summary": Value::Null,
            "requests": [],
        },
        "tool_policy": Value::Null,
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

    let mut lines = Vec::new();
    lines.push(format!(
        "background task queued in session `{}`",
        payload
            .get("current_session_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    ));
    lines.extend(render_task_detail_lines(task)?);
    append_task_lookup_error_line(payload, &mut lines);

    if !recipes.is_empty() {
        lines.push("recipes:".to_owned());
        for recipe in recipes {
            let text = recipe.as_str().unwrap_or("");
            lines.push(format!("- {text}"));
        }
    }

    if !next_steps.is_empty() {
        lines.push("next steps:".to_owned());
        for step in next_steps {
            let text = step.as_str().unwrap_or("");
            lines.push(format!("- {text}"));
        }
    }

    Ok(lines.join("\n"))
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
    lines.push(format!(
        "visible background tasks from session `{scope}`: {returned_count}/{matched_count}"
    ));
    if tasks.is_empty() {
        lines.push("No async background tasks are currently visible.".to_owned());
        return Ok(lines.join("\n"));
    }

    for task in tasks {
        let line = render_task_brief_line(task)?;
        lines.push(format!("- {line}"));
    }

    Ok(lines.join("\n"))
}

fn render_tasks_status_text(payload: &Value) -> CliResult<String> {
    let task = payload
        .get("task")
        .ok_or_else(|| "tasks status payload missing task".to_owned())?;
    let lines = render_task_detail_lines(task)?;
    Ok(lines.join("\n"))
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
    lines.push(format!(
        "events for `{task_id}` (next_after_id={next_after_id})"
    ));
    if events.is_empty() {
        lines.push("No newer events.".to_owned());
        return Ok(lines.join("\n"));
    }

    for event in events {
        let event_id = event.get("id").and_then(Value::as_i64).unwrap_or_default();
        let event_kind = event
            .get("event_kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let ts = event.get("ts").and_then(Value::as_i64).unwrap_or_default();
        lines.push(format!("- #{event_id} {event_kind} ts={ts}"));
    }

    Ok(lines.join("\n"))
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
            lines.push(format!("- #{event_id} {event_kind}"));
        }
    }

    Ok(lines.join("\n"))
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
        lines.push(format!("result: {result}"));
    }
    if let Some(message) = message {
        lines.push(format!("message: {message}"));
    }
    if !action.is_null() {
        let rendered_action = serde_json::to_string_pretty(&action)
            .map_err(|error| format!("render action failed: {error}"))?;
        lines.push("action:".to_owned());
        lines.push(rendered_action);
    }
    lines.extend(render_task_detail_lines(task)?);
    append_task_lookup_error_line(payload, &mut lines);
    Ok(lines.join("\n"))
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
    let label = task.get("label").and_then(Value::as_str).unwrap_or("-");
    let approval_attention = task
        .get("approval")
        .and_then(|value| value.get("attention_summary"))
        .and_then(|value| value.get("needs_attention_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let line = format!(
        "{task_id} state={state} phase={phase} label={label} approval_attention={approval_attention}"
    );
    Ok(line)
}

fn render_task_detail_lines(task: &Value) -> CliResult<Vec<String>> {
    let task_id = required_string_field(task, "task_id", "task detail")?;
    let scope_session_id = task
        .get("scope_session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let label = task.get("label").and_then(Value::as_str).unwrap_or("-");
    let state = task
        .get("session_state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let phase = task
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
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
    let effective_tool_ids = task
        .get("tool_policy")
        .and_then(|value| value.get("effective_tool_ids"))
        .and_then(Value::as_array)
        .map(|values| render_string_array(values))
        .unwrap_or_else(|| "-".to_owned());
    let effective_runtime_narrowing = task
        .get("tool_policy")
        .and_then(|value| value.get("effective_runtime_narrowing"))
        .cloned()
        .unwrap_or(Value::Null);
    let rendered_runtime_narrowing = if effective_runtime_narrowing.is_null() {
        "-".to_owned()
    } else {
        serde_json::to_string(&effective_runtime_narrowing)
            .map_err(|error| format!("render runtime narrowing failed: {error}"))?
    };

    let mut lines = Vec::new();
    lines.push(format!("task_id: {task_id}"));
    lines.push(format!("scope_session_id: {scope_session_id}"));
    lines.push(format!("label: {label}"));
    lines.push(format!("state: {state}"));
    lines.push(format!("phase: {phase}"));
    lines.push(format!("timeout_seconds: {timeout_seconds}"));
    lines.push(format!("last_error: {last_error}"));
    lines.push(format!("approval_requests: {approval_total}"));
    lines.push(format!("approval_attention: {approval_attention}"));
    lines.push(format!("effective_tool_ids: {effective_tool_ids}"));
    lines.push(format!(
        "effective_runtime_narrowing: {rendered_runtime_narrowing}"
    ));
    Ok(lines)
}

fn append_task_lookup_error_line(payload: &Value, lines: &mut Vec<String>) {
    let Some(task_lookup_error) = payload.get("task_lookup_error").and_then(Value::as_str) else {
        return;
    };
    lines.push(format!("task_lookup_error: {task_lookup_error}"));
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
