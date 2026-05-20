use loong_app as mvp;
use loong_spec::CliResult;
use serde_json::Value;

use super::{TasksCommandExecution, unknown_task_status_payload};

pub fn render_tasks_cli_text(execution: &TasksCommandExecution) -> CliResult<String> {
    let command = execution
        .payload
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "tasks CLI payload missing command".to_owned())?;

    match command {
        "create" => render_tasks_create_text(&execution.payload),
        "list" => render_tasks_list_text(&execution.payload),
        "status" => render_tasks_status_text(&execution.payload),
        "events" => render_tasks_events_text(&execution.payload),
        "wait" => render_tasks_wait_text(&execution.payload),
        "cancel" | "recover" => render_tasks_mutation_text(&execution.payload),
        other => Err(format!("unknown tasks CLI render command `{other}`")),
    }
}

pub fn render_task_brief_line(task: &Value) -> CliResult<String> {
    let task_id = required_string_field(task, "task_id", "task summary")?;
    let task_status = task
        .get("task_status")
        .cloned()
        .unwrap_or_else(unknown_task_status_payload);
    let line = format!(
        "{} status={} blocked={} state={} workflow_phase={} delegate_phase={} label={} owner_kind={} approval_attention={} signals={}",
        crate::sessions_cli::sanitize_terminal_text(task_id.as_str()),
        task_status
            .get("display")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        task_status
            .get("blocked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        task.get("session_state")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        task.get("workflow")
            .and_then(|value| value.get("phase"))
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        task.get("phase")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        crate::sessions_cli::sanitize_terminal_text(
            task.get("label").and_then(Value::as_str).unwrap_or("-")
        ),
        crate::sessions_cli::sanitize_terminal_text(
            task.get("owner_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ),
        task.get("approval")
            .and_then(|value| value.get("attention_summary"))
            .and_then(|value| value.get("needs_attention_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        task_status
            .get("signals")
            .and_then(Value::as_array)
            .map(|values| render_string_array(values))
            .unwrap_or_else(|| "-".to_owned()),
    );
    Ok(line)
}

pub fn render_task_detail_lines(task: &Value) -> CliResult<Vec<String>> {
    let task_id = required_string_field(task, "task_id", "task detail")?;
    let task_status = task
        .get("task_status")
        .cloned()
        .unwrap_or_else(unknown_task_status_payload);
    let requested_tool_ids =
        render_task_tool_policy_tool_ids(task, "visible_requested_tool_ids", "requested_tool_ids");
    let effective_tool_ids =
        render_task_tool_policy_tool_ids(task, "visible_effective_tool_ids", "effective_tool_ids");
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
    let prompt_frame_summary =
        crate::session_prompt_frame_cli::render_prompt_frame_summary(task.get("prompt_frame"));
    let safe_lane_summary =
        crate::session_runtime_truth_cli::render_safe_lane_summary(task.get("safe_lane"));
    let turn_checkpoint_summary = crate::session_runtime_truth_cli::render_turn_checkpoint_summary(
        task.get("turn_checkpoint"),
    );

    let mut lines = vec![
        format!(
            "task_id: {}",
            crate::sessions_cli::sanitize_terminal_text(task_id.as_str())
        ),
        format!(
            "task_session_id: {}",
            crate::sessions_cli::sanitize_terminal_text(
                task.get("task_session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            )
        ),
        format!(
            "owner_session_id: {}",
            crate::sessions_cli::sanitize_terminal_text(
                task.get("owner_session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            )
        ),
        format!(
            "scope_session_id: {}",
            crate::sessions_cli::sanitize_terminal_text(
                task.get("scope_session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            )
        ),
        format!(
            "label: {}",
            crate::sessions_cli::sanitize_terminal_text(
                task.get("label").and_then(Value::as_str).unwrap_or("-")
            )
        ),
        format!(
            "task_status: {}",
            task_status
                .get("display")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ),
        format!(
            "task_blocked: {}",
            task_status
                .get("blocked")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "task_needs_attention: {}",
            task_status
                .get("needs_attention")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
        format!(
            "task_next_action: {}",
            task_status
                .get("next_action")
                .and_then(Value::as_str)
                .unwrap_or("status")
        ),
        format!(
            "task_signals: {}",
            task_status
                .get("signals")
                .and_then(Value::as_array)
                .map(|values| render_string_array(values))
                .unwrap_or_else(|| "-".to_owned())
        ),
        format!(
            "state: {}",
            task.get("session_state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ),
        format!(
            "workflow_id: {}",
            nested_string(task, &["workflow", "workflow_id"]).unwrap_or("-")
        ),
        format!(
            "workflow_phase: {}",
            nested_string(task, &["workflow", "phase"]).unwrap_or("-")
        ),
        format!(
            "workflow_operation_kind: {}",
            nested_string(task, &["workflow", "operation_kind"]).unwrap_or("-")
        ),
        format!(
            "workflow_operation_scope: {}",
            nested_string(task, &["workflow", "operation_scope"]).unwrap_or("-")
        ),
        format!(
            "workflow_task_session_id: {}",
            nested_string(task, &["workflow", "task_session_id"]).unwrap_or("-")
        ),
        format!(
            "workflow_binding_mode: {}",
            nested_string(task, &["workflow", "binding", "mode"]).unwrap_or("-")
        ),
        format!(
            "workflow_execution_surface: {}",
            nested_string(task, &["workflow", "binding", "execution_surface"]).unwrap_or("-")
        ),
        format!(
            "workflow_worktree_id: {}",
            nested_string(task, &["workflow", "binding", "worktree", "worktree_id"]).unwrap_or("-")
        ),
        format!(
            "workflow_workspace_root: {}",
            nested_string(task, &["workflow", "binding", "worktree", "workspace_root"])
                .unwrap_or("-")
        ),
        format!(
            "phase: {}",
            task.get("phase")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ),
        format!(
            "owner_kind: {}",
            task.get("owner_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ),
        format!(
            "timeout_seconds: {}",
            task.get("timeout_seconds")
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_owned())
        ),
        format!(
            "last_error: {}",
            crate::sessions_cli::sanitize_terminal_text(
                task.get("last_error")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
            )
        ),
        format!(
            "approval_requests: {}",
            task.get("approval")
                .and_then(|value| value.get("matched_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
        format!(
            "approval_attention: {}",
            task.get("approval")
                .and_then(|value| value.get("attention_summary"))
                .and_then(|value| value.get("needs_attention_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
    ];
    if requested_tool_ids != "-" {
        lines.push(format!(
            "requested_tool_ids: {}",
            crate::sessions_cli::sanitize_terminal_text(requested_tool_ids.as_str())
        ));
    }
    lines.extend([
        format!(
            "effective_tool_ids: {}",
            crate::sessions_cli::sanitize_terminal_text(effective_tool_ids.as_str())
        ),
        format!(
            "effective_runtime_narrowing: {}",
            crate::sessions_cli::sanitize_terminal_text(rendered_runtime_narrowing.as_str())
        ),
        format!(
            "prompt_frame: {}",
            crate::sessions_cli::sanitize_terminal_text(prompt_frame_summary.as_str())
        ),
        format!(
            "safe_lane: {}",
            crate::sessions_cli::sanitize_terminal_text(safe_lane_summary.as_str())
        ),
        format!(
            "turn_checkpoint: {}",
            crate::sessions_cli::sanitize_terminal_text(turn_checkpoint_summary.as_str())
        ),
    ]);
    if let Some(approval_lookup_error) = task
        .get("approval_lookup_error")
        .and_then(Value::as_str)
        .map(crate::sessions_cli::sanitize_terminal_text)
    {
        lines.push(format!("approval_lookup_error: {approval_lookup_error}"));
    }
    if let Some(tool_policy_lookup_error) = task
        .get("tool_policy_lookup_error")
        .and_then(Value::as_str)
        .map(crate::sessions_cli::sanitize_terminal_text)
    {
        lines.push(format!(
            "tool_policy_lookup_error: {tool_policy_lookup_error}"
        ));
    }
    Ok(lines)
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

    let mut queued_lines = vec![format!(
        "background task queued from scope session `{}`",
        crate::sessions_cli::sanitize_terminal_text(scope)
    )];
    queued_lines.extend(render_task_detail_lines(task)?);
    append_task_lookup_error_line(payload, &mut queued_lines);

    let next_lines = next_steps
        .iter()
        .filter_map(Value::as_str)
        .map(crate::sessions_cli::sanitize_terminal_text)
        .map(|text| format!("- {text}"))
        .collect::<Vec<_>>();
    let recipe_lines = recipes
        .iter()
        .filter_map(Value::as_str)
        .map(crate::sessions_cli::sanitize_terminal_text)
        .map(|text| format!("- {text}"))
        .collect::<Vec<_>>();

    let mut sections = Vec::new();
    if !next_lines.is_empty() {
        sections.push(("next steps", next_lines));
    }
    if !recipe_lines.is_empty() {
        sections.push(("recipes", recipe_lines));
    }
    sections.push(("queued task", queued_lines));

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

    let mut lines = vec![format!(
        "visible background tasks from scope session `{}`: {returned_count}/{matched_count}",
        crate::sessions_cli::sanitize_terminal_text(scope)
    )];
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
        lines.push(format!("- {}", render_task_brief_line(task)?));
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
    Ok(render_tasks_surface(
        "task detail",
        "background tasks",
        Vec::new(),
        vec![("task", render_task_detail_lines(task)?)],
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

    let mut lines = vec![format!(
        "events for `{}` (next_after_id={next_after_id})",
        crate::sessions_cli::sanitize_terminal_text(task_id)
    )];
    if events.is_empty() {
        lines.push("No newer events.".to_owned());
    } else {
        for event in events {
            let event_id = event.get("id").and_then(Value::as_i64).unwrap_or_default();
            let event_kind = event
                .get("event_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let ts = event.get("ts").and_then(Value::as_i64).unwrap_or_default();
            lines.push(format!(
                "- #{event_id} {} ts={ts}",
                crate::sessions_cli::sanitize_terminal_text(event_kind)
            ));
        }
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

    let mut lines = vec![format!(
        "wait result: {wait_status} (next_after_id={next_after_id})"
    )];
    lines.extend(render_task_detail_lines(task)?);
    if !events.is_empty() {
        lines.push("observed events:".to_owned());
        for event in events {
            let event_id = event.get("id").and_then(Value::as_i64).unwrap_or_default();
            let event_kind = event
                .get("event_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            lines.push(format!(
                "- #{event_id} {}",
                crate::sessions_cli::sanitize_terminal_text(event_kind)
            ));
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

    let mut lines = vec![format!("{command} dry_run={dry_run}")];
    if let Some(result) = result {
        lines.push(format!(
            "result: {}",
            crate::sessions_cli::sanitize_terminal_text(result)
        ));
    }
    if let Some(message) = message {
        lines.push(format!(
            "message: {}",
            crate::sessions_cli::sanitize_terminal_text(message)
        ));
    }
    if !action.is_null() {
        lines.push("action:".to_owned());
        lines.push(
            serde_json::to_string_pretty(&action)
                .map_err(|error| format!("render action failed: {error}"))?,
        );
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

fn append_task_lookup_error_line(payload: &Value, lines: &mut Vec<String>) {
    if let Some(task_lookup_error) = payload.get("task_lookup_error").and_then(Value::as_str) {
        lines.push(format!(
            "task_lookup_error: {}",
            crate::sessions_cli::sanitize_terminal_text(task_lookup_error)
        ));
    }
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
    let items = values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if items.is_empty() {
        "-".to_owned()
    } else {
        items.join(", ")
    }
}

fn required_string_field(value: &Value, field: &str, context: &str) -> CliResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{context} missing string field `{field}`"))
}

fn nested_string<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str()
}
