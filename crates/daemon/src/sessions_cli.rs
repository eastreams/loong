use clap::Subcommand;
use kernel::ToolCoreRequest;
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde_json::{Value, json};

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SessionsCommands {
    /// List visible persisted sessions for the scoped session lineage
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        parent_session_id: Option<String>,
        #[arg(long, default_value_t = false)]
        overdue_only: bool,
        #[arg(long, default_value_t = false)]
        include_archived: bool,
        #[arg(long, default_value_t = false)]
        include_delegate_lifecycle: bool,
    },
    /// Inspect one visible persisted session
    #[command(visible_alias = "info")]
    Status { session_id: String },
    /// Show recent lifecycle events for one visible session
    Events {
        session_id: String,
        #[arg(long)]
        after_id: Option<i64>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Wait on one visible session and return incremental events
    Wait {
        session_id: String,
        #[arg(long)]
        after_id: Option<i64>,
        #[arg(long, default_value_t = 1_000)]
        timeout_ms: u64,
    },
    /// Show recent transcript turns for one visible session
    History {
        session_id: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Cancel one visible session
    Cancel {
        session_id: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Recover one visible session
    Recover {
        session_id: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Archive one visible terminal session
    Archive {
        session_id: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone)]
pub struct SessionsCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub session: String,
    pub command: SessionsCommands,
}

#[derive(Debug, Clone)]
pub struct SessionsCommandExecution {
    pub resolved_config_path: String,
    pub current_session_id: String,
    pub payload: Value,
}

pub async fn run_sessions_cli(options: SessionsCommandOptions) -> CliResult<()> {
    let as_json = options.json;
    let execution = execute_sessions_command(options).await?;
    if as_json {
        let pretty = serde_json::to_string_pretty(&execution.payload)
            .map_err(|error| format!("serialize sessions CLI output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = render_sessions_cli_text(&execution)?;
    println!("{rendered}");
    Ok(())
}

pub async fn execute_sessions_command(
    options: SessionsCommandOptions,
) -> CliResult<SessionsCommandExecution> {
    let SessionsCommandOptions {
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
    let resolved_config_path = resolved_path.display().to_string();

    let payload = match command {
        SessionsCommands::List {
            limit,
            state,
            kind,
            parent_session_id,
            overdue_only,
            include_archived,
            include_delegate_lifecycle,
        } => execute_list_command(
            &resolved_config_path,
            &current_session_id,
            &memory_config,
            tool_config,
            limit,
            state.as_deref(),
            kind.as_deref(),
            parent_session_id.as_deref(),
            overdue_only,
            include_archived,
            include_delegate_lifecycle,
        )?,
        SessionsCommands::Status { session_id } => execute_status_command(
            &resolved_config_path,
            &current_session_id,
            &memory_config,
            tool_config,
            &session_id,
        )?,
        SessionsCommands::Events {
            session_id,
            after_id,
            limit,
        } => execute_events_command(
            &resolved_config_path,
            &current_session_id,
            &memory_config,
            tool_config,
            &session_id,
            after_id,
            limit,
        )?,
        SessionsCommands::Wait {
            session_id,
            after_id,
            timeout_ms,
        } => {
            execute_wait_command(
                &resolved_config_path,
                &current_session_id,
                &memory_config,
                tool_config,
                &session_id,
                after_id,
                timeout_ms,
            )
            .await?
        }
        SessionsCommands::History { session_id, limit } => execute_history_command(
            &resolved_config_path,
            &current_session_id,
            &memory_config,
            tool_config,
            &session_id,
            limit,
        )?,
        SessionsCommands::Cancel {
            session_id,
            dry_run,
        } => execute_mutation_command(
            "cancel",
            "session_cancel",
            "cancel_action",
            &resolved_config_path,
            &current_session_id,
            &memory_config,
            tool_config,
            &session_id,
            dry_run,
        )?,
        SessionsCommands::Recover {
            session_id,
            dry_run,
        } => execute_mutation_command(
            "recover",
            "session_recover",
            "recovery_action",
            &resolved_config_path,
            &current_session_id,
            &memory_config,
            tool_config,
            &session_id,
            dry_run,
        )?,
        SessionsCommands::Archive {
            session_id,
            dry_run,
        } => execute_mutation_command(
            "archive",
            "session_archive",
            "archive_action",
            &resolved_config_path,
            &current_session_id,
            &memory_config,
            tool_config,
            &session_id,
            dry_run,
        )?,
    };

    Ok(SessionsCommandExecution {
        resolved_config_path,
        current_session_id,
        payload,
    })
}

fn execute_list_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    limit: usize,
    state: Option<&str>,
    kind: Option<&str>,
    parent_session_id: Option<&str>,
    overdue_only: bool,
    include_archived: bool,
    include_delegate_lifecycle: bool,
) -> CliResult<Value> {
    let raw_limit = limit.clamp(1, 200);
    let payload = json!({
        "limit": raw_limit,
        "state": state,
        "kind": kind,
        "parent_session_id": parent_session_id,
        "overdue_only": overdue_only,
        "include_archived": include_archived,
        "include_delegate_lifecycle": include_delegate_lifecycle,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "sessions_list",
        payload,
    )?;
    let filters = outcome
        .payload
        .get("filters")
        .cloned()
        .unwrap_or(Value::Null);
    let matched_count = outcome
        .payload
        .get("matched_count")
        .cloned()
        .unwrap_or(Value::Null);
    let returned_count = outcome
        .payload
        .get("returned_count")
        .cloned()
        .unwrap_or(Value::Null);
    let sessions = outcome
        .payload
        .get("sessions")
        .cloned()
        .unwrap_or_else(|| json!([]));

    Ok(json!({
        "command": "list",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "filters": filters,
        "matched_count": matched_count,
        "returned_count": returned_count,
        "sessions": sessions,
    }))
}

fn execute_status_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
) -> CliResult<Value> {
    let detail =
        load_session_status_payload(memory_config, tool_config, current_session_id, session_id)?;
    let recipes = build_session_recipes(resolved_config_path, current_session_id, session_id);
    let next_steps = build_session_next_steps();

    Ok(json!({
        "command": "status",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "session_id": session_id,
        "detail": detail,
        "recipes": recipes,
        "next_steps": next_steps,
    }))
}

fn execute_events_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
    after_id: Option<i64>,
    limit: usize,
) -> CliResult<Value> {
    let _ =
        load_session_status_payload(memory_config, tool_config, current_session_id, session_id)?;
    let event_limit = limit.clamp(1, 200);
    let payload = json!({
        "session_id": session_id,
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

    Ok(json!({
        "command": "events",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "session_id": session_id,
        "after_id": after_id,
        "next_after_id": next_after_id,
        "events": events,
    }))
}

async fn execute_wait_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
) -> CliResult<Value> {
    let _ =
        load_session_status_payload(memory_config, tool_config, current_session_id, session_id)?;
    let bounded_timeout_ms = timeout_ms.clamp(1, 30_000);
    let payload = json!({
        "session_id": session_id,
        "after_id": after_id,
        "timeout_ms": bounded_timeout_ms,
    });
    let outcome = mvp::tools::wait_for_session_with_config(
        payload,
        current_session_id,
        memory_config,
        tool_config,
    )
    .await?;

    Ok(json!({
        "command": "wait",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "session_id": session_id,
        "wait_status": outcome.status,
        "detail": outcome.payload,
    }))
}

fn execute_history_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
    limit: usize,
) -> CliResult<Value> {
    let _ =
        load_session_status_payload(memory_config, tool_config, current_session_id, session_id)?;
    let history_limit = limit.clamp(1, 200);
    let payload = json!({
        "session_id": session_id,
        "limit": history_limit,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        "sessions_history",
        payload,
    )?;
    let turns = outcome
        .payload
        .get("turns")
        .cloned()
        .unwrap_or_else(|| json!([]));

    Ok(json!({
        "command": "history",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "session_id": session_id,
        "limit": history_limit,
        "turns": turns,
    }))
}

fn execute_mutation_command(
    command_name: &str,
    tool_name: &str,
    action_field: &str,
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
    dry_run: bool,
) -> CliResult<Value> {
    let payload = json!({
        "session_ids": [session_id],
        "dry_run": dry_run,
    });
    let outcome = execute_app_tool_request(
        memory_config,
        tool_config,
        current_session_id,
        tool_name,
        payload,
    )?;
    let result = extract_single_mutation_result(&outcome.payload)
        .ok_or_else(|| format!("{command_name} payload missing single result"))?;
    let message = result.get("message").cloned().unwrap_or(Value::Null);
    let action = result.get("action").cloned().unwrap_or_else(|| {
        outcome
            .payload
            .get(action_field)
            .cloned()
            .unwrap_or(Value::Null)
    });
    let inspection = result.get("inspection").cloned().unwrap_or(Value::Null);
    let mutation_result = result.get("result").cloned().unwrap_or(Value::Null);

    Ok(json!({
        "command": command_name,
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "session_id": session_id,
        "dry_run": dry_run,
        "result": mutation_result,
        "message": message,
        "action": action,
        "inspection": inspection,
    }))
}

fn execute_app_tool_request(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    tool_name: &str,
    payload: Value,
) -> CliResult<kernel::ToolCoreOutcome> {
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

fn load_session_status_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    session_id: &str,
) -> CliResult<Value> {
    let payload = json!({
        "session_id": session_id,
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

fn extract_single_mutation_result(payload: &Value) -> Option<Value> {
    let results = payload.get("results")?.as_array()?;
    if results.len() != 1 {
        return None;
    }
    results.first().cloned()
}

fn build_session_recipes(
    resolved_config_path: &str,
    current_session_id: &str,
    session_id: &str,
) -> Vec<String> {
    let command_name = crate::active_cli_command_name();
    let config_arg = crate::cli_handoff::shell_quote_argument(resolved_config_path);
    let session_arg = crate::cli_handoff::shell_quote_argument(current_session_id);
    let target_arg = crate::cli_handoff::shell_quote_argument(session_id);

    let status_recipe = format!(
        "{command_name} sessions status --config {config_arg} --session {session_arg} {target_arg}"
    );
    let history_recipe = format!(
        "{command_name} sessions history --config {config_arg} --session {session_arg} {target_arg}"
    );
    let wait_recipe = format!(
        "{command_name} sessions wait --config {config_arg} --session {session_arg} {target_arg}"
    );
    let events_recipe = format!(
        "{command_name} sessions events --config {config_arg} --session {session_arg} {target_arg}"
    );

    vec![status_recipe, history_recipe, wait_recipe, events_recipe]
}

fn build_session_next_steps() -> Vec<String> {
    let step_one =
        "Use `sessions history` to inspect transcript continuity for the selected session."
            .to_owned();
    let step_two =
        "Use `sessions wait` or `sessions events` when you need bounded progress checks."
            .to_owned();
    let step_three =
        "Use `sessions cancel`, `sessions recover`, or `sessions archive` only after status confirms the state transition is valid."
            .to_owned();
    vec![step_one, step_two, step_three]
}

fn normalize_session_scope(raw: &str) -> CliResult<String> {
    let session = raw.trim();
    if session.is_empty() {
        return Err("sessions CLI requires a non-empty session scope".to_owned());
    }
    Ok(session.to_owned())
}

fn required_string_field(value: &Value, field: &str, context: &str) -> CliResult<String> {
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context} missing string field `{field}`"))?;
    Ok(text.to_owned())
}

pub fn render_sessions_cli_text(execution: &SessionsCommandExecution) -> CliResult<String> {
    let command = execution
        .payload
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "sessions CLI payload missing command".to_owned())?;

    let rendered = match command {
        "list" => render_sessions_list_text(&execution.payload)?,
        "status" => render_sessions_status_text(&execution.payload)?,
        "events" => render_sessions_events_text(&execution.payload)?,
        "wait" => render_sessions_wait_text(&execution.payload)?,
        "history" => render_sessions_history_text(&execution.payload)?,
        "cancel" | "recover" | "archive" => render_sessions_mutation_text(&execution.payload)?,
        other => {
            return Err(format!("unknown sessions CLI render command `{other}`"));
        }
    };
    Ok(rendered)
}

fn sanitize_terminal_text(value: &str) -> String {
    let mut sanitized = String::new();
    for character in value.chars() {
        if character.is_control() {
            let escaped = character.escape_default().to_string();
            sanitized.push_str(escaped.as_str());
            continue;
        }
        sanitized.push(character);
    }
    sanitized
}

fn render_sessions_list_text(payload: &Value) -> CliResult<String> {
    let sessions = payload
        .get("sessions")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions list payload missing sessions array".to_owned())?;
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
    let sanitized_scope = sanitize_terminal_text(scope);

    let mut session_lines = Vec::new();
    session_lines.push(format!(
        "visible sessions from scope `{sanitized_scope}`: {returned_count}/{matched_count}"
    ));
    if sessions.is_empty() {
        session_lines.push("No persisted sessions are currently visible.".to_owned());
        return Ok(render_sessions_surface(
            "visible sessions",
            "session shell",
            Vec::new(),
            vec![("sessions", session_lines)],
            vec!["Use `sessions status <id>` to inspect one session in detail.".to_owned()],
        ));
    }

    for session in sessions {
        let line = render_session_brief_line(session)?;
        session_lines.push(format!("- {line}"));
    }

    Ok(render_sessions_surface(
        "visible sessions",
        "session shell",
        Vec::new(),
        vec![("sessions", session_lines)],
        vec![
            "Use `sessions status <id>` for a single session, or `sessions history <id>` for transcript turns."
                .to_owned(),
        ],
    ))
}

fn render_sessions_status_text(payload: &Value) -> CliResult<String> {
    let detail = payload
        .get("detail")
        .ok_or_else(|| "sessions status payload missing detail".to_owned())?;
    let recipes = payload
        .get("recipes")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions status payload missing recipes".to_owned())?;
    let next_steps = payload
        .get("next_steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions status payload missing next_steps".to_owned())?;

    let detail_lines = render_session_inspection_lines(detail)?;
    let mut sections = vec![("session detail", detail_lines)];
    let mut footer_lines = vec![
        "Use `sessions events`, `sessions wait`, and `sessions history` to keep drilling into the same session."
            .to_owned(),
    ];
    let mut recipes_lines = Vec::new();
    if !recipes.is_empty() {
        for recipe in recipes {
            let text = recipe.as_str().unwrap_or("");
            let sanitized_text = sanitize_terminal_text(text);
            recipes_lines.push(format!("- {sanitized_text}"));
        }
    }
    if !recipes_lines.is_empty() {
        sections.push(("recipes", recipes_lines));
    }
    let mut next_lines = Vec::new();
    if !next_steps.is_empty() {
        for step in next_steps {
            let text = step.as_str().unwrap_or("");
            let sanitized_text = sanitize_terminal_text(text);
            next_lines.push(format!("- {sanitized_text}"));
        }
    }
    if !next_lines.is_empty() {
        sections.insert(0, ("next steps", next_lines));
        footer_lines = vec!["Use the first next step as the operator handoff, then come back here if the session needs deeper inspection.".to_owned()];
    }

    Ok(render_sessions_surface(
        "session detail",
        "session shell",
        Vec::new(),
        sections,
        footer_lines,
    ))
}

fn render_sessions_events_text(payload: &Value) -> CliResult<String> {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let events = payload
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions events payload missing events array".to_owned())?;
    let next_after_id = payload
        .get("next_after_id")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let mut lines = Vec::new();
    let sanitized_session_id = sanitize_terminal_text(session_id);
    lines.push(format!(
        "events for `{sanitized_session_id}` (next_after_id={next_after_id})"
    ));
    if events.is_empty() {
        lines.push("No newer events.".to_owned());
        return Ok(render_sessions_surface(
            "session events",
            "session shell",
            Vec::new(),
            vec![("events", lines)],
            vec![
                "Use `sessions wait` to keep following the same session incrementally.".to_owned(),
            ],
        ));
    }

    for event in events {
        let event_id = event.get("id").and_then(Value::as_i64).unwrap_or_default();
        let event_kind = event
            .get("event_kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let ts = event.get("ts").and_then(Value::as_i64).unwrap_or_default();
        let sanitized_event_kind = sanitize_terminal_text(event_kind);
        lines.push(format!("- #{event_id} {sanitized_event_kind} ts={ts}"));
    }

    Ok(render_sessions_surface(
        "session events",
        "session shell",
        Vec::new(),
        vec![("events", lines)],
        vec!["Use `sessions wait` for incremental follow-up or `sessions status` for the latest session state.".to_owned()],
    ))
}

fn render_sessions_wait_text(payload: &Value) -> CliResult<String> {
    let wait_status = payload
        .get("wait_status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let detail = payload
        .get("detail")
        .ok_or_else(|| "sessions wait payload missing detail".to_owned())?;
    let events = detail
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions wait detail missing events array".to_owned())?;
    let next_after_id = detail
        .get("next_after_id")
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let mut lines = Vec::new();
    lines.push(format!(
        "wait result: {wait_status} (next_after_id={next_after_id})"
    ));
    lines.extend(render_session_inspection_lines(detail)?);
    if !events.is_empty() {
        lines.push("observed events:".to_owned());
        for event in events {
            let event_id = event.get("id").and_then(Value::as_i64).unwrap_or_default();
            let event_kind = event
                .get("event_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let sanitized_event_kind = sanitize_terminal_text(event_kind);
            lines.push(format!("- #{event_id} {sanitized_event_kind}"));
        }
    }

    Ok(render_sessions_surface(
        "session wait",
        "session shell",
        Vec::new(),
        vec![("result", lines)],
        vec![
            "Re-run `sessions wait` with the returned cursor when you need more lifecycle changes."
                .to_owned(),
        ],
    ))
}

fn render_sessions_history_text(payload: &Value) -> CliResult<String> {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let limit = payload.get("limit").and_then(Value::as_u64).unwrap_or(0);
    let turns = payload
        .get("turns")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions history payload missing turns array".to_owned())?;

    let mut lines = Vec::new();
    let sanitized_session_id = sanitize_terminal_text(session_id);
    lines.push(format!(
        "history for `{sanitized_session_id}` (limit={limit})"
    ));
    if turns.is_empty() {
        lines.push("No transcript turns are currently stored.".to_owned());
        return Ok(render_sessions_surface(
            "session history",
            "session shell",
            Vec::new(),
            vec![("history", lines)],
            vec!["Use `sessions status` to compare transcript turns with workflow state and lifecycle metadata.".to_owned()],
        ));
    }

    for turn in turns {
        let role = turn
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let content = turn.get("content").and_then(Value::as_str).unwrap_or("");
        let sanitized_role = sanitize_terminal_text(role);
        let sanitized_content = sanitize_terminal_text(content);
        lines.push(format!("- {sanitized_role}: {sanitized_content}"));
    }

    Ok(render_sessions_surface(
        "session history",
        "session shell",
        Vec::new(),
        vec![("history", lines)],
        vec!["Use `sessions status` to compare transcript turns with workflow state and lifecycle metadata.".to_owned()],
    ))
}

fn render_sessions_mutation_text(payload: &Value) -> CliResult<String> {
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let dry_run = payload
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result = payload
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let action = payload.get("action").cloned().unwrap_or(Value::Null);
    let inspection = payload.get("inspection").cloned().unwrap_or(Value::Null);

    let mut lines = Vec::new();
    let sanitized_command = sanitize_terminal_text(command);
    let sanitized_result = sanitize_terminal_text(result);
    let sanitized_message = sanitize_terminal_text(message);
    lines.push(format!("{sanitized_command} dry_run={dry_run}"));
    lines.push(format!("result: {sanitized_result}"));
    lines.push(format!("message: {sanitized_message}"));
    if !action.is_null() {
        let rendered_action = serde_json::to_string_pretty(&action)
            .map_err(|error| format!("render action failed: {error}"))?;
        lines.push("action:".to_owned());
        lines.push(rendered_action);
    }
    if !inspection.is_null() {
        lines.extend(render_session_inspection_lines(&inspection)?);
    }

    Ok(render_sessions_surface(
        "session action",
        "session shell",
        Vec::new(),
        vec![("action result", lines)],
        vec![
            "Use `sessions status <id>` to confirm the current session state after the mutation."
                .to_owned(),
        ],
    ))
}

fn render_sessions_surface(
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

fn render_session_brief_line(session: &Value) -> CliResult<String> {
    let session_id = required_string_field(session, "session_id", "session summary")?;
    let state = session
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let kind = session
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let label = session.get("label").and_then(Value::as_str).unwrap_or("-");
    let task = session
        .get("workflow")
        .and_then(|value| value.get("task"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_phase = session
        .get("workflow")
        .and_then(|value| value.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let lineage_depth = session
        .get("workflow")
        .and_then(|value| value.get("lineage_depth"))
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let line = format!(
        "{} state={state} kind={kind} workflow_phase={workflow_phase} label={} task={} depth={lineage_depth}",
        sanitize_terminal_text(session_id.as_str()),
        sanitize_terminal_text(label),
        sanitize_terminal_text(task),
    );
    Ok(line)
}

fn render_session_inspection_lines(detail: &Value) -> CliResult<Vec<String>> {
    let session = detail
        .get("session")
        .ok_or_else(|| "session inspection missing session".to_owned())?;
    let session_id = required_string_field(session, "session_id", "session inspection")?;
    let kind = session
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let state = session
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let parent_session_id = session
        .get("parent_session_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let label = session.get("label").and_then(Value::as_str).unwrap_or("-");
    let turn_count = session
        .get("turn_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_turn_at = session
        .get("last_turn_at")
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let last_error = session
        .get("last_error")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow = detail.get("workflow").cloned().unwrap_or(Value::Null);
    let task = workflow.get("task").and_then(Value::as_str).unwrap_or("-");
    let workflow_id = workflow
        .get("workflow_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_phase = workflow.get("phase").and_then(Value::as_str).unwrap_or("-");
    let workflow_operation_kind = workflow
        .get("operation_kind")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_operation_scope = workflow
        .get("operation_scope")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_task_session_id = workflow
        .get("task_session_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_binding_mode = workflow
        .get("binding")
        .and_then(|value| value.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_execution_surface = workflow
        .get("binding")
        .and_then(|value| value.get("execution_surface"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_worktree_id = workflow
        .get("binding")
        .and_then(|value| value.get("worktree"))
        .and_then(|value| value.get("worktree_id"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let workflow_workspace_root = workflow
        .get("binding")
        .and_then(|value| value.get("worktree"))
        .and_then(|value| value.get("workspace_root"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let lineage_root_session_id = workflow
        .get("lineage_root_session_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let lineage_depth = workflow
        .get("lineage_depth")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let continuity =
        render_runtime_self_continuity_summary(workflow.get("runtime_self_continuity"));
    let delegate_mode = detail
        .get("delegate_lifecycle")
        .and_then(|value| value.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let delegate_phase = detail
        .get("delegate_lifecycle")
        .and_then(|value| value.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let timeout_seconds = detail
        .get("delegate_lifecycle")
        .and_then(|value| value.get("timeout_seconds"))
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let terminal_outcome_state = detail
        .get("terminal_outcome_state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let terminal_status = detail
        .get("terminal_outcome")
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let recovery_kind = detail
        .get("recovery")
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("-");
    let recent_events = detail
        .get("recent_events")
        .and_then(Value::as_array)
        .map(|value| value.len())
        .unwrap_or(0);
    let sanitized_session_id = sanitize_terminal_text(session_id.as_str());
    let sanitized_parent_session_id = sanitize_terminal_text(parent_session_id);
    let sanitized_label = sanitize_terminal_text(label);
    let sanitized_task = sanitize_terminal_text(task);
    let sanitized_workflow_id = sanitize_terminal_text(workflow_id);
    let sanitized_workflow_task_session_id = sanitize_terminal_text(workflow_task_session_id);
    let sanitized_workflow_binding_mode = sanitize_terminal_text(workflow_binding_mode);
    let sanitized_workflow_execution_surface = sanitize_terminal_text(workflow_execution_surface);
    let sanitized_workflow_worktree_id = sanitize_terminal_text(workflow_worktree_id);
    let sanitized_workflow_workspace_root = sanitize_terminal_text(workflow_workspace_root);
    let sanitized_lineage_root_session_id = sanitize_terminal_text(lineage_root_session_id);
    let sanitized_last_error = sanitize_terminal_text(last_error);

    let mut lines = Vec::new();
    lines.push(format!("session_id: {sanitized_session_id}"));
    lines.push(format!("kind: {kind}"));
    lines.push(format!("state: {state}"));
    lines.push(format!("workflow_id: {sanitized_workflow_id}"));
    lines.push(format!("workflow_phase: {workflow_phase}"));
    lines.push(format!(
        "workflow_operation_kind: {workflow_operation_kind}"
    ));
    lines.push(format!(
        "workflow_operation_scope: {workflow_operation_scope}"
    ));
    lines.push(format!(
        "workflow_task_session_id: {sanitized_workflow_task_session_id}"
    ));
    lines.push(format!(
        "workflow_binding_mode: {sanitized_workflow_binding_mode}"
    ));
    lines.push(format!(
        "workflow_execution_surface: {sanitized_workflow_execution_surface}"
    ));
    lines.push(format!(
        "workflow_worktree_id: {sanitized_workflow_worktree_id}"
    ));
    lines.push(format!(
        "workflow_workspace_root: {sanitized_workflow_workspace_root}"
    ));
    lines.push(format!("parent_session_id: {sanitized_parent_session_id}"));
    lines.push(format!("label: {sanitized_label}"));
    lines.push(format!("task: {sanitized_task}"));
    lines.push(format!(
        "lineage_root_session_id: {sanitized_lineage_root_session_id}"
    ));
    lines.push(format!("lineage_depth: {lineage_depth}"));
    lines.push(format!("runtime_self_continuity: {continuity}"));
    lines.push(format!("turn_count: {turn_count}"));
    lines.push(format!("last_turn_at: {last_turn_at}"));
    lines.push(format!("last_error: {sanitized_last_error}"));
    lines.push(format!("delegate_mode: {delegate_mode}"));
    lines.push(format!("delegate_phase: {delegate_phase}"));
    lines.push(format!("timeout_seconds: {timeout_seconds}"));
    lines.push(format!("terminal_outcome_state: {terminal_outcome_state}"));
    lines.push(format!("terminal_status: {terminal_status}"));
    lines.push(format!("recovery_kind: {recovery_kind}"));
    lines.push(format!("recent_events: {recent_events}"));
    Ok(lines)
}

fn render_runtime_self_continuity_summary(runtime_self_continuity: Option<&Value>) -> String {
    let Some(runtime_self_continuity) = runtime_self_continuity else {
        return "-".to_owned();
    };
    let present = runtime_self_continuity
        .get("present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !present {
        return "absent".to_owned();
    }

    let resolved_identity_present = runtime_self_continuity
        .get("resolved_identity_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let session_profile_projection_present = runtime_self_continuity
        .get("session_profile_projection_present")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    format!(
        "present resolved_identity={} session_profile_projection={}",
        resolved_identity_present, session_profile_projection_present
    )
}
