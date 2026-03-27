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

    let current_session_id = normalize_session_scope(&session)?;
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
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
    let runtime = mvp::conversation::DefaultConversationRuntime::from_config_or_env(config)?;
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
        timeout_seconds,
        binding,
    )
    .await?;
    let task_id = required_string_field(&queued.payload, "child_session_id", "tasks create")?;
    let task_detail = build_task_detail(memory_config, tool_config, current_session_id, &task_id)?;
    let recipes = build_task_recipes(resolved_config_path, current_session_id, &task_id);
    let next_steps = build_task_next_steps();
    let payload = json!({
        "command": "create",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "queued_outcome": queued.payload,
        "task": task_detail,
        "recipes": recipes,
        "next_steps": next_steps,
    });
    Ok(payload)
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
    let scan_limit = 200usize;
    let sessions_payload = json!({
        "limit": scan_limit,
        "state": state,
        "kind": "delegate_child",
        "overdue_only": overdue_only,
        "include_archived": include_archived,
        "include_delegate_lifecycle": true,
    });
    let sessions_outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "sessions_list",
        sessions_payload,
    )?;
    let session_ids = extract_async_background_task_ids(&sessions_outcome.payload)?;
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
    let task = build_task_detail(memory_config, tool_config, current_session_id, task_id)?;
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
    let task = build_task_detail(memory_config, tool_config, current_session_id, task_id)?;
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

fn extract_async_background_task_ids(payload: &Value) -> CliResult<Vec<String>> {
    let sessions = payload
        .get("sessions")
        .and_then(Value::as_array)
        .ok_or_else(|| "tasks list payload missing sessions array".to_owned())?;
    let mut task_ids = Vec::new();
    for session in sessions {
        let session_id = required_string_field(session, "session_id", "tasks list session entry")?;
        let delegate_lifecycle = session
            .get("delegate_lifecycle")
            .cloned()
            .unwrap_or(Value::Null);
        let mode = delegate_lifecycle
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("");
        if mode != "async" {
            continue;
        }
        task_ids.push(session_id);
    }
    Ok(task_ids)
}

fn build_task_detail(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    task_id: &str,
) -> CliResult<Value> {
    let status_payload =
        load_task_status_payload(memory_config, tool_config, current_session_id, task_id)?;
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
    let session_kind = session.get("kind").and_then(Value::as_str).unwrap_or("");
    let delegate_mode = delegate.get("mode").and_then(Value::as_str).unwrap_or("");
    if session_kind != "delegate_child" || delegate_mode != "async" {
        return Err(format!(
            "tasks_cli_not_background_task: session `{task_id}` is not an async delegate child"
        ));
    }

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
    let command_name = crate::CLI_COMMAND_NAME;
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
