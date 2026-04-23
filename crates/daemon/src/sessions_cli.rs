use clap::Subcommand;
use kernel::ToolCoreRequest;
use loong_app as mvp;
use loong_spec::CliResult;
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
    /// Plan or apply bounded self-heal actions for one visible session
    Heal {
        session_id: String,
        #[arg(long, default_value_t = false)]
        apply: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionHealApplyStrategy {
    SessionRecover,
    TurnCheckpointRepair,
    ObserveOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionHealAction {
    id: String,
    kind: String,
    source: String,
    tool_name: String,
    description: String,
    command: String,
    can_apply: bool,
    requires_mutation: bool,
    apply_strategy: SessionHealApplyStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionHealPlan {
    actions: Vec<SessionHealAction>,
    attention_hints: Vec<String>,
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
        SessionsCommands::Status { session_id } => {
            execute_status_command(
                &resolved_config_path,
                &current_session_id,
                &memory_config,
                tool_config,
                &session_id,
            )
            .await?
        }
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
        SessionsCommands::Heal { session_id, apply } => {
            execute_heal_command(
                &resolved_config_path,
                &config,
                &current_session_id,
                &memory_config,
                tool_config,
                &session_id,
                apply,
            )
            .await?
        }
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

async fn execute_status_command(
    resolved_config_path: &str,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
) -> CliResult<Value> {
    let detail = load_session_status_payload_with_runtime_summaries(
        memory_config,
        tool_config,
        current_session_id,
        session_id,
    )
    .await?;
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

async fn load_session_status_payload_with_runtime_summaries(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    current_session_id: &str,
    session_id: &str,
) -> CliResult<Value> {
    let mut detail =
        load_session_status_payload(memory_config, tool_config, current_session_id, session_id)?;
    let prompt_frame = crate::session_prompt_frame_cli::load_session_prompt_frame_payload(
        memory_config,
        session_id,
    )
    .await;
    let safe_lane =
        crate::session_runtime_truth_cli::load_session_safe_lane_payload(memory_config, session_id)
            .await;
    let turn_checkpoint = crate::session_runtime_truth_cli::load_session_turn_checkpoint_payload(
        memory_config,
        session_id,
    )
    .await;

    let detail_object = detail
        .as_object_mut()
        .ok_or_else(|| "session status payload must be an object".to_owned())?;
    detail_object.insert("prompt_frame".to_owned(), prompt_frame);
    detail_object.insert("safe_lane".to_owned(), safe_lane);
    detail_object.insert("turn_checkpoint".to_owned(), turn_checkpoint);

    Ok(detail)
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

async fn execute_heal_command(
    resolved_config_path: &str,
    config: &mvp::config::LoongConfig,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
    apply: bool,
) -> CliResult<Value> {
    let detail_before = load_session_status_payload_with_runtime_summaries(
        memory_config,
        tool_config,
        current_session_id,
        session_id,
    )
    .await?;
    let plan = build_session_heal_plan(
        resolved_config_path,
        current_session_id,
        session_id,
        &detail_before,
    )?;
    let applied_actions = if apply {
        execute_session_heal_plan(
            resolved_config_path,
            config,
            current_session_id,
            memory_config,
            tool_config,
            session_id,
            &plan,
        )
        .await?
    } else {
        Vec::new()
    };
    let detail_after = if apply {
        load_session_status_payload_with_runtime_summaries(
            memory_config,
            tool_config,
            current_session_id,
            session_id,
        )
        .await?
    } else {
        detail_before.clone()
    };
    let recipes =
        build_session_heal_recipes(resolved_config_path, current_session_id, session_id, &plan);
    let next_steps = build_session_heal_next_steps(
        resolved_config_path,
        current_session_id,
        session_id,
        &plan,
        apply,
        applied_actions.as_slice(),
    );
    let plan_payload = session_heal_plan_json(&plan);

    Ok(json!({
        "command": "heal",
        "config": resolved_config_path,
        "current_session_id": current_session_id,
        "session_id": session_id,
        "apply": apply,
        "detail": detail_after,
        "plan": plan_payload,
        "applied_actions": applied_actions,
        "recipes": recipes,
        "next_steps": next_steps,
    }))
}

async fn execute_session_heal_plan(
    resolved_config_path: &str,
    config: &mvp::config::LoongConfig,
    current_session_id: &str,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &mvp::config::ToolConfig,
    session_id: &str,
    plan: &SessionHealPlan,
) -> CliResult<Vec<Value>> {
    let mut applied_actions = Vec::new();

    for action in &plan.actions {
        if !action.can_apply {
            continue;
        }

        let applied_action = match action.apply_strategy {
            SessionHealApplyStrategy::SessionRecover => {
                let payload = execute_mutation_command(
                    "recover",
                    "session_recover",
                    "recovery_action",
                    resolved_config_path,
                    current_session_id,
                    memory_config,
                    tool_config,
                    session_id,
                    false,
                )?;
                json!({
                    "id": action.id,
                    "kind": action.kind,
                    "tool_name": action.tool_name,
                    "status": "applied",
                    "result": payload,
                })
            }
            SessionHealApplyStrategy::TurnCheckpointRepair => {
                let payload = execute_turn_checkpoint_heal_action(config, session_id).await?;
                let status = payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                json!({
                    "id": action.id,
                    "kind": action.kind,
                    "tool_name": action.tool_name,
                    "status": status,
                    "result": payload,
                })
            }
            SessionHealApplyStrategy::ObserveOnly => {
                continue;
            }
        };

        applied_actions.push(applied_action);
    }

    Ok(applied_actions)
}

async fn execute_turn_checkpoint_heal_action(
    config: &mvp::config::LoongConfig,
    session_id: &str,
) -> CliResult<Value> {
    let runtime_kernel = bootstrap_sessions_runtime_kernel(config)?;
    let binding = runtime_kernel.conversation_binding();
    let coordinator = mvp::conversation::ConversationTurnCoordinator::new();
    let outcome = coordinator
        .repair_production_turn_checkpoint_tail(config, session_id, binding)
        .await?;
    let source = outcome.source().map(|value| value.as_str()).unwrap_or("-");
    let after_turn_status = outcome.after_turn_status().unwrap_or("-");
    let compaction_status = outcome.compaction_status().unwrap_or("-");

    Ok(json!({
        "status": outcome.status().as_str(),
        "action": outcome.action().as_str(),
        "source": source,
        "reason": outcome.reason().as_str(),
        "session_state": outcome.session_state().as_str(),
        "checkpoint_events": outcome.checkpoint_events(),
        "after_turn_status": after_turn_status,
        "compaction_status": compaction_status,
    }))
}

fn bootstrap_sessions_runtime_kernel(
    config: &mvp::config::LoongConfig,
) -> CliResult<mvp::runtime_bridge::RuntimeKernelOwner> {
    let agent_id = "cli-sessions-heal";
    let runtime_kernel = mvp::runtime_bridge::RuntimeKernelOwner::bootstrap(agent_id, config)?;
    Ok(runtime_kernel)
}

fn build_session_heal_plan(
    resolved_config_path: &str,
    current_session_id: &str,
    session_id: &str,
    detail: &Value,
) -> CliResult<SessionHealPlan> {
    let diagnostics = detail.get("diagnostics").cloned().unwrap_or(Value::Null);
    let attention_hints = diagnostics
        .get("attention_hints")
        .and_then(Value::as_array)
        .map(|hints| {
            hints
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut actions = Vec::new();

    if let Some(recommended_action) = diagnostics.get("recommended_action") {
        let mapped_action = map_recommended_action_to_session_heal_action(
            resolved_config_path,
            current_session_id,
            session_id,
            recommended_action,
        )?;
        if let Some(mapped_action) = mapped_action {
            actions.push(mapped_action);
        }
    }

    let checkpoint_action = maybe_build_turn_checkpoint_heal_action(
        resolved_config_path,
        current_session_id,
        session_id,
        detail,
    );
    if let Some(checkpoint_action) = checkpoint_action {
        actions.push(checkpoint_action);
    }

    Ok(SessionHealPlan {
        actions,
        attention_hints,
    })
}

fn map_recommended_action_to_session_heal_action(
    resolved_config_path: &str,
    current_session_id: &str,
    session_id: &str,
    action: &Value,
) -> CliResult<Option<SessionHealAction>> {
    let tool_name = action
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if tool_name.is_empty() {
        return Ok(None);
    }

    let kind = action
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let source = action
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let note = action
        .get("note")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let requires_mutation = action
        .get("requires_mutation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let command = build_session_heal_action_command(
        resolved_config_path,
        current_session_id,
        session_id,
        &tool_name,
    );
    let description = build_session_heal_action_description(
        &tool_name,
        &kind,
        note.as_deref(),
        requires_mutation,
    );
    let apply_strategy = session_heal_apply_strategy(tool_name.as_str(), source.as_str());
    let can_apply = !matches!(apply_strategy, SessionHealApplyStrategy::ObserveOnly);
    let action_id = format!("recommended:{tool_name}");

    Ok(Some(SessionHealAction {
        id: action_id,
        kind,
        source,
        tool_name,
        description,
        command,
        can_apply,
        requires_mutation,
        apply_strategy,
    }))
}

fn maybe_build_turn_checkpoint_heal_action(
    resolved_config_path: &str,
    current_session_id: &str,
    session_id: &str,
    detail: &Value,
) -> Option<SessionHealAction> {
    let requires_recovery = detail
        .pointer("/turn_checkpoint/summary/requires_recovery")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !requires_recovery {
        return None;
    }

    let command_name = crate::active_cli_command_name();
    let config_arg = crate::cli_handoff::shell_quote_argument(resolved_config_path);
    let session_arg = crate::cli_handoff::shell_quote_argument(current_session_id);
    let target_arg = crate::cli_handoff::shell_quote_argument(session_id);
    let command = format!(
        "{command_name} sessions heal --config {config_arg} --session {session_arg} {target_arg} --apply"
    );
    let description = "Run production turn-checkpoint repair for the selected session.".to_owned();

    Some(SessionHealAction {
        id: "checkpoint:repair".to_owned(),
        kind: "turn_checkpoint_repair".to_owned(),
        source: "turn_checkpoint_summary".to_owned(),
        tool_name: "turn_checkpoint_repair".to_owned(),
        description,
        command,
        can_apply: true,
        requires_mutation: true,
        apply_strategy: SessionHealApplyStrategy::TurnCheckpointRepair,
    })
}

fn build_session_heal_action_command(
    resolved_config_path: &str,
    current_session_id: &str,
    session_id: &str,
    tool_name: &str,
) -> String {
    let command_name = crate::active_cli_command_name();
    let config_arg = crate::cli_handoff::shell_quote_argument(resolved_config_path);
    let session_arg = crate::cli_handoff::shell_quote_argument(current_session_id);
    let target_arg = crate::cli_handoff::shell_quote_argument(session_id);

    match tool_name {
        "session_recover" => format!(
            "{command_name} sessions recover --config {config_arg} --session {session_arg} {target_arg}"
        ),
        "session_wait" => format!(
            "{command_name} sessions wait --config {config_arg} --session {session_arg} {target_arg}"
        ),
        "session_status" => format!(
            "{command_name} sessions status --config {config_arg} --session {session_arg} {target_arg}"
        ),
        _ => format!(
            "{command_name} sessions status --config {config_arg} --session {session_arg} {target_arg}"
        ),
    }
}

fn build_session_heal_action_description(
    tool_name: &str,
    kind: &str,
    note: Option<&str>,
    requires_mutation: bool,
) -> String {
    if let Some(note) = note {
        let trimmed_note = note.trim();
        if !trimmed_note.is_empty() {
            return trimmed_note.to_owned();
        }
    }

    match (tool_name, requires_mutation) {
        ("session_recover", true) => {
            "Apply the existing overdue async delegate recovery path.".to_owned()
        }
        ("session_wait", false) => {
            "Wait for the current session to reach a newer durable state.".to_owned()
        }
        ("session_status", false) => {
            "Re-read the current session status before choosing a mutation.".to_owned()
        }
        _ => {
            format!("Follow the recommended `{tool_name}` action for `{kind}`.")
        }
    }
}

fn session_heal_apply_strategy(tool_name: &str, source: &str) -> SessionHealApplyStrategy {
    if tool_name == "session_recover" && source == "session_recover_plan" {
        return SessionHealApplyStrategy::SessionRecover;
    }

    SessionHealApplyStrategy::ObserveOnly
}

fn session_heal_plan_json(plan: &SessionHealPlan) -> Value {
    let applyable_count = plan
        .actions
        .iter()
        .filter(|action| action.can_apply)
        .count();
    let action_count = plan.actions.len();
    let attention_count = plan.attention_hints.len();
    let actions = plan
        .actions
        .iter()
        .map(session_heal_action_json)
        .collect::<Vec<_>>();

    json!({
        "action_count": action_count,
        "applyable_count": applyable_count,
        "attention_count": attention_count,
        "attention_hints": plan.attention_hints,
        "actions": actions,
    })
}

fn session_heal_action_json(action: &SessionHealAction) -> Value {
    json!({
        "id": action.id,
        "kind": action.kind,
        "source": action.source,
        "tool_name": action.tool_name,
        "description": action.description,
        "command": action.command,
        "can_apply": action.can_apply,
        "requires_mutation": action.requires_mutation,
    })
}

fn build_session_heal_recipes(
    resolved_config_path: &str,
    current_session_id: &str,
    session_id: &str,
    plan: &SessionHealPlan,
) -> Vec<String> {
    let command_name = crate::active_cli_command_name();
    let config_arg = crate::cli_handoff::shell_quote_argument(resolved_config_path);
    let session_arg = crate::cli_handoff::shell_quote_argument(current_session_id);
    let target_arg = crate::cli_handoff::shell_quote_argument(session_id);
    let plan_recipe = format!(
        "{command_name} sessions heal --config {config_arg} --session {session_arg} {target_arg}"
    );
    let apply_recipe = format!(
        "{command_name} sessions heal --config {config_arg} --session {session_arg} {target_arg} --apply"
    );
    let mut recipes = vec![plan_recipe, apply_recipe];

    for action in &plan.actions {
        recipes.push(action.command.clone());
    }

    recipes
}

fn build_session_heal_next_steps(
    resolved_config_path: &str,
    current_session_id: &str,
    session_id: &str,
    plan: &SessionHealPlan,
    apply: bool,
    applied_actions: &[Value],
) -> Vec<String> {
    let mut next_steps = Vec::new();
    let applyable_action_exists = plan.actions.iter().any(|action| action.can_apply);

    if !apply && applyable_action_exists {
        let command_name = crate::active_cli_command_name();
        let config_arg = crate::cli_handoff::shell_quote_argument(resolved_config_path);
        let session_arg = crate::cli_handoff::shell_quote_argument(current_session_id);
        let target_arg = crate::cli_handoff::shell_quote_argument(session_id);
        next_steps.push(format!(
            "Run `{command_name} sessions heal --config {config_arg} --session {session_arg} {target_arg} --apply` to execute the bounded self-heal actions."
        ));
    }

    if !apply && !applyable_action_exists && !plan.actions.is_empty() {
        let first_action = plan.actions.first();
        if let Some(first_action) = first_action {
            next_steps.push(format!(
                "Follow the recommended command for the first action: `{}`.",
                first_action.command
            ));
        }
    }

    if plan.actions.is_empty() {
        next_steps.push(
            "No bounded self-heal action is currently available; inspect `sessions status`, `sessions events`, and `sessions wait` for more evidence."
                .to_owned(),
        );
    }

    if apply && !applied_actions.is_empty() {
        let command_name = crate::active_cli_command_name();
        let config_arg = crate::cli_handoff::shell_quote_argument(resolved_config_path);
        let session_arg = crate::cli_handoff::shell_quote_argument(current_session_id);
        let target_arg = crate::cli_handoff::shell_quote_argument(session_id);
        next_steps.push(format!(
            "Re-run `{command_name} sessions status --config {config_arg} --session {session_arg} {target_arg}` to confirm the refreshed durable state."
        ));
    }

    next_steps
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
        "heal" => render_sessions_heal_text(&execution.payload)?,
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

pub(crate) fn sanitize_terminal_text(value: &str) -> String {
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

fn render_sessions_heal_text(payload: &Value) -> CliResult<String> {
    let detail = payload
        .get("detail")
        .ok_or_else(|| "sessions heal payload missing detail".to_owned())?;
    let plan = payload
        .get("plan")
        .ok_or_else(|| "sessions heal payload missing plan".to_owned())?;
    let recipes = payload
        .get("recipes")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions heal payload missing recipes".to_owned())?;
    let next_steps = payload
        .get("next_steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions heal payload missing next_steps".to_owned())?;
    let applied_actions = payload
        .get("applied_actions")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions heal payload missing applied_actions".to_owned())?;

    let detail_lines = render_session_inspection_lines(detail)?;
    let plan_lines = render_session_heal_plan_lines(plan)?;
    let mut sections = vec![
        ("self-heal plan", plan_lines),
        ("session detail", detail_lines),
    ];

    if !applied_actions.is_empty() {
        let applied_lines = render_session_heal_applied_lines(applied_actions);
        sections.insert(1, ("applied actions", applied_lines));
    }

    let mut recipes_lines = Vec::new();
    for recipe in recipes {
        let raw_recipe = recipe.as_str().unwrap_or("");
        let sanitized_recipe = sanitize_terminal_text(raw_recipe);
        recipes_lines.push(format!("- {sanitized_recipe}"));
    }
    if !recipes_lines.is_empty() {
        sections.push(("recipes", recipes_lines));
    }

    let mut next_lines = Vec::new();
    for step in next_steps {
        let raw_step = step.as_str().unwrap_or("");
        let sanitized_step = sanitize_terminal_text(raw_step);
        next_lines.push(format!("- {sanitized_step}"));
    }
    if !next_lines.is_empty() {
        sections.insert(0, ("next steps", next_lines));
    }

    Ok(render_sessions_surface(
        "session self-heal",
        "session shell",
        Vec::new(),
        sections,
        vec!["Use `sessions heal --apply` only when the surfaced actions match the desired bounded recovery path.".to_owned()],
    ))
}

fn render_session_heal_plan_lines(plan: &Value) -> CliResult<Vec<String>> {
    let action_count = plan
        .get("action_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let applyable_count = plan
        .get("applyable_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let attention_count = plan
        .get("attention_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let actions = plan
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions heal plan missing actions".to_owned())?;
    let attention_hints = plan
        .get("attention_hints")
        .and_then(Value::as_array)
        .ok_or_else(|| "sessions heal plan missing attention_hints".to_owned())?;

    let mut lines = Vec::new();
    lines.push(format!(
        "actions={action_count} applyable={applyable_count} attention_hints={attention_count}"
    ));

    for action in actions {
        let rendered_action = render_session_heal_action_line(action);
        lines.push(format!("- {rendered_action}"));
    }

    for hint in attention_hints {
        let rendered_hint = hint.as_str().unwrap_or("-");
        let sanitized_hint = sanitize_terminal_text(rendered_hint);
        lines.push(format!("- hint {sanitized_hint}"));
    }

    if actions.is_empty() && attention_hints.is_empty() {
        lines.push("No bounded self-heal action is currently available.".to_owned());
    }

    Ok(lines)
}

fn render_session_heal_action_line(action: &Value) -> String {
    let id = action.get("id").and_then(Value::as_str).unwrap_or("-");
    let tool_name = action
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let kind = action.get("kind").and_then(Value::as_str).unwrap_or("-");
    let source = action.get("source").and_then(Value::as_str).unwrap_or("-");
    let can_apply = action
        .get("can_apply")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let apply_flag = if can_apply { "yes" } else { "no" };

    format!("{id} tool={tool_name} kind={kind} source={source} apply={apply_flag}")
}

fn render_session_heal_applied_lines(applied_actions: &[Value]) -> Vec<String> {
    let mut lines = Vec::new();

    for applied_action in applied_actions {
        let id = applied_action
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let tool_name = applied_action
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let status = applied_action
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("-");
        lines.push(format!("- {id} tool={tool_name} status={status}"));
    }

    lines
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
    let prompt_frame_summary =
        crate::session_prompt_frame_cli::render_prompt_frame_summary(detail.get("prompt_frame"));
    let prompt_frame_summary = sanitize_terminal_text(prompt_frame_summary.as_str());
    let safe_lane_summary =
        crate::session_runtime_truth_cli::render_safe_lane_summary(detail.get("safe_lane"));
    let safe_lane_summary = sanitize_terminal_text(safe_lane_summary.as_str());
    let turn_checkpoint_summary = crate::session_runtime_truth_cli::render_turn_checkpoint_summary(
        detail.get("turn_checkpoint"),
    );
    let turn_checkpoint_summary = sanitize_terminal_text(turn_checkpoint_summary.as_str());
    let diagnostics = detail.get("diagnostics").cloned().unwrap_or(Value::Null);
    let diagnostics_latest_provider_failover = render_session_latest_provider_failover_summary(
        diagnostics.get("latest_provider_failover"),
    );
    let diagnostics_latest_provider_failover =
        sanitize_terminal_text(diagnostics_latest_provider_failover.as_str());
    let diagnostics_recommended_action =
        render_session_recommended_action_summary(diagnostics.get("recommended_action"));
    let diagnostics_recommended_action =
        sanitize_terminal_text(diagnostics_recommended_action.as_str());
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
    lines.push(format!("prompt_frame: {prompt_frame_summary}"));
    lines.push(format!("safe_lane: {safe_lane_summary}"));
    lines.push(format!("turn_checkpoint: {turn_checkpoint_summary}"));
    lines.push(format!(
        "latest_provider_failover: {diagnostics_latest_provider_failover}"
    ));
    lines.push(format!(
        "recommended_action: {diagnostics_recommended_action}"
    ));
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

fn render_session_latest_provider_failover_summary(diagnostic: Option<&Value>) -> String {
    let Some(diagnostic) = diagnostic else {
        return "-".to_owned();
    };

    let reason = diagnostic
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let model = diagnostic
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let stage = diagnostic
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let request_id = diagnostic
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("-");

    format!("reason={reason} model={model} stage={stage} request_id={request_id}")
}

fn render_session_recommended_action_summary(action: Option<&Value>) -> String {
    let Some(action) = action else {
        return "-".to_owned();
    };

    let tool_name = action
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let kind = action.get("kind").and_then(Value::as_str).unwrap_or("-");
    let source = action.get("source").and_then(Value::as_str).unwrap_or("-");

    format!("tool={tool_name} kind={kind} source={source}")
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        build_session_heal_plan, render_session_heal_plan_lines, render_session_inspection_lines,
    };

    #[test]
    fn render_session_inspection_lines_includes_diagnostics_summaries() {
        let detail = json!({
            "session": {
                "session_id": "session-1",
                "kind": "root",
                "state": "running",
                "parent_session_id": null,
                "label": "Root",
                "turn_count": 3,
                "last_turn_at": 123,
                "last_error": "rate_limited"
            },
            "workflow": {},
            "terminal_outcome_state": "not_terminal",
            "terminal_outcome": null,
            "recovery": null,
            "recent_events": [],
            "diagnostics": {
                "latest_provider_failover": {
                    "reason": "rate_limited",
                    "model": "gpt-4o",
                    "stage": "status_failure",
                    "request_id": "req-123"
                },
                "recommended_action": {
                    "tool_name": "session_wait",
                    "kind": "follow_resume_recipe",
                    "source": "task_progress_resume_recipe"
                }
            }
        });

        let lines = render_session_inspection_lines(&detail).expect("render lines");

        assert!(
            lines.iter().any(|line| {
                line == "latest_provider_failover: reason=rate_limited model=gpt-4o stage=status_failure request_id=req-123"
            }),
            "expected latest_provider_failover summary, got: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line == "recommended_action: tool=session_wait kind=follow_resume_recipe source=task_progress_resume_recipe"
            }),
            "expected recommended_action summary, got: {lines:#?}"
        );
    }

    #[test]
    fn build_session_heal_plan_adds_turn_checkpoint_repair_action_when_needed() {
        let detail = json!({
            "diagnostics": {
                "attention_hints": ["checkpoint attention"]
            },
            "turn_checkpoint": {
                "summary": {
                    "requires_recovery": true
                }
            }
        });

        let plan = build_session_heal_plan("/tmp/loong.toml", "ops-root", "session-1", &detail)
            .expect("build heal plan");

        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.actions[0].tool_name, "turn_checkpoint_repair");
        assert!(plan.actions[0].can_apply);
        assert_eq!(
            plan.attention_hints,
            vec!["checkpoint attention".to_owned()]
        );
    }

    #[test]
    fn render_session_heal_plan_lines_surface_actions_and_hints() {
        let plan = json!({
            "action_count": 1,
            "applyable_count": 1,
            "attention_count": 1,
            "actions": [{
                "id": "recommended:session_recover",
                "tool_name": "session_recover",
                "kind": "queued_async_overdue_marked_failed",
                "source": "session_recover_plan",
                "can_apply": true
            }],
            "attention_hints": ["provider_failover_present reason=rate_limited"]
        });

        let lines = render_session_heal_plan_lines(&plan).expect("render heal plan lines");

        assert!(
            lines.iter().any(|line| {
                line.contains("actions=1")
                    && line.contains("applyable=1")
                    && line.contains("attention_hints=1")
            }),
            "expected heal plan summary, got: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| {
                line.contains("recommended:session_recover")
                    && line.contains("tool=session_recover")
                    && line.contains("apply=yes")
            }),
            "expected action line, got: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| { line.contains("hint provider_failover_present") }),
            "expected attention hint line, got: {lines:#?}"
        );
    }
}
