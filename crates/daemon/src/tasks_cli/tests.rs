use super::*;
use crate::mvp;
use crate::tasks_cli::{render_task_brief_line, render_task_detail_lines};
use mvp::session::repository::{
    NewSessionEvent, NewSessionRecord, SessionKind, SessionRepository, SessionState,
};

fn build_task_payload(
    session_state: &str,
    phase: &str,
    task_progress_status: Option<&str>,
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
            "base_tool_ids": ["read", "web.fetch"],
            "effective_tool_ids": ["read"],
            "effective_runtime_narrowing": {
                "web_fetch": {
                    "allowed_domains": ["docs.example.com"],
                },
            },
        })
    } else {
        json!({
            "base_tool_ids": ["read"],
            "effective_tool_ids": ["read"],
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
    let task_progress = task_progress_status
        .map(|status| json!({ "status": status }))
        .unwrap_or(Value::Null);
    let terminal_outcome_state = if session_state == "completed"
        || session_state == "failed"
        || session_state == "timed_out"
    {
        json!("present")
    } else {
        Value::Null
    };
    let recovery = if recovered {
        json!({ "kind": "delegate_terminal_finalize_persist_failed" })
    } else {
        Value::Null
    };
    let task_status = build_task_status_payload(
        &session,
        &delegate,
        &task_progress,
        &terminal_outcome_state,
        &recovery,
        &json!(approval_requests),
        &approval_summary,
        &tool_policy,
        &recent_events,
    );

    json!({
        "task_id": "delegate:task-1",
        "task_session_id": "task-owner",
        "owner_session_id": "task-owner",
        "scope_session_id": "ops-root",
        "label": "Release Check",
        "session_state": session_state,
        "phase": phase,
        "workflow": {
            "task_progress": task_progress,
        },
        "timeout_seconds": 60,
        "last_error": Value::Null,
        "approval": {
            "matched_count": approval_requests.len(),
            "attention_summary": approval_summary,
        },
        "tool_policy": tool_policy,
        "task_status": task_status,
        "terminal_outcome_state": terminal_outcome_state,
        "recovery": recovery,
    })
}

#[test]
fn build_task_status_payload_uses_approval_action_and_tool_narrowing_signal() {
    let task = build_task_payload(
        "ready",
        "queued",
        None,
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
    let task = build_task_payload("failed", "failed", Some("failed"), None, false, true, None);
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
    let task = build_task_payload(
        "running",
        "running",
        None,
        None,
        false,
        false,
        Some("overdue"),
    );
    let task_status = &task["task_status"];

    assert_eq!(task_status["kind"], "overdue");
    assert_eq!(task_status["blocked"], true);
    assert_eq!(task_status["status"], "overdue");
    assert_eq!(task_status["needs_attention"], true);
    assert_eq!(task_status["next_action"], "recover");
}

#[test]
fn build_task_status_payload_prefers_queued_task_progress_handle_over_session_phase_guess() {
    let approval_requests = Vec::<Value>::new();
    let approval_summary = json!({
        "needs_attention_count": 0_u64,
    });
    let tool_policy = json!({
        "base_tool_ids": ["read"],
        "effective_tool_ids": ["read"],
        "effective_runtime_narrowing": Value::Null,
    });
    let recent_events = json!([]);
    let delegate = json!({
        "phase": "running",
        "staleness": Value::Null,
        "cancellation": Value::Null,
    });
    let session = json!({
        "state": "running",
    });
    let task_progress = json!({
        "status": "active",
        "active_handles": [
            {
                "handle_kind": "background_task_host",
                "state": "queued",
            }
        ],
        "resume_recipe": {
            "recommended_tool": "task_wait",
            "task_session_id": "task-owner",
        }
    });

    let task_status = build_task_status_payload(
        &session,
        &delegate,
        &task_progress,
        &Value::Null,
        &Value::Null,
        &json!(approval_requests),
        &approval_summary,
        &tool_policy,
        &recent_events,
    );

    assert_eq!(task_status["kind"], "queued");
    assert_eq!(task_status["status"], "queued");
    assert_eq!(task_status["next_action"], "wait");
}

#[test]
fn render_task_detail_lines_surface_task_status_summary() {
    let task = build_task_payload(
        "ready",
        "queued",
        None,
        Some("resolve_request"),
        true,
        false,
        None,
    );
    let rendered = render_task_detail_lines(&task).expect("render task detail");
    let joined = rendered.join("\n");

    assert!(joined.contains("task_status: approval_pending"));
    assert!(joined.contains("task_session_id: task-owner"));
    assert!(joined.contains("owner_session_id: task-owner"));
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
        None,
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
    let task_target = ResolvedCliTaskTarget {
        task_id: "delegate:task-1".to_owned(),
        owner_session_id: "delegate-session-1".to_owned(),
        task_session_id: "delegate-session-1".to_owned(),
    };

    let (payload, lookup_error) = load_best_effort_task_approvals_payload(
        &memory_config,
        &tool_config,
        "ops-root",
        &task_target,
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
    let task_target = ResolvedCliTaskTarget {
        task_id: "delegate:task-1".to_owned(),
        owner_session_id: "delegate-session-1".to_owned(),
        task_session_id: "delegate-session-1".to_owned(),
    };

    let (payload, lookup_error) = load_best_effort_task_tool_policy_payload(
        &memory_config,
        &tool_config,
        "ops-root",
        &task_target,
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
        &ResolvedCliTaskTarget {
            task_id: "delegate:task-1".to_owned(),
            task_session_id: "delegate-session-1".to_owned(),
            owner_session_id: "delegate-session-1".to_owned(),
        },
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
    assert_eq!(detail["task_session_id"], "delegate-session-1");
    assert_eq!(detail["owner_session_id"], "delegate-session-1");
    assert_eq!(detail["approval_lookup_error"], "approval lookup failed");
    assert_eq!(
        detail["tool_policy_lookup_error"],
        "tool policy lookup failed"
    );
    assert_eq!(detail["tool_policy"], Value::Null);
    assert_eq!(detail["approval"]["matched_count"], 0);
    assert_eq!(detail["terminal_outcome_state"], "missing");
}

fn isolated_memory_config(name: &str) -> mvp::session::store::SessionStoreConfig {
    let root = std::env::temp_dir().join(format!(
        "loong-tasks-cli-{name}-{}-{}",
        std::process::id(),
        current_unix_timestamp()
    ));
    let _ = std::fs::create_dir_all(&root);
    mvp::session::store::SessionStoreConfig {
        sqlite_path: Some(root.join("memory.sqlite3")),
        ..mvp::session::store::SessionStoreConfig::default()
    }
}

#[tokio::test]
async fn execute_cancel_command_uses_canonical_task_identity() {
    let store = isolated_memory_config("task-cancel");
    let repo = SessionRepository::new(&store).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "task-owner".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Task Owner".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create task owner");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task_scope": { "task_id": "task-root" },
            "task_session_id": "task-owner",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: mvp::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: mvp::task_progress::task_progress_event_payload(
            "unit_test",
            &mvp::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "background_task_host".to_owned(),
                status: mvp::task_progress::TaskProgressStatus::Active,
                intent_summary: Some("Cancel queued task".to_owned()),
                verification_state: Some(mvp::task_progress::TaskVerificationState::NotStarted),
                active_handles: vec![mvp::task_progress::TaskActiveHandleRecord {
                    handle_kind: "background_task_host".to_owned(),
                    handle_id: "task-owner".to_owned(),
                    state: "queued".to_owned(),
                    last_event_at: Some(123),
                    stop_condition: "delegate_child_terminal_or_recovery".to_owned(),
                }],
                resume_recipe: Some(mvp::task_progress::TaskResumeRecipeRecord {
                    recommended_tool: "task_wait".to_owned(),
                    task_session_id: "task-owner".to_owned(),
                    note: Some("Use task_wait for the durable queued background task.".to_owned()),
                }),
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress");
    let memory_config = mvp::memory::runtime_config::MemoryRuntimeConfig::for_sqlite_path(
        store.sqlite_path.clone().expect("sqlite path"),
    );
    let tool_config = mvp::config::ToolConfig::default();
    let payload = execute_cancel_command(
        "/tmp/loong.toml",
        "root-session",
        &memory_config,
        &tool_config,
        "task-root",
        true,
    )
    .await
    .expect("cancel command");

    assert_eq!(payload["task"]["task_id"], "task-root");
    assert_eq!(payload["task"]["owner_session_id"], "task-owner");
    assert_eq!(payload["action"]["kind"], "queued_async_cancelled");
    assert_eq!(payload["dry_run"], true);
}

#[tokio::test]
async fn execute_recover_command_uses_canonical_task_identity() {
    let store = isolated_memory_config("task-recover");
    let repo = SessionRepository::new(&store).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "task-owner".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Task Owner".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create task owner");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task_scope": { "task_id": "task-root" },
            "task_session_id": "task-owner",
            "timeout_seconds": 1
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: mvp::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: mvp::task_progress::task_progress_event_payload(
            "unit_test",
            &mvp::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "background_task_host".to_owned(),
                status: mvp::task_progress::TaskProgressStatus::Blocked,
                intent_summary: Some("Recover overdue task".to_owned()),
                verification_state: Some(mvp::task_progress::TaskVerificationState::Pending),
                active_handles: vec![mvp::task_progress::TaskActiveHandleRecord {
                    handle_kind: "background_task_host".to_owned(),
                    handle_id: "task-owner".to_owned(),
                    state: "queued".to_owned(),
                    last_event_at: Some(123),
                    stop_condition: "delegate_child_terminal_or_recovery".to_owned(),
                }],
                resume_recipe: Some(mvp::task_progress::TaskResumeRecipeRecord {
                    recommended_tool: "task_wait".to_owned(),
                    task_session_id: "task-owner".to_owned(),
                    note: Some("Use task_wait for the durable queued background task.".to_owned()),
                }),
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress");
    let memory_config = mvp::memory::runtime_config::MemoryRuntimeConfig::for_sqlite_path(
        store.sqlite_path.clone().expect("sqlite path"),
    );
    let tool_config = mvp::config::ToolConfig::default();
    let payload = execute_recover_command(
        "/tmp/loong.toml",
        "root-session",
        &memory_config,
        &tool_config,
        "task-root",
        true,
    )
    .await
    .expect("recover command");

    assert_eq!(payload["task"]["task_id"], "task-root");
    assert_eq!(payload["task"]["owner_session_id"], "task-owner");
    assert!(
        !payload["result"].is_null(),
        "task recover should surface a mutation result"
    );
    assert_eq!(payload["dry_run"], true);
}
