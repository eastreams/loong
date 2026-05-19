use loong_spec::CliResult;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
pub(super) struct TaskStatusSummary {
    pub(super) is_background_task: bool,
    pub(super) is_overdue: bool,
}

pub(super) fn summarize_task_status_payload(
    status_payload: &Value,
) -> CliResult<TaskStatusSummary> {
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
    Ok(TaskStatusSummary {
        is_background_task: session_kind == "delegate_child" && delegate_mode == "async",
        is_overdue: staleness_state == "overdue",
    })
}

pub(super) fn build_task_status_payload(
    session: &Value,
    delegate: &Value,
    task_progress: &Value,
    terminal_outcome_state: &Value,
    recovery: &Value,
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
    let has_approval_attention = approval_attention_summary
        .get("needs_attention_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        > 0;
    let approval_primary_action = primary_approval_action(approval_requests).map(ToOwned::to_owned);
    let recovered = recent_events_contains_kind(recent_events, "delegate_recovery_applied");
    let tool_narrowing_active = task_tool_narrowing_active(tool_policy);
    let task_progress_status = task_progress
        .get("status")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let task_progress_handle_state = task_progress
        .get("active_handles")
        .and_then(Value::as_array)
        .and_then(|handles| handles.first())
        .and_then(|handle| handle.get("state"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let terminal_outcome_state = terminal_outcome_state
        .as_str()
        .filter(|value| !value.trim().is_empty());
    let recovery_kind = recovery
        .get("kind")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let kind = derive_task_status_kind(
        session_state,
        phase,
        staleness_state,
        cancellation_state,
        has_approval_attention,
        task_progress_status,
        task_progress_handle_state,
        terminal_outcome_state,
        recovery_kind,
    );
    let signals = build_task_status_signals(
        kind,
        recovered,
        tool_narrowing_active,
        has_approval_attention,
        staleness_state,
        cancellation_state,
    );
    json!({
        "status": kind,
        "kind": kind,
        "display": render_task_status_display(kind, recovered),
        "blocked": task_status_is_blocked(kind),
        "terminal": task_status_is_terminal(kind),
        "needs_attention": task_status_needs_attention(kind, approval_primary_action.as_deref()),
        "next_action": task_status_next_action(kind, approval_primary_action.as_deref()),
        "approval_primary_action": approval_primary_action,
        "recovered": recovered,
        "tool_narrowing_active": tool_narrowing_active,
        "signals": signals,
    })
}

pub(super) fn unknown_task_status_payload() -> Value {
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
    task_progress_status: Option<&str>,
    task_progress_handle_state: Option<&str>,
    terminal_outcome_state: Option<&str>,
    recovery_kind: Option<&str>,
) -> &'static str {
    if let Some(task_progress_status) = task_progress_status {
        return match task_progress_status {
            "completed" => "completed",
            "failed" => "failed",
            "blocked" => {
                if terminal_outcome_state == Some("present") || recovery_kind.is_some() {
                    "failed"
                } else {
                    "blocked"
                }
            }
            "waiting" => "waiting",
            "verifying" | "active" => {
                if task_progress_handle_state == Some("queued") {
                    "queued"
                } else if terminal_outcome_state == Some("present") {
                    match session_state {
                        "completed" => "completed",
                        "failed" | "timed_out" => "failed",
                        _ => "running",
                    }
                } else {
                    "running"
                }
            }
            _ => "running",
        };
    }
    if session_state == "completed" {
        return "completed";
    }
    if session_state == "failed" {
        return "failed";
    }
    if session_state == "timed_out" {
        return "timed_out";
    }
    if staleness_state == Some("overdue") {
        return "overdue";
    }
    if cancellation_state == Some("requested") {
        return "cancel_requested";
    }
    if has_approval_attention {
        return "approval_pending";
    }
    if session_state == "running" {
        return "running";
    }
    if session_state == "ready" || phase == "queued" {
        return "queued";
    }
    "unknown"
}

fn render_task_status_display(kind: &str, recovered: bool) -> String {
    if recovered {
        format!("{kind} (recovered)")
    } else {
        kind.to_owned()
    }
}

fn task_status_is_blocked(kind: &str) -> bool {
    matches!(kind, "approval_pending" | "overdue")
}

fn task_status_is_terminal(kind: &str) -> bool {
    matches!(kind, "completed" | "failed" | "timed_out")
}

fn task_status_needs_attention(kind: &str, approval_primary_action: Option<&str>) -> bool {
    matches!(
        kind,
        "approval_pending" | "overdue" | "failed" | "timed_out"
    ) || approval_primary_action.is_some()
}

fn task_status_next_action(kind: &str, approval_primary_action: Option<&str>) -> String {
    if let Some(approval_primary_action) = approval_primary_action {
        return approval_primary_action.to_owned();
    }
    match kind {
        "approval_pending" => "status".to_owned(),
        "overdue" => "recover".to_owned(),
        "queued" | "running" | "cancel_requested" => "wait".to_owned(),
        "completed" | "failed" | "timed_out" => "events".to_owned(),
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
    if task_status_is_terminal(kind) {
        signals.push("terminal".to_owned());
    }
    signals
}

fn recent_events_contains_kind(recent_events: &Value, expected_kind: &str) -> bool {
    recent_events.as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event
                .get("event_kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                == expected_kind
        })
    })
}

fn primary_approval_action(approval_requests: &Value) -> Option<&str> {
    approval_requests.as_array()?.iter().find_map(|request| {
        request
            .get("attention")
            .and_then(|value| value.get("primary_action"))
            .and_then(Value::as_str)
    })
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
    let runtime_narrowing_active = tool_policy
        .get("effective_runtime_narrowing")
        .cloned()
        .unwrap_or(Value::Null)
        != Value::Null;
    effective_tool_ids != base_tool_ids || runtime_narrowing_active
}
