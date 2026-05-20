use std::fs;

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use loong_kernel::mailbox::{AgentPath, MailboxContent};
use rusqlite::params;
use serde_json::{Value, json};
use tokio::time::{Duration, Instant, sleep};

use crate::config::{SessionVisibility, ToolConfig};
use crate::conversation::{InterAgentMessage, mailbox_for_session};
use crate::session::repository::{
    FinalizeSessionTerminalRequest, NewSessionEvent, NewSessionRecord, SessionEventRecord,
    SessionKind, SessionRepository, SessionState, SessionSummaryRecord,
};
use crate::session::store::{SessionStoreConfig, append_session_turn_direct};

use super::{
    execute_session_tool_with_config, execute_session_tool_with_policies,
    wait_for_single_session_with_policies,
};

fn isolated_memory_config(test_name: &str) -> SessionStoreConfig {
    let base = crate::test_support::unique_temp_dir(&format!("loong-session-tools-{test_name}"));
    fs::create_dir_all(&base).expect("create session tool test root");
    let db_path = base.join("memory.sqlite3");
    SessionStoreConfig {
        sqlite_path: Some(db_path),
        runtime_config: None,
    }
}

fn execute_session_mutation_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    config: &SessionStoreConfig,
) -> Result<ToolCoreOutcome, String> {
    let mut tool_config = ToolConfig::default();
    tool_config.sessions.allow_mutation = true;
    execute_session_tool_with_policies(request, current_session_id, config, &tool_config)
}

fn overwrite_session_event_ts(
    config: &SessionStoreConfig,
    session_id: &str,
    event_kind: &str,
    ts: i64,
) {
    let db_path = config
        .sqlite_path
        .as_ref()
        .expect("sqlite path for session tools test");
    let conn = rusqlite::Connection::open(db_path).expect("open sqlite db");
    let updated = conn
        .execute(
            "UPDATE session_events
                 SET ts = ?3
                 WHERE session_id = ?1 AND event_kind = ?2",
            params![session_id, event_kind, ts],
        )
        .expect("update session event ts");
    assert!(updated > 0, "expected at least one updated event row");
}

fn overwrite_session_updated_at(config: &SessionStoreConfig, session_id: &str, ts: i64) {
    let db_path = config
        .sqlite_path
        .as_ref()
        .expect("sqlite path for session tools test");
    let conn = rusqlite::Connection::open(db_path).expect("open sqlite db");
    let updated = conn
        .execute(
            "UPDATE sessions
                 SET updated_at = ?2
                 WHERE session_id = ?1",
            params![session_id, ts],
        )
        .expect("update session updated_at");
    assert!(updated > 0, "expected at least one updated session row");
}

fn batch_result<'a>(payload: &'a Value, session_id: &str) -> &'a Value {
    payload["results"]
        .as_array()
        .expect("results array")
        .iter()
        .find(|item| item.get("session_id").and_then(Value::as_str) == Some(session_id))
        .unwrap_or_else(|| panic!("missing batch result for session `{session_id}`"))
}

#[test]
fn session_mutation_tools_can_be_explicitly_disabled() {
    let config = isolated_memory_config("session-mutation-disabled");
    let mut tool_config = ToolConfig::default();
    tool_config.sessions.allow_mutation = false;
    for tool_name in ["session_archive", "session_cancel", "session_recover"] {
        let error = execute_session_tool_with_policies(
            ToolCoreRequest {
                tool_name: tool_name.to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
            &tool_config,
        )
        .expect_err("session mutation tools should require explicit opt-in");
        let expected_error =
            format!("app_tool_disabled: session mutation tool `{tool_name}` is disabled by config");
        let matches_expected_error = error.contains(expected_error.as_str());

        assert!(
            matches_expected_error,
            "expected mutation gating error for {tool_name}, got: {error}"
        );
    }
}

#[test]
fn sessions_list_returns_current_session_and_children() {
    let config = isolated_memory_config("sessions-list");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.create_session(NewSessionRecord {
        session_id: "other-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Other".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create other");

    append_session_turn_direct("root-session", "user", "root turn", &config)
        .expect("append root turn");
    append_session_turn_direct("child-session", "assistant", "child turn", &config)
        .expect("append child turn");
    append_session_turn_direct("other-session", "user", "other turn", &config)
        .expect("append other turn");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({}),
        },
        "root-session",
        &config,
    )
    .expect("sessions_list outcome");

    let sessions = outcome.payload["sessions"]
        .as_array()
        .expect("sessions array");
    let ids: Vec<&str> = sessions
        .iter()
        .filter_map(|item: &Value| item.get("session_id"))
        .filter_map(Value::as_str)
        .collect();
    assert!(ids.contains(&"root-session"));
    assert!(ids.contains(&"child-session"));
    assert!(!ids.contains(&"other-session"));
}

#[test]
fn sessions_list_respects_self_visibility_policy() {
    let config = isolated_memory_config("sessions-list-self-only");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    let mut tool_config = ToolConfig::default();
    tool_config.sessions.visibility = SessionVisibility::SelfOnly;

    let outcome = execute_session_tool_with_policies(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({}),
        },
        "root-session",
        &config,
        &tool_config,
    )
    .expect("sessions_list outcome");

    let sessions = outcome.payload["sessions"]
        .as_array()
        .expect("sessions array");
    let ids: Vec<&str> = sessions
        .iter()
        .filter_map(|item: &Value| item.get("session_id"))
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(ids, vec!["root-session"]);
}

#[test]
fn sessions_list_filters_visible_sessions_by_state_kind_and_parent() {
    let config = isolated_memory_config("sessions-list-filtered");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-running".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Running Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create running child");
    repo.create_session(NewSessionRecord {
        session_id: "child-completed".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Completed Child".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create completed child");
    repo.create_session(NewSessionRecord {
        session_id: "grandchild-running".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("child-running".to_owned()),
        label: Some("Grandchild".to_owned()),
        state: SessionState::Running,
    })
    .expect("create grandchild");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({
                "state": "running",
                "kind": "delegate_child",
                "parent_session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("sessions_list outcome");

    let sessions = outcome.payload["sessions"]
        .as_array()
        .expect("sessions array");
    let ids: Vec<&str> = sessions
        .iter()
        .filter_map(|item: &Value| item.get("session_id"))
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(ids, vec!["child-running"]);
    assert_eq!(outcome.payload["matched_count"], 1);
    assert_eq!(outcome.payload["returned_count"], 1);
}

#[test]
fn sessions_list_excludes_archived_sessions_by_default() {
    let config = isolated_memory_config("sessions-list-excludes-archived");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "archived-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Archived".to_owned()),
        state: SessionState::Running,
    })
    .expect("create archived child");
    repo.create_session(NewSessionRecord {
        session_id: "visible-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Visible".to_owned()),
        state: SessionState::Running,
    })
    .expect("create visible child");
    for session_id in ["archived-child", "visible-child"] {
        repo.finalize_session_terminal(
            session_id,
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({ "result": "ok" }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({ "child_session_id": session_id }),
                frozen_result: None,
            },
        )
        .expect("finalize child");
    }

    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_archive".to_owned(),
            payload: json!({
                "session_id": "archived-child"
            }),
        },
        "root-session",
        &config,
    )
    .expect("archive child");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({}),
        },
        "root-session",
        &config,
    )
    .expect("sessions_list outcome");

    let sessions = outcome.payload["sessions"]
        .as_array()
        .expect("sessions array");
    let ids: Vec<&str> = sessions
        .iter()
        .filter_map(|item: &Value| item.get("session_id"))
        .filter_map(Value::as_str)
        .collect();
    assert!(ids.contains(&"root-session"));
    assert!(ids.contains(&"visible-child"));
    assert!(!ids.contains(&"archived-child"));
}

#[test]
fn sessions_list_can_include_archived_sessions_when_requested() {
    let config = isolated_memory_config("sessions-list-include-archived");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "archived-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Archived".to_owned()),
        state: SessionState::Running,
    })
    .expect("create archived child");
    repo.finalize_session_terminal(
        "archived-child",
        FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({ "result": "ok" }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({ "child_session_id": "archived-child" }),
            frozen_result: None,
        },
    )
    .expect("finalize child");
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_archive".to_owned(),
            payload: json!({
                "session_id": "archived-child"
            }),
        },
        "root-session",
        &config,
    )
    .expect("archive child");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({
                "include_archived": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("sessions_list outcome");

    let archived = outcome.payload["sessions"]
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|item| item["session_id"] == "archived-child")
        .expect("archived session");
    let coordination = archived["subagent"]["coordination"]
        .as_array()
        .expect("coordination actions");
    let archive_actions = coordination
        .iter()
        .filter(|action| action["tool_name"] == "session_archive")
        .count();
    assert_eq!(outcome.payload["filters"]["include_archived"], true);
    assert_eq!(archived["archived"], true);
    assert!(archived["archived_at"].is_number());
    assert_eq!(archived["subagent"]["session_id"], "archived-child");
    assert_eq!(archive_actions, 0);
}

#[test]
fn sessions_list_overdue_only_uses_lifecycle_anchor_events() {
    let config = isolated_memory_config("sessions-list-overdue-only");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "overdue-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Overdue".to_owned()),
        state: SessionState::Running,
    })
    .expect("create overdue child");
    repo.create_session(NewSessionRecord {
        session_id: "fresh-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Fresh".to_owned()),
        state: SessionState::Running,
    })
    .expect("create fresh child");

    repo.append_event(NewSessionEvent {
        session_id: "overdue-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({ "timeout_seconds": 30 }),
    })
    .expect("append overdue queued");
    repo.append_event(NewSessionEvent {
        session_id: "overdue-child".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({ "timeout_seconds": 30 }),
    })
    .expect("append overdue started");
    overwrite_session_event_ts(
        &config,
        "overdue-child",
        "delegate_queued",
        super::current_unix_ts() - 120,
    );
    overwrite_session_event_ts(
        &config,
        "overdue-child",
        "delegate_started",
        super::current_unix_ts() - 90,
    );
    for step in 0..20 {
        repo.append_event(NewSessionEvent {
            session_id: "overdue-child".to_owned(),
            event_kind: format!("delegate_progress_{step}"),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "step": step }),
        })
        .expect("append overdue progress");
    }

    repo.append_event(NewSessionEvent {
        session_id: "fresh-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({ "timeout_seconds": 300 }),
    })
    .expect("append fresh queued");
    repo.append_event(NewSessionEvent {
        session_id: "fresh-child".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({ "timeout_seconds": 300 }),
    })
    .expect("append fresh started");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({
                "kind": "delegate_child",
                "overdue_only": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("sessions_list outcome");

    let sessions = outcome.payload["sessions"]
        .as_array()
        .expect("sessions array");
    let ids: Vec<&str> = sessions
        .iter()
        .filter_map(|item: &Value| item.get("session_id"))
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(ids, vec!["overdue-child"]);
    assert_eq!(outcome.payload["matched_count"], 1);
    assert_eq!(sessions[0]["delegate_lifecycle"]["mode"], "async");
    assert_eq!(sessions[0]["delegate_lifecycle"]["phase"], "running");
    assert_eq!(
        sessions[0]["delegate_lifecycle"]["staleness"]["state"],
        "overdue"
    );
    assert_eq!(
        sessions[0]["delegate_lifecycle"]["staleness"]["reference"],
        "started"
    );
}

#[test]
fn sessions_list_applies_offset_pagination() {
    let config = isolated_memory_config("sessions-list-offset");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "000-root".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    for session_id in ["001-child", "002-child", "003-child"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("000-root".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
    }

    overwrite_session_updated_at(&config, "000-root", 400);
    overwrite_session_updated_at(&config, "001-child", 300);
    overwrite_session_updated_at(&config, "002-child", 200);
    overwrite_session_updated_at(&config, "003-child", 100);

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({
                "limit": 2,
                "offset": 1
            }),
        },
        "000-root",
        &config,
    )
    .expect("sessions_list outcome");

    let sessions_value = &outcome.payload["sessions"];
    let sessions = sessions_value.as_array().expect("sessions array");
    let mut ids = Vec::new();
    for item in sessions {
        let session_id_value = item.get("session_id");
        let Some(session_id_value) = session_id_value else {
            continue;
        };
        let session_id = session_id_value.as_str();
        let Some(session_id) = session_id else {
            continue;
        };
        ids.push(session_id);
    }

    let filter_offset_value = &outcome.payload["filters"]["offset"];
    let matched_count_value = &outcome.payload["matched_count"];
    let returned_count_value = &outcome.payload["returned_count"];
    let has_more_value = &outcome.payload["has_more"];
    let filter_offset = filter_offset_value.as_u64().expect("filter offset");
    let matched_count = matched_count_value.as_u64().expect("matched count");
    let returned_count = returned_count_value.as_u64().expect("returned count");
    let has_more = has_more_value.as_bool().expect("has more");

    assert_eq!(ids, vec!["001-child", "002-child"]);
    assert_eq!(filter_offset, 1);
    assert_eq!(matched_count, 4);
    assert_eq!(returned_count, 2);
    assert!(has_more);
}

#[test]
fn sessions_list_includes_workflow_metadata_for_delegate_children() {
    let config = isolated_memory_config("sessions-list-workflow");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Research Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research release readiness",
            "task_scope": {
                "task_id": "task-release-readiness"
            },
            "task_session_id": "child-session",
            "label": "Research Child",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 120,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["read"],
                "workspace_root": "/tmp/loong/sessions-list-workflow/child-session",
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
        }),
    })
    .expect("append queued");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_list".to_owned(),
            payload: json!({
                "kind": "delegate_child"
            }),
        },
        "root-session",
        &config,
    )
    .expect("sessions_list outcome");

    let child = outcome.payload["sessions"]
        .as_array()
        .expect("sessions array")
        .iter()
        .find(|item| item["session_id"] == "child-session")
        .expect("child session");
    assert_eq!(child["workflow"]["workflow_id"], "root-session");
    assert_eq!(child["workflow"]["task"], "research release readiness");
    assert_eq!(child["workflow"]["phase"], "execute");
    assert_eq!(child["workflow"]["operation_kind"], "task");
    assert_eq!(child["workflow"]["operation_scope"], "task");
    assert_eq!(child["workflow"]["task_session_id"], "child-session");
    assert_eq!(child["workflow"]["lineage_root_session_id"], "root-session");
    assert_eq!(child["workflow"]["lineage_depth"], 1);
    assert_eq!(child["workflow"]["binding"]["session_id"], "child-session");
    assert_eq!(
        child["workflow"]["binding"]["task_id"],
        "task-release-readiness"
    );
    assert_eq!(
        child["workflow"]["binding"]["task_session_id"],
        "child-session"
    );
    assert_eq!(child["workflow"]["binding"]["mode"], "advisory_only");
    assert_eq!(
        child["workflow"]["binding"]["execution_surface"],
        "delegate.async"
    );
    assert_eq!(
        child["workflow"]["binding"]["worktree"]["worktree_id"],
        "child-session"
    );
    assert_eq!(child["subagent"]["session_id"], "child-session");
    assert_eq!(child["subagent_identity"]["nickname"], "Research Child");
    assert_eq!(
        child["subagent_contract"]["profile"]["role"],
        "orchestrator"
    );
    assert_eq!(
        child["subagent_contract"]["profile"]["control_scope"],
        "children"
    );
}

#[test]
fn sessions_history_returns_transcript_without_control_events() {
    let config = isolated_memory_config("sessions-history");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_completed".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({"status": "ok"}),
    })
    .expect("append event");

    append_session_turn_direct("child-session", "user", "hello", &config)
        .expect("append user turn");
    append_session_turn_direct("child-session", "assistant", "world", &config)
        .expect("append assistant turn");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "sessions_history".to_owned(),
            payload: json!({
                "session_id": "child-session",
                "limit": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("sessions_history outcome");

    let turns = outcome.payload["turns"].as_array().expect("turns array");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["role"], "user");
    assert_eq!(turns[0]["content"], "hello");
    assert_eq!(turns[1]["role"], "assistant");
    assert_eq!(turns[1]["content"], "world");
}

#[test]
fn session_fork_head_creates_named_head_visible_in_session_heads() {
    let config = isolated_memory_config("session-fork-head-tool");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    append_session_turn_direct("root-session", "user", "hello", &config).expect("append user turn");
    append_session_turn_direct("root-session", "assistant", "world", &config)
        .expect("append assistant turn");

    let fork_outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_fork_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "node_id": "session-turn:root-session:1",
                "head_name": "thread/alpha"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_fork_head outcome");
    assert_eq!(fork_outcome.payload["tool"], "session_fork_head");
    assert_eq!(fork_outcome.payload["head"]["head_name"], "thread/alpha");

    let heads_outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_heads".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_heads outcome");

    assert_eq!(heads_outcome.payload["head_count"], 2);
    let head_names = heads_outcome.payload["heads"]
        .as_array()
        .expect("heads array")
        .iter()
        .filter_map(|value| value["head_name"].as_str())
        .collect::<Vec<_>>();
    assert!(head_names.contains(&"active"));
    assert!(head_names.contains(&"thread/alpha"));
}

#[test]
fn session_create_checkpoint_creates_artifact_and_checkpoint_head() {
    let config = isolated_memory_config("session-create-checkpoint-tool");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    append_session_turn_direct("root-session", "user", "hello", &config).expect("append user turn");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_create_checkpoint".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "label": "draft-a"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_create_checkpoint outcome");

    assert_eq!(outcome.payload["tool"], "session_create_checkpoint");
    assert_eq!(outcome.payload["head"]["head_name"], "checkpoint/draft-a");
    assert_eq!(outcome.payload["head"]["head_mode"], "pinned");
    assert_eq!(outcome.payload["artifact"]["kind"], "checkpoint");

    let artifacts_outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_artifacts".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_artifacts outcome");

    assert_eq!(artifacts_outcome.payload["artifact_count"], 1);
    assert_eq!(
        artifacts_outcome.payload["artifacts"][0]["summary_text"],
        "draft-a"
    );
}

#[test]
fn session_pin_and_unpin_head_updates_explicit_mode() {
    let config = isolated_memory_config("session-pin-unpin-head-tool");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    append_session_turn_direct("root-session", "user", "hello", &config).expect("append user turn");

    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_fork_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "node_id": "session-turn:root-session:1",
                "head_name": "thread/alpha"
            }),
        },
        "root-session",
        &config,
    )
    .expect("fork head");

    let pin_outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_pin_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "head_name": "thread/alpha"
            }),
        },
        "root-session",
        &config,
    )
    .expect("pin head");

    assert_eq!(pin_outcome.payload["head"]["head_mode"], "pinned");

    let unpin_outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_unpin_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "head_name": "thread/alpha"
            }),
        },
        "root-session",
        &config,
    )
    .expect("unpin head");

    assert_eq!(unpin_outcome.payload["head"]["head_mode"], "live");
}

#[test]
fn session_create_branch_summary_captures_head_exclusive_range() {
    let config = isolated_memory_config("session-create-branch-summary-tool");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    append_session_turn_direct("root-session", "user", "hello", &config).expect("append user turn");
    append_session_turn_direct("root-session", "assistant", "world", &config)
        .expect("append assistant turn");
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_fork_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "node_id": "session-turn:root-session:2",
                "head_name": "mainline"
            }),
        },
        "root-session",
        &config,
    )
    .expect("fork mainline head");
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_fork_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "node_id": "session-turn:root-session:1",
                "head_name": "thread/alpha"
            }),
        },
        "root-session",
        &config,
    )
    .expect("fork thread head");
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_set_active_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "head_name": "thread/alpha"
            }),
        },
        "root-session",
        &config,
    )
    .expect("set branch active");
    append_session_turn_direct("root-session", "assistant", "branch reply", &config)
        .expect("append branch turn");
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_fork_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "node_id": "session-turn:root-session:3",
                "head_name": "thread/alpha-tip"
            }),
        },
        "root-session",
        &config,
    )
    .expect("fork branch tip head");
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_set_active_head".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "head_name": "mainline"
            }),
        },
        "root-session",
        &config,
    )
    .expect("restore mainline active");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_create_branch_summary".to_owned(),
            payload: json!({
                "session_id": "root-session",
                "head_name": "thread/alpha-tip",
                "summary_text": "alpha summary"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_create_branch_summary outcome");

    assert_eq!(outcome.payload["tool"], "session_create_branch_summary");
    assert_eq!(outcome.payload["artifact"]["kind"], "branch_summary");
    assert_eq!(outcome.payload["artifact"]["head_name"], "thread/alpha-tip");
    assert_eq!(
        outcome.payload["artifact"]["anchor_node_id"],
        "session-turn:root-session:1"
    );
    assert_eq!(
        outcome.payload["artifact"]["source_start_node_id"],
        "session-turn:root-session:3"
    );
    assert_eq!(
        outcome.payload["artifact"]["source_end_node_id"],
        "session-turn:root-session:3"
    );
    assert_eq!(outcome.payload["summary_text"], "alpha summary");
}

#[test]
fn session_status_returns_state_and_last_error() {
    let config = isolated_memory_config("session-status");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Failed,
    })
    .expect("create child");
    repo.update_session_state(
        "child-session",
        SessionState::Failed,
        Some("delegate_timeout".to_owned()),
    )
    .expect("update child status");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_failed".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({"error": "delegate_timeout"}),
    })
    .expect("append event");
    repo.upsert_terminal_outcome(
        "child-session",
        "error",
        json!({
            "child_session_id": "child-session",
            "error": "delegate_timeout",
            "duration_ms": 12
        }),
    )
    .expect("upsert terminal outcome");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(outcome.payload["session"]["session_id"], "child-session");
    assert_eq!(outcome.payload["session"]["state"], "failed");
    assert_eq!(outcome.payload["session"]["last_error"], "delegate_timeout");
    assert_eq!(outcome.payload["terminal_outcome_state"], "present");
    assert!(outcome.payload["terminal_outcome_missing_reason"].is_null());
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
    assert_eq!(
        outcome.payload["terminal_outcome"]["payload"]["error"],
        "delegate_timeout"
    );
    let recent_events = outcome.payload["recent_events"]
        .as_array()
        .expect("recent_events array");
    assert_eq!(recent_events.len(), 1);
    assert_eq!(recent_events[0]["event_kind"], "delegate_failed");
}

#[test]
fn session_status_includes_workflow_metadata_for_delegate_child() {
    let config = isolated_memory_config("session-status-workflow");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Continuity Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research continuity",
                "task_scope": {
                    "task_id": "task-continuity"
                },
                "task_session_id": "child-session",
                "label": "Continuity Child",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 90,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["read"],
                    "workspace_root": "/tmp/loong/session-status-workflow/child-session",
                    "kernel_bound": false,
                    "runtime_narrowing": {}
                },
                "runtime_self_continuity": {
                    "runtime_self": {
                        "standing_instructions": ["Stay concise."],
                        "tool_usage_policy": ["Prefer visible evidence."],
                        "soul_guidance": ["Keep continuity explicit."],
                        "identity_context": ["# Identity\n- Name: Child"],
                        "user_context": ["Operator prefers concise technical summaries."]
                    },
                    "resolved_identity": {
                        "source": "workspace_self",
                        "content": "# Identity\n- Name: Child"
                    },
                    "session_profile_projection": "## Session Profile\nOperator prefers concise technical summaries."
                }
            }),
        })
        .expect("append delegate_started");
    append_session_turn_direct("child-session", "user", "hello", &config)
        .expect("append user turn");
    append_session_turn_direct("child-session", "assistant", "world", &config)
        .expect("append assistant turn");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(outcome.payload["workflow"]["workflow_id"], "root-session");
    assert_eq!(outcome.payload["workflow"]["task"], "research continuity");
    assert_eq!(outcome.payload["workflow"]["phase"], "execute");
    assert_eq!(outcome.payload["workflow"]["operation_kind"], "task");
    assert_eq!(outcome.payload["workflow"]["operation_scope"], "task");
    assert_eq!(
        outcome.payload["workflow"]["task_session_id"],
        "child-session"
    );
    assert_eq!(
        outcome.payload["workflow"]["lineage_root_session_id"],
        "root-session"
    );
    assert_eq!(outcome.payload["workflow"]["lineage_depth"], 1);
    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["present"],
        true
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["session_id"],
        "child-session"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["task_id"],
        "task-continuity"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["task_session_id"],
        "child-session"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["mode"],
        "advisory_only"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["execution_surface"],
        "delegate.async"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["worktree"]["worktree_id"],
        "child-session"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["worktree"]["workspace_root"],
        "/tmp/loong/session-status-workflow/child-session"
    );
    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["resolved_identity_present"],
        true
    );
    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["session_profile_projection_present"],
        true
    );
    assert_eq!(outcome.payload["subagent"]["session_id"], "child-session");
    assert_eq!(
        outcome.payload["subagent_identity"]["nickname"],
        "Continuity Child"
    );
    assert_eq!(
        outcome.payload["subagent_contract"]["profile"]["role"],
        "orchestrator"
    );
    assert_eq!(
        outcome.payload["subagent_contract"]["profile"]["control_scope"],
        "children"
    );
    assert_eq!(outcome.payload["session"]["turn_count"], 2);
    assert!(outcome.payload["session"]["last_turn_at"].is_number());
}

#[test]
fn session_status_includes_runtime_self_continuity_from_refresh_events() {
    let config = isolated_memory_config("session-status-refresh-continuity");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: crate::runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
                .to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "source": "compaction",
                "runtime_self_continuity": {
                    "runtime_self": {
                        "standing_instructions": ["Stay concise."],
                        "tool_usage_policy": ["Prefer visible evidence."],
                        "soul_guidance": ["Keep continuity explicit."],
                        "identity_context": ["# Identity\n- Name: Root"],
                        "user_context": ["Operator prefers concise technical summaries."]
                    },
                    "resolved_identity": {
                        "source": "workspace_self",
                        "content": "# Identity\n- Name: Root"
                    },
                    "session_profile_projection": "## Session Profile\nOperator prefers concise technical summaries."
                }
            }),
        })
        .expect("append runtime self continuity refresh");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["present"],
        true
    );
    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["resolved_identity_present"],
        true
    );
    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["session_profile_projection_present"],
        true
    );
}

#[test]
fn session_status_includes_task_progress_from_latest_event() {
    let config = isolated_memory_config("session-status-task-progress");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "root-session".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: Some("Watch long-running task progress".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: vec![crate::task_progress::TaskActiveHandleRecord {
                    handle_kind: "conversation_turn".to_owned(),
                    handle_id: "root-session".to_owned(),
                    state: "waiting".to_owned(),
                    last_event_at: Some(123),
                    stop_condition: "terminal_reply".to_owned(),
                }],
                resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                    recommended_tool: "session_wait".to_owned(),
                    task_session_id: "root-session".to_owned(),
                    note: Some("Wait for durable task-progress transitions.".to_owned()),
                }),
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["task_id"],
        "root-session"
    );
    assert_eq!(outcome.payload["task_progress"]["task_id"], "root-session");
    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["status"],
        "waiting"
    );
    assert_eq!(outcome.payload["task_progress"]["status"], "waiting");
    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["intent_summary"],
        "Watch long-running task progress"
    );
    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["verification_state"],
        "pending"
    );
    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["active_handles"][0]["handle_kind"],
        "conversation_turn"
    );
    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["resume_recipe"]["recommended_tool"],
        "session_wait"
    );
    assert_eq!(
        outcome.payload["task_progress"]["resume_recipe"]["recommended_tool"],
        "session_wait"
    );
}

#[test]
fn session_status_keeps_runtime_self_continuity_after_more_than_64_newer_events() {
    let config = isolated_memory_config("session-status-refresh-continuity-stale-window");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: crate::runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
                .to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "source": "compaction",
                "runtime_self_continuity": {
                    "runtime_self": {
                        "standing_instructions": ["Stay concise."],
                        "tool_usage_policy": ["Prefer visible evidence."],
                        "soul_guidance": ["Keep continuity explicit."],
                        "identity_context": ["# Identity\n- Name: Root"],
                        "user_context": ["Operator prefers concise technical summaries."]
                    },
                    "resolved_identity": {
                        "source": "workspace_self",
                        "content": "# Identity\n- Name: Root"
                    },
                    "session_profile_projection": "## Session Profile\nOperator prefers concise technical summaries."
                }
            }),
        })
        .expect("append runtime self continuity refresh");

    for index in 0..70 {
        let event_kind = format!("noise_event_{index}");
        let payload = json!({ "index": index });
        let event = NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind,
            actor_session_id: Some("root-session".to_owned()),
            payload_json: payload,
        };
        repo.append_event(event).expect("append noise event");
    }

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["present"],
        true
    );
    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["resolved_identity_present"],
        true
    );
    assert_eq!(
        outcome.payload["workflow"]["runtime_self_continuity"]["session_profile_projection_present"],
        true
    );
}

#[test]
fn session_status_keeps_task_progress_outside_recent_event_window() {
    let config = isolated_memory_config("session-status-task-progress-stale-window");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "root-session".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: Some("Keep durable task progress visible".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                    recommended_tool: "session_status".to_owned(),
                    task_session_id: "root-session".to_owned(),
                    note: Some(
                        "Use session_status even after the recent window moves on.".to_owned(),
                    ),
                }),
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");

    for index in 0..80 {
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: format!("noise_event_{index}"),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "index": index }),
        })
        .expect("append noise event");
    }

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["status"],
        "waiting"
    );
    assert_eq!(outcome.payload["task_progress"]["status"], "waiting");
    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["intent_summary"],
        "Keep durable task progress visible"
    );
    assert_eq!(
        outcome.payload["workflow"]["task_progress"]["resume_recipe"]["recommended_tool"],
        "session_status"
    );
    assert_eq!(
        outcome.payload["task_progress"]["resume_recipe"]["recommended_tool"],
        "session_status"
    );
}

#[test]
fn task_status_resolves_canonical_task_id_and_exposes_owner_session_id() {
    let config = isolated_memory_config("task-status-aliases");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "task-owner".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Task Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: Some("Task tool status".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                    recommended_tool: "task_wait".to_owned(),
                    task_session_id: "task-owner".to_owned(),
                    note: Some("Wait on the task surface.".to_owned()),
                }),
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_status".to_owned(),
            payload: json!({
                "task_id": "task-root"
            }),
        },
        "task-owner",
        &config,
    )
    .expect("task_status outcome");

    assert_eq!(outcome.payload["tool"], "task_status");
    assert_eq!(outcome.payload["task_id"], "task-root");
    assert_eq!(outcome.payload["owner_session_id"], "task-owner");
    assert_eq!(outcome.payload["task_session_id"], "task-owner");
    assert_eq!(outcome.payload["session"]["session_id"], "task-owner");
    assert_eq!(outcome.payload["task_session_count"], 1);
    assert_eq!(
        outcome.payload["task_sessions"][0]["task_session_id"],
        "task-owner"
    );
    assert_eq!(
        outcome.payload["task_sessions"][0]["is_current_owner"],
        true
    );
    assert_eq!(outcome.payload["task_state"], "waiting");
    assert_eq!(outcome.payload["task_is_stable"], true);
    assert_eq!(outcome.payload["task_progress"]["status"], "waiting");
    assert_eq!(
        outcome.payload["task_progress"]["resume_recipe"]["recommended_tool"],
        "task_wait"
    );
}

#[test]
fn task_status_resolves_binding_only_task_identity_before_task_progress_exists() {
    let config = isolated_memory_config("task-status-binding-only");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "binding only task",
            "task_scope": {
                "task_id": "task-bind-only"
            },
            "task_session_id": "child-session",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 90,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["read"],
                "workspace_root": "/tmp/loong/task-status-binding-only/child-session",
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
        }),
    })
    .expect("append delegate_queued");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_status".to_owned(),
            payload: json!({
                "task_id": "task-bind-only"
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_status outcome");

    assert_eq!(outcome.payload["tool"], "task_status");
    assert_eq!(outcome.payload["task_id"], "task-bind-only");
    assert_eq!(outcome.payload["owner_session_id"], "child-session");
    assert_eq!(outcome.payload["task_session_id"], "child-session");
    assert_eq!(outcome.payload["task_session_count"], 1);
    assert_eq!(
        outcome.payload["task_sessions"][0]["task_session_id"],
        "child-session"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["task_id"],
        "task-bind-only"
    );
    assert_eq!(
        outcome.payload["workflow"]["binding"]["task_session_id"],
        "child-session"
    );
    assert_eq!(outcome.payload["task_state"], "ready");
    assert!(outcome.payload["task_progress"].is_null());
}

#[test]
fn task_history_reads_history_by_canonical_task_id() {
    let config = isolated_memory_config("task-history");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "task-owner".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Task Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Active,
                intent_summary: Some("Task history".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::NotStarted),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");
    append_session_turn_direct("task-owner", "user", "hello", &config).expect("append user turn");
    append_session_turn_direct("task-owner", "assistant", "world", &config)
        .expect("append assistant turn");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_history".to_owned(),
            payload: json!({
                "task_id": "task-root",
                "limit": 10
            }),
        },
        "task-owner",
        &config,
    )
    .expect("task_history outcome");

    assert_eq!(outcome.payload["tool"], "task_history");
    assert_eq!(outcome.payload["task_id"], "task-root");
    assert_eq!(outcome.payload["owner_session_id"], "task-owner");
    assert_eq!(outcome.payload["task_session_id"], "task-owner");
    assert_eq!(outcome.payload["lineage_session_count"], 1);
    assert_eq!(
        outcome.payload["task_sessions"][0]["task_session_id"],
        "task-owner"
    );
    assert_eq!(
        outcome.payload["task_sessions"][0]["session_state"],
        "running"
    );
    assert_eq!(
        outcome.payload["task_sessions"][0]["is_current_owner"],
        true
    );
    assert_eq!(outcome.payload["turns"][0]["content"], "hello");
    assert_eq!(outcome.payload["turns"][1]["content"], "world");
    assert_eq!(outcome.payload["turns"][0]["task_session_id"], "task-owner");
    assert_eq!(outcome.payload["turns"][1]["task_session_id"], "task-owner");
    assert_eq!(outcome.payload["turns"][0]["is_current_owner"], true);
    assert_eq!(
        outcome.payload["task_events"][0]["event_kind"],
        crate::task_progress::TASK_PROGRESS_EVENT_KIND
    );
    assert_eq!(
        outcome.payload["task_events"][0]["task_session_id"],
        "task-owner"
    );
    assert_eq!(outcome.payload["task_events"][0]["is_current_owner"], true);
}

#[test]
fn task_history_aggregates_visible_task_lineage_across_owner_sessions() {
    let config = isolated_memory_config("task-history-lineage");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    for session_id in ["owner-old", "owner-new"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
    }
    repo.append_event(NewSessionEvent {
        session_id: "owner-old".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("owner-old".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Active,
                intent_summary: Some("Old owner".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::NotStarted),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 10,
            },
        ),
    })
    .expect("append old task progress");
    repo.append_event(NewSessionEvent {
        session_id: "owner-new".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "task lineage handoff",
            "task_scope": {
                "task_id": "task-root"
            },
            "task_session_id": "owner-new",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 90,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["read"],
                "workspace_root": "/tmp/loong/task-history-lineage/owner-new",
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
        }),
    })
    .expect("append delegate queued");
    repo.append_event(NewSessionEvent {
        session_id: "owner-new".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("owner-new".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Completed,
                intent_summary: Some("New owner".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Passed),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 20,
            },
        ),
    })
    .expect("append new task progress");
    append_session_turn_direct("owner-old", "user", "old owner turn", &config)
        .expect("append old owner turn");
    append_session_turn_direct("owner-new", "assistant", "new owner turn", &config)
        .expect("append new owner turn");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_history".to_owned(),
            payload: json!({
                "task_id": "task-root",
                "limit": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_history outcome");

    assert_eq!(outcome.payload["tool"], "task_history");
    assert_eq!(outcome.payload["task_id"], "task-root");
    assert_eq!(outcome.payload["owner_session_id"], "owner-new");
    assert_eq!(outcome.payload["task_session_id"], "owner-new");
    assert_eq!(outcome.payload["lineage_session_count"], 2);
    let task_sessions = outcome.payload["task_sessions"]
        .as_array()
        .expect("task sessions");
    assert_eq!(task_sessions.len(), 2);
    assert_eq!(task_sessions[0]["task_session_id"], "owner-old");
    assert_eq!(task_sessions[1]["task_session_id"], "owner-new");
    assert_eq!(task_sessions[0]["is_current_owner"], false);
    assert_eq!(task_sessions[1]["is_current_owner"], true);

    let turns = outcome.payload["turns"].as_array().expect("turns");
    let task_turn_sessions = turns
        .iter()
        .map(|turn| {
            turn.get("task_session_id")
                .and_then(Value::as_str)
                .expect("task_session_id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(task_turn_sessions.contains(&"owner-old".to_owned()));
    assert!(task_turn_sessions.contains(&"owner-new".to_owned()));

    let task_events = outcome.payload["task_events"]
        .as_array()
        .expect("task events");
    let event_kinds = task_events
        .iter()
        .map(|event| {
            event
                .get("event_kind")
                .and_then(Value::as_str)
                .expect("event kind")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(event_kinds.contains(&"delegate_queued".to_owned()));
    assert!(event_kinds.contains(&crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned()));
}

#[test]
fn task_events_supports_lineage_aggregation_and_cursor_follow_up() {
    let config = isolated_memory_config("task-events-lineage");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    for session_id in ["owner-old", "owner-new"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
    }
    repo.append_event(NewSessionEvent {
        session_id: "owner-old".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "task events handoff",
            "task_scope": {
                "task_id": "task-root"
            },
            "task_session_id": "owner-old"
        }),
    })
    .expect("append delegate queued");
    repo.append_event(NewSessionEvent {
        session_id: "owner-new".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("owner-new".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Completed,
                intent_summary: Some("Completed by new owner".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Passed),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 20,
            },
        ),
    })
    .expect("append completed task progress");

    let first = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_events".to_owned(),
            payload: json!({
                "task_id": "task-root",
                "after_id": 0,
                "limit": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_events outcome");

    assert_eq!(first.payload["tool"], "task_events");
    assert_eq!(first.payload["task_id"], "task-root");
    assert_eq!(first.payload["owner_session_id"], "owner-new");
    assert_eq!(first.payload["task_session_id"], "owner-new");
    assert_eq!(first.payload["task_session_count"], 2);
    let task_sessions = first.payload["task_sessions"]
        .as_array()
        .expect("task sessions");
    assert_eq!(task_sessions.len(), 2);
    let task_session_ids = task_sessions
        .iter()
        .map(|task_session| {
            task_session
                .get("task_session_id")
                .and_then(Value::as_str)
                .expect("task_session_id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(task_session_ids.contains(&"owner-old".to_owned()));
    assert!(task_session_ids.contains(&"owner-new".to_owned()));
    let current_owner_records = task_sessions
        .iter()
        .filter(|task_session| {
            task_session
                .get("is_current_owner")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    assert_eq!(current_owner_records, 1);
    let events = first.payload["events"].as_array().expect("events");
    assert_eq!(events.len(), 2);
    let next_after_id = first.payload["next_after_id"]
        .as_i64()
        .expect("next_after_id");
    assert!(next_after_id > 0);

    let second = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_events".to_owned(),
            payload: json!({
                "task_id": "task-root",
                "after_id": next_after_id,
                "limit": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_events follow-up outcome");

    assert_eq!(second.payload["events"], json!([]));
    assert_eq!(second.payload["next_after_id"], next_after_id);
    assert_eq!(second.payload["task_session_count"], 2);
}

#[test]
fn task_recover_uses_canonical_task_id_to_recover_owner_session() {
    let config = isolated_memory_config("task-recover-canonical");
    let repo = SessionRepository::new(&config).expect("repository");
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
    .expect("create owner");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task_scope": {
                "task_id": "task-root",
            },
            "task_session_id": "task-owner",
            "timeout_seconds": 1
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "background_task_host".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Blocked,
                intent_summary: Some("Recover overdue task".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");
    repo.update_session_state("task-owner", SessionState::Ready, None)
        .expect("keep ready state");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_recover".to_owned(),
            payload: json!({
                "task_id": "task-root",
                "dry_run": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_recover outcome");

    assert_eq!(outcome.payload["task_id"], "task-root");
    assert_eq!(outcome.payload["owner_session_id"], "task-owner");
}

#[test]
fn task_cancel_uses_canonical_task_id_to_cancel_owner_session() {
    let config = isolated_memory_config("task-cancel-canonical");
    let repo = SessionRepository::new(&config).expect("repository");
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
    .expect("create owner");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task_scope": {
                "task_id": "task-root",
            },
            "task_session_id": "task-owner",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "background_task_host".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Active,
                intent_summary: Some("Cancel queued task".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::NotStarted),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_cancel".to_owned(),
            payload: json!({
                "task_id": "task-root",
                "dry_run": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_cancel outcome");

    assert_eq!(outcome.payload["task_id"], "task-root");
    assert_eq!(outcome.payload["owner_session_id"], "task-owner");
}

#[test]
fn task_status_batch_reports_task_ids_without_session_id_aliases() {
    let config = isolated_memory_config("task-status-batch");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");

    for (session_id, task_id, updated_at) in [("owner-a", "task-a", 10), ("owner-b", "task-b", 20)]
    {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: task_id.to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some(format!("Status for {task_id}")),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at,
                },
            ),
        })
        .expect("append task progress event");
    }

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_status".to_owned(),
            payload: json!({
                "task_ids": ["task-a", "task-b"]
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_status batch outcome");

    let results = outcome.payload["results"]
        .as_array()
        .expect("batch results");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["task_id"], "task-a");
    assert_eq!(results[0]["owner_session_id"], "owner-a");
    assert_eq!(results[0]["task_session_id"], "owner-a");
    assert_eq!(results[0]["task_session_count"], 1);
    assert_eq!(results[0]["task_sessions"][0]["task_session_id"], "owner-a");
    assert_eq!(results[0]["task_state"], "waiting");
    assert_eq!(results[0]["task_is_stable"], true);
    assert!(results[0].get("session_id").is_none());
    assert_eq!(results[1]["task_id"], "task-b");
    assert_eq!(results[1]["owner_session_id"], "owner-b");
    assert_eq!(results[1]["task_session_id"], "owner-b");
    assert_eq!(results[1]["task_session_count"], 1);
    assert_eq!(results[1]["task_sessions"][0]["task_session_id"], "owner-b");
    assert!(results[1].get("session_id").is_none());
}

#[test]
fn tasks_list_returns_visible_task_progress_records() {
    let config = isolated_memory_config("tasks-list");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    for session_id in ["task-a", "task-b", "no-task"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
    }
    for (session_id, status) in [
        ("task-a", crate::task_progress::TaskProgressStatus::Waiting),
        (
            "task-b",
            crate::task_progress::TaskProgressStatus::Completed,
        ),
    ] {
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: session_id.to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status,
                    intent_summary: Some(format!("summary-{session_id}")),
                    verification_state: None,
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");
    }

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_list".to_owned(),
            payload: json!({}),
        },
        "root-session",
        &config,
    )
    .expect("tasks_list outcome");

    assert_eq!(outcome.payload["tool"], "tasks_list");
    assert_eq!(outcome.payload["matched_count"], 2);
    assert_eq!(
        outcome.payload["tasks"]
            .as_array()
            .expect("tasks array")
            .len(),
        2
    );
}

#[test]
fn tasks_list_filters_stable_only_and_task_state() {
    let config = isolated_memory_config("tasks-list-filters-stable");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    for (session_id, status) in [
        (
            "task-active",
            crate::task_progress::TaskProgressStatus::Active,
        ),
        (
            "task-waiting",
            crate::task_progress::TaskProgressStatus::Waiting,
        ),
        (
            "task-completed",
            crate::task_progress::TaskProgressStatus::Completed,
        ),
    ] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: session_id.to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status,
                    intent_summary: Some(session_id.to_owned()),
                    verification_state: None,
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");
    }

    let stable_only = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_list".to_owned(),
            payload: json!({
                "stable_only": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("stable tasks_list outcome");
    assert_eq!(stable_only.payload["matched_count"], 2);

    let waiting_only = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_list".to_owned(),
            payload: json!({
                "task_state": "waiting"
            }),
        },
        "root-session",
        &config,
    )
    .expect("waiting tasks_list outcome");
    assert_eq!(waiting_only.payload["matched_count"], 1);
    assert_eq!(waiting_only.payload["tasks"][0]["task_id"], "task-waiting");
}

#[test]
fn tasks_search_matches_summary_and_state_filters() {
    let config = isolated_memory_config("tasks-search");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    for (session_id, summary, status) in [
        (
            "task-alpha",
            "refresh approval queue",
            crate::task_progress::TaskProgressStatus::Waiting,
        ),
        (
            "task-beta",
            "rebuild search index",
            crate::task_progress::TaskProgressStatus::Completed,
        ),
    ] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: session_id.to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status,
                    intent_summary: Some(summary.to_owned()),
                    verification_state: None,
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 1,
                },
            ),
        })
        .expect("append task progress event");
    }

    let summary_match = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_search".to_owned(),
            payload: json!({
                "query": "approval",
                "max_results": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("tasks_search outcome");

    assert_eq!(summary_match.payload["tool"], "tasks_search");
    assert_eq!(summary_match.payload["matched_count"], 1);
    assert_eq!(summary_match.payload["tasks"][0]["task_id"], "task-alpha");

    let state_match = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_search".to_owned(),
            payload: json!({
                "query": "task",
                "task_state": "completed",
                "max_results": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("tasks_search filtered outcome");

    assert_eq!(state_match.payload["matched_count"], 1);
    assert_eq!(state_match.payload["tasks"][0]["task_id"], "task-beta");
}

#[test]
fn task_surfaces_deduplicate_shared_canonical_task_ids_to_latest_owner_session() {
    let config = isolated_memory_config("task-deduplicate-latest-owner");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");

    for session_id in ["owner-old", "owner-new"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
    }

    for (session_id, summary, updated_at) in [
        ("owner-old", "legacy owner", 10),
        ("owner-new", "latest owner", 20),
    ] {
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-shared".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some(summary.to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at,
                },
            ),
        })
        .expect("append task progress event");
    }

    let task_status = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "task_status".to_owned(),
            payload: json!({
                "task_id": "task-shared"
            }),
        },
        "root-session",
        &config,
    )
    .expect("task_status outcome");
    assert_eq!(task_status.payload["task_id"], "task-shared");
    assert_eq!(task_status.payload["owner_session_id"], "owner-new");
    assert_eq!(task_status.payload["task_session_id"], "owner-new");
    assert_eq!(task_status.payload["task_session_count"], 2);
    assert_eq!(
        task_status.payload["task_sessions"][0]["task_session_id"],
        "owner-old"
    );
    assert_eq!(
        task_status.payload["task_sessions"][0]["is_current_owner"],
        false
    );
    assert_eq!(
        task_status.payload["task_sessions"][1]["task_session_id"],
        "owner-new"
    );
    assert_eq!(
        task_status.payload["task_sessions"][1]["is_current_owner"],
        true
    );
    assert_eq!(
        task_status.payload["task_progress"]["intent_summary"],
        "latest owner"
    );

    let tasks_list = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_list".to_owned(),
            payload: json!({}),
        },
        "root-session",
        &config,
    )
    .expect("tasks_list outcome");
    assert_eq!(tasks_list.payload["matched_count"], 1);
    assert_eq!(tasks_list.payload["tasks"][0]["task_id"], "task-shared");
    assert_eq!(
        tasks_list.payload["tasks"][0]["owner_session_id"],
        "owner-new"
    );
    assert_eq!(
        tasks_list.payload["tasks"][0]["intent_summary"],
        "latest owner"
    );

    let tasks_search = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_search".to_owned(),
            payload: json!({
                "query": "task-shared",
                "max_results": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("tasks_search outcome");
    assert_eq!(tasks_search.payload["matched_count"], 1);
    assert_eq!(tasks_search.payload["tasks"][0]["task_id"], "task-shared");
    assert_eq!(
        tasks_search.payload["tasks"][0]["owner_session_id"],
        "owner-new"
    );
}

#[test]
fn load_session_workflow_record_propagates_unexpected_lineage_lookup_failures() {
    let config = isolated_memory_config("session-workflow-lineage-errors");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");

    let session = repo
        .load_session_summary_with_legacy_fallback("root-session")
        .expect("load session summary")
        .expect("root session summary");

    let db_path = config
        .sqlite_path
        .as_ref()
        .expect("sqlite path for session tools test");
    let conn = rusqlite::Connection::open(db_path).expect("open sqlite db");
    conn.execute("DROP TABLE sessions", [])
        .expect("drop sessions table");

    let error = super::load_session_workflow_record(&repo, &session, None)
        .expect_err("unexpected lineage lookup failures should surface");

    assert!(
        error.contains("no such table: sessions"),
        "expected sqlite lineage lookup failure, got: {error}"
    );
}

#[test]
fn optional_lineage_lookup_only_degrades_expected_gap_errors() {
    let broken = super::optional_lineage_lookup::<usize>(Err(
        "session_lineage_broken: missing parent row for `child-session`".to_owned(),
    ))
    .expect("broken lineage should degrade to missing");
    assert_eq!(broken, None);

    let cycle = super::optional_lineage_lookup::<usize>(Err(
        "session_lineage_cycle_detected: `child-session` reappeared".to_owned(),
    ))
    .expect("cycle lineage should degrade to missing");
    assert_eq!(cycle, None);

    let error = super::optional_lineage_lookup::<usize>(Err(
        "query sessions failed: database is locked".to_owned(),
    ))
    .expect_err("unexpected lineage lookup failures should not be swallowed");
    assert_eq!(error, "query sessions failed: database is locked");
}

#[test]
fn session_tool_policy_tools_round_trip_and_clear_policy() {
    let config = isolated_memory_config("session-tool-policy-tools");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let set = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_tool_policy_set".to_owned(),
            payload: json!({
                "tool_ids": ["read", "session_status"],
                "runtime_narrowing": {
                    "browser": {
                        "max_sessions": 2,
                    },
                    "web_fetch": {
                        "allowed_domains": ["docs.example.com"],
                        "blocked_domains": ["deny.example.com"],
                        "allow_private_hosts": false,
                    }
                }
            }),
        },
        "root-session",
        &config,
    )
    .expect("set session tool policy");

    assert_eq!(set.payload["action"], "created");
    assert_eq!(set.payload["policy"]["has_policy"], true);
    assert_eq!(
        set.payload["policy"]["requested_tool_ids"],
        json!(["read", "session_status"])
    );
    assert_eq!(
        set.payload["policy"]["visible_requested_tool_ids"],
        json!(["read", "session_status"])
    );
    assert_eq!(
        set.payload["policy"]["effective_tool_ids"],
        json!(["read", "session_status"])
    );
    assert_eq!(
        set.payload["policy"]["visible_effective_tool_ids"],
        json!(["read", "session_status"])
    );
    assert_eq!(
        set.payload["policy"]["requested_runtime_narrowing"]["browser"]["max_sessions"],
        2
    );
    assert_eq!(
        set.payload["policy"]["effective_runtime_narrowing"]["web_fetch"]["allowed_domains"],
        json!(["docs.example.com"])
    );

    let status = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_tool_policy_status".to_owned(),
            payload: json!({}),
        },
        "root-session",
        &config,
    )
    .expect("session tool policy status");

    assert_eq!(status.payload["policy"]["has_policy"], true);
    assert_eq!(
        status.payload["policy"]["requested_tool_ids"],
        json!(["read", "session_status"])
    );
    assert_eq!(
        status.payload["policy"]["visible_requested_tool_ids"],
        json!(["read", "session_status"])
    );
    assert_eq!(
        status.payload["policy"]["requested_runtime_narrowing"]["web_fetch"]["blocked_domains"],
        json!(["deny.example.com"])
    );

    let clear = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_tool_policy_clear".to_owned(),
            payload: json!({}),
        },
        "root-session",
        &config,
    )
    .expect("clear session tool policy");

    assert_eq!(clear.payload["action"], "cleared");
    assert_eq!(clear.payload["policy"]["has_policy"], false);
    assert_eq!(clear.payload["policy"]["requested_tool_ids"], json!([]));
    assert!(
        clear.payload["policy"]["effective_tool_ids"]
            .as_array()
            .expect("effective tool ids")
            .iter()
            .any(|value| value == "session_status")
    );
}

#[test]
fn session_tool_policy_set_bootstraps_current_root_session_when_missing() {
    let config = isolated_memory_config("session-tool-policy-bootstrap");
    let repo = SessionRepository::new(&config).expect("repository");

    let set = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_tool_policy_set".to_owned(),
            payload: json!({
                "tool_ids": ["read", "session_status"]
            }),
        },
        "fresh-root-session",
        &config,
    )
    .expect("set session tool policy");

    assert_eq!(set.payload["action"], "created");
    let session = repo
        .load_session("fresh-root-session")
        .expect("load bootstrapped root session")
        .expect("bootstrapped root session");
    assert_eq!(session.kind, SessionKind::Root);
    assert_eq!(session.state, SessionState::Ready);

    let policy = repo
        .load_session_tool_policy("fresh-root-session")
        .expect("load bootstrapped session tool policy")
        .expect("bootstrapped session tool policy");
    assert_eq!(
        policy.requested_tool_ids,
        vec!["read".to_owned(), "session_status".to_owned()]
    );
}

#[test]
fn session_tool_policy_set_rejects_legacy_discovery_wrappers() {
    let config = isolated_memory_config("session-tool-policy-legacy-wrapper");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let error = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_tool_policy_set".to_owned(),
            payload: json!({
                "tool_ids": ["tool.search", "session_status"]
            }),
        },
        "root-session",
        &config,
    )
    .expect_err("legacy discovery wrappers should be rejected");

    assert!(error.contains("legacy discovery wrapper"), "error: {error}");
}

#[cfg(feature = "feishu-integration")]
#[test]
fn session_tool_policy_root_tool_view_includes_runtime_discovered_feishu_tools() {
    let runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
        feishu: Some(crate::tools::runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some(loong_contracts::SecretRef::Inline(
                    "test-feishu-app-id".to_owned(),
                )),
                app_secret: Some(loong_contracts::SecretRef::Inline(
                    "test-feishu-app-secret".to_owned(),
                )),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        }),
        ..crate::tools::runtime_config::ToolRuntimeConfig::default()
    };
    let tool_config = ToolConfig::default();
    let tool_view = super::session_tool_policy_root_tool_view(&tool_config, &runtime_config);

    assert!(tool_view.contains("feishu.whoami"));
    assert!(tool_view.contains("feishu.messages.send"));
}

#[test]
fn session_status_reports_missing_terminal_outcome_for_recovered_failed_session() {
    let config = isolated_memory_config("session-status-recovered-failed");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Failed,
    })
    .expect("create child");
    repo.update_session_state(
        "child-session",
        SessionState::Failed,
        Some("opaque_recovery_failure".to_owned()),
    )
    .expect("update child status");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_recovery_applied".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "recovery_kind": "terminal_finalize_persist_failed",
            "recovered_state": "failed",
            "recovery_error": "delegate_terminal_finalize_failed: database busy",
            "attempted_terminal_event_kind": "delegate_completed",
            "attempted_outcome_status": "ok"
        }),
    })
    .expect("append event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(outcome.payload["session"]["session_id"], "child-session");
    assert_eq!(outcome.payload["session"]["state"], "failed");
    assert_eq!(
        outcome.payload["session"]["last_error"],
        "opaque_recovery_failure"
    );
    assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
    assert_eq!(
        outcome.payload["terminal_outcome_missing_reason"],
        "terminal_finalize_persist_failed"
    );
    assert_eq!(
        outcome.payload["recovery"]["kind"],
        "terminal_finalize_persist_failed"
    );
    assert_eq!(
        outcome.payload["recovery"]["event_kind"],
        "delegate_recovery_applied"
    );
    assert_eq!(
        outcome.payload["recovery"]["recovery_error"],
        "delegate_terminal_finalize_failed: database busy"
    );
    assert_eq!(
        outcome.payload["recovery"]["attempted_terminal_event_kind"],
        "delegate_completed"
    );
    assert_eq!(outcome.payload["recovery"]["source"], "event");
    assert!(outcome.payload["terminal_outcome"].is_null());
}

#[test]
fn session_status_synthesizes_recovery_from_last_error_when_event_missing() {
    let config = isolated_memory_config("session-status-recovery-fallback");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Failed,
    })
    .expect("create child");
    repo.update_session_state(
        "child-session",
        SessionState::Failed,
        Some("delegate_terminal_finalize_failed: database busy".to_owned()),
    )
    .expect("update child status");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
    assert_eq!(
        outcome.payload["terminal_outcome_missing_reason"],
        "terminal_finalize_persist_failed"
    );
    assert_eq!(
        outcome.payload["recovery"]["kind"],
        "terminal_finalize_persist_failed"
    );
    assert_eq!(outcome.payload["recovery"]["source"], "last_error");
    assert_eq!(
        outcome.payload["recovery"]["recovery_error"],
        "delegate_terminal_finalize_failed: database busy"
    );
    assert!(outcome.payload["recovery"]["event_kind"].is_null());
}

#[test]
fn session_status_synthesizes_unknown_recovery_when_metadata_missing() {
    let config = isolated_memory_config("session-status-recovery-unknown");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Failed,
    })
    .expect("create child");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
    assert_eq!(
        outcome.payload["terminal_outcome_missing_reason"],
        "unknown"
    );
    assert_eq!(outcome.payload["recovery"]["kind"], "unknown");
    assert_eq!(outcome.payload["recovery"]["source"], "none");
    assert!(outcome.payload["recovery"]["recovery_error"].is_null());
    assert!(outcome.payload["recovery"]["event_kind"].is_null());
}

#[test]
fn session_status_surfaces_latest_provider_failover_diagnostics() {
    let config = isolated_memory_config("session-status-provider-failover");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: "trust_provider_failover".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "source": "provider_runtime",
            "binding": "kernel",
            "provider_id": "openai",
            "provider_failover": {
                "reason": "rate_limited",
                "stage": "status_failure",
                "model": "gpt-4o",
                "attempt": 2,
                "max_attempts": 3,
                "status_code": 429,
                "request_id": "req-123"
            }
        }),
    })
    .expect("append provider failover event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["diagnostics"]["latest_provider_failover"]["provider_id"],
        "openai"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["latest_provider_failover"]["reason"],
        "rate_limited"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["latest_provider_failover"]["model"],
        "gpt-4o"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["latest_provider_failover"]["status_code"],
        429
    );
    let attention_hints = outcome.payload["diagnostics"]["attention_hints"]
        .as_array()
        .expect("attention_hints array");
    assert!(
        attention_hints.iter().any(|hint| {
            hint.as_str().is_some_and(|hint| {
                hint.contains("provider_failover_present")
                    && hint.contains("reason=rate_limited")
                    && hint.contains("request_id=req-123")
            })
        }),
        "expected provider failover attention hint, got: {attention_hints:?}"
    );
}

#[test]
fn session_status_recommends_session_recover_for_overdue_async_child() {
    let config = isolated_memory_config("session-status-recover-recommendation");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    overwrite_session_event_ts(
        &config,
        "child-session",
        "delegate_queued",
        super::current_unix_ts() - 90,
    );

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["tool_name"],
        "session_recover"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["kind"],
        "queued_async_overdue_marked_failed"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["source"],
        "session_recover_plan"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["requires_mutation"],
        true
    );
}

#[test]
fn session_status_recommends_resume_recipe_when_recover_plan_is_unavailable() {
    let config = isolated_memory_config("session-status-resume-recipe-recommendation");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "root-session".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: Some("Wait for the durable task to settle".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                    recommended_tool: "session_wait".to_owned(),
                    task_session_id: "root-session".to_owned(),
                    note: Some("Wait for the terminal transition.".to_owned()),
                }),
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "root-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["tool_name"],
        "session_wait"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["kind"],
        "follow_resume_recipe"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["source"],
        "task_progress_resume_recipe"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["task_status"],
        "waiting"
    );
    assert_eq!(
        outcome.payload["diagnostics"]["recommended_action"]["requires_mutation"],
        false
    );
}

#[test]
fn session_recover_marks_overdue_queued_async_child_failed() {
    let config = isolated_memory_config("session-recover-overdue");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "label": "Child",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    overwrite_session_event_ts(
        &config,
        "child-session",
        "delegate_queued",
        super::current_unix_ts() - 90,
    );

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_recover".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_recover outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["session"]["state"], "failed");
    assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "failed");
    assert!(outcome.payload["delegate_lifecycle"]["staleness"].is_null());
    assert_eq!(outcome.payload["terminal_outcome_state"], "present");
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
    let frozen_error_code =
        outcome.payload["terminal_outcome"]["frozen_result"]["content"]["error"]["code"]
            .as_str()
            .expect("queued frozen error code");
    assert!(
        frozen_error_code.starts_with("delegate_async_queued_overdue_marked_failed:"),
        "unexpected queued frozen error code: {frozen_error_code}"
    );
    assert_eq!(
        outcome.payload["recovery_action"]["kind"],
        "queued_async_overdue_marked_failed"
    );
    assert_eq!(
        outcome.payload["recent_events"]
            .as_array()
            .expect("recent events array")
            .last()
            .expect("latest recent event")["event_kind"],
        "delegate_recovery_applied"
    );
}

#[test]
fn session_recover_rejects_fresh_queued_child() {
    let config = isolated_memory_config("session-recover-fresh");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append queued event");

    let error = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_recover".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect_err("fresh queued child should be rejected");

    assert!(
        error.contains("session_recover_not_recoverable"),
        "expected recoverability rejection, got: {error}"
    );
}

#[test]
fn session_recover_marks_overdue_running_async_child_failed() {
    let config = isolated_memory_config("session-recover-running-overdue");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 30
        }),
    })
    .expect("append started event");
    overwrite_session_event_ts(
        &config,
        "child-session",
        "delegate_queued",
        super::current_unix_ts() - 120,
    );
    overwrite_session_event_ts(
        &config,
        "child-session",
        "delegate_started",
        super::current_unix_ts() - 90,
    );

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_recover".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_recover outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["session"]["state"], "failed");
    assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "failed");
    assert!(outcome.payload["delegate_lifecycle"]["staleness"].is_null());
    assert_eq!(outcome.payload["terminal_outcome_state"], "present");
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
    let frozen_error_code =
        outcome.payload["terminal_outcome"]["frozen_result"]["content"]["error"]["code"]
            .as_str()
            .expect("running frozen error code");
    assert!(
        frozen_error_code.starts_with("delegate_async_running_overdue_marked_failed:"),
        "unexpected running frozen error code: {frozen_error_code}"
    );
    assert_eq!(
        outcome.payload["recovery_action"]["kind"],
        "running_async_overdue_marked_failed"
    );
    assert_eq!(
        outcome.payload["recovery_action"]["previous_state"],
        "running"
    );
    assert_eq!(outcome.payload["recovery_action"]["reference"], "started");
    assert_eq!(
        outcome.payload["recent_events"]
            .as_array()
            .expect("recent events array")
            .last()
            .expect("latest recent event")["event_kind"],
        "delegate_recovery_applied"
    );
}

#[test]
fn session_recover_rejects_fresh_running_child() {
    let config = isolated_memory_config("session-recover-running");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 30
        }),
    })
    .expect("append started event");

    let error = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_recover".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect_err("running child should be rejected");

    assert!(
        error.contains("session_recover_not_recoverable"),
        "expected recoverability rejection, got: {error}"
    );
}

#[test]
fn session_recover_batch_dry_run_reports_mixed_results_without_mutation() {
    let config = isolated_memory_config("session-recover-batch-dry-run");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "overdue-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Overdue".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create overdue child");
    repo.create_session(NewSessionRecord {
        session_id: "fresh-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Fresh".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create fresh child");
    repo.create_session(NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create hidden root");
    repo.append_event(NewSessionEvent {
        session_id: "overdue-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 30
        }),
    })
    .expect("append overdue queued");
    repo.append_event(NewSessionEvent {
        session_id: "fresh-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append fresh queued");
    overwrite_session_event_ts(
        &config,
        "overdue-child",
        "delegate_queued",
        super::current_unix_ts() - 90,
    );

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_recover".to_owned(),
            payload: json!({
                "session_ids": ["overdue-child", "fresh-child", "hidden-root"],
                "dry_run": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_recover batch dry_run outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_recover");
    assert_eq!(outcome.payload["dry_run"], true);
    assert_eq!(outcome.payload["requested_count"], 3);
    assert_eq!(outcome.payload["result_counts"]["would_apply"], 1);
    assert_eq!(
        outcome.payload["result_counts"]["skipped_not_recoverable"],
        1
    );
    assert_eq!(outcome.payload["result_counts"]["skipped_not_visible"], 1);

    let overdue = batch_result(&outcome.payload, "overdue-child");
    assert_eq!(overdue["result"], "would_apply");
    assert_eq!(
        overdue["action"]["kind"],
        "queued_async_overdue_marked_failed"
    );
    assert_eq!(overdue["inspection"]["session"]["state"], "ready");

    let fresh = batch_result(&outcome.payload, "fresh-child");
    assert_eq!(fresh["result"], "skipped_not_recoverable");
    assert!(
        fresh["message"]
            .as_str()
            .expect("fresh batch message")
            .contains("session_recover_not_recoverable")
    );
    assert_eq!(fresh["inspection"]["session"]["state"], "ready");

    let hidden = batch_result(&outcome.payload, "hidden-root");
    assert_eq!(hidden["result"], "skipped_not_visible");
    assert!(
        hidden["message"]
            .as_str()
            .expect("hidden batch message")
            .contains("visibility_denied")
    );
    assert!(hidden["inspection"].is_null());

    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("overdue-child")
            .expect("load overdue summary")
            .expect("overdue session")
            .state,
        SessionState::Ready
    );
    assert!(
        repo.load_terminal_outcome("overdue-child")
            .expect("load overdue outcome")
            .is_none()
    );
}

#[test]
fn session_recover_batch_apply_reports_partial_success() {
    let config = isolated_memory_config("session-recover-batch-apply");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "queued-overdue".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Queued Overdue".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create queued overdue");
    repo.create_session(NewSessionRecord {
        session_id: "running-overdue".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Running Overdue".to_owned()),
        state: SessionState::Running,
    })
    .expect("create running overdue");
    repo.create_session(NewSessionRecord {
        session_id: "fresh-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Fresh".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create fresh child");
    repo.append_event(NewSessionEvent {
        session_id: "queued-overdue".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "queued work",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued overdue event");
    repo.append_event(NewSessionEvent {
        session_id: "running-overdue".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "running work",
            "timeout_seconds": 30
        }),
    })
    .expect("append running queued event");
    repo.append_event(NewSessionEvent {
        session_id: "running-overdue".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "running work",
            "timeout_seconds": 30
        }),
    })
    .expect("append running started event");
    repo.append_event(NewSessionEvent {
        session_id: "fresh-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "fresh work",
            "timeout_seconds": 60
        }),
    })
    .expect("append fresh event");
    overwrite_session_event_ts(
        &config,
        "queued-overdue",
        "delegate_queued",
        super::current_unix_ts() - 90,
    );
    overwrite_session_event_ts(
        &config,
        "running-overdue",
        "delegate_queued",
        super::current_unix_ts() - 120,
    );
    overwrite_session_event_ts(
        &config,
        "running-overdue",
        "delegate_started",
        super::current_unix_ts() - 90,
    );

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_recover".to_owned(),
            payload: json!({
                "session_ids": ["queued-overdue", "running-overdue", "fresh-child"]
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_recover batch apply outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_recover");
    assert_eq!(outcome.payload["dry_run"], false);
    assert_eq!(outcome.payload["requested_count"], 3);
    assert_eq!(outcome.payload["result_counts"]["applied"], 2);
    assert_eq!(
        outcome.payload["result_counts"]["skipped_not_recoverable"],
        1
    );

    let queued = batch_result(&outcome.payload, "queued-overdue");
    assert_eq!(queued["result"], "applied");
    assert_eq!(queued["inspection"]["session"]["state"], "failed");
    assert_eq!(
        queued["action"]["kind"],
        "queued_async_overdue_marked_failed"
    );
    assert_eq!(
        queued["inspection"]["delegate_lifecycle"]["phase"],
        "failed"
    );

    let running = batch_result(&outcome.payload, "running-overdue");
    assert_eq!(running["result"], "applied");
    assert_eq!(running["inspection"]["session"]["state"], "failed");
    assert_eq!(
        running["action"]["kind"],
        "running_async_overdue_marked_failed"
    );
    assert_eq!(running["action"]["reference"], "started");
    assert_eq!(
        running["inspection"]["recent_events"]
            .as_array()
            .expect("running recent events")
            .last()
            .expect("running latest event")["event_kind"],
        "delegate_recovery_applied"
    );

    let fresh = batch_result(&outcome.payload, "fresh-child");
    assert_eq!(fresh["result"], "skipped_not_recoverable");
    assert_eq!(fresh["inspection"]["session"]["state"], "ready");

    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("queued-overdue")
            .expect("load queued summary")
            .expect("queued session")
            .state,
        SessionState::Failed
    );
    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("running-overdue")
            .expect("load running summary")
            .expect("running session")
            .state,
        SessionState::Failed
    );
    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("fresh-child")
            .expect("load fresh summary")
            .expect("fresh session")
            .state,
        SessionState::Ready
    );
    assert!(
        repo.load_terminal_outcome("queued-overdue")
            .expect("load queued outcome")
            .is_some()
    );
    assert!(
        repo.load_terminal_outcome("running-overdue")
            .expect("load running outcome")
            .is_some()
    );
    assert!(
        repo.load_terminal_outcome("fresh-child")
            .expect("load fresh outcome")
            .is_none()
    );
}

#[test]
fn session_cancel_cancels_queued_async_child() {
    let config = isolated_memory_config("session-cancel-queued");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append queued event");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_cancel".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_cancel outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["session"]["state"], "failed");
    assert_eq!(outcome.payload["workflow"]["phase"], "cancelled");
    assert_eq!(outcome.payload["terminal_outcome_state"], "present");
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
    assert_eq!(
        outcome.payload["terminal_outcome"]["frozen_result"]["content"]["error"]["code"],
        "delegate_cancelled: operator_requested"
    );
    assert_eq!(
        outcome.payload["cancel_action"]["kind"],
        "queued_async_cancelled"
    );
    assert_eq!(
        outcome.payload["recent_events"]
            .as_array()
            .expect("recent events array")
            .last()
            .expect("latest recent event")["event_kind"],
        "delegate_cancelled"
    );
}

#[test]
fn session_cancel_requests_running_async_child_cancellation() {
    let config = isolated_memory_config("session-cancel-running");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append started event");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_cancel".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_cancel outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["session"]["state"], "running");
    assert_eq!(outcome.payload["terminal_outcome_state"], "not_terminal");
    assert_eq!(
        outcome.payload["cancel_action"]["kind"],
        "running_async_cancel_requested"
    );
    assert_eq!(
        outcome.payload["recent_events"]
            .as_array()
            .expect("recent events array")
            .last()
            .expect("latest recent event")["event_kind"],
        "delegate_cancel_requested"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["cancellation"]["state"],
        "requested"
    );
}

#[test]
fn session_cancel_batch_dry_run_reports_mixed_results_without_mutation() {
    let config = isolated_memory_config("session-cancel-batch-dry-run");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "queued-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Queued".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create queued child");
    repo.create_session(NewSessionRecord {
        session_id: "running-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Running".to_owned()),
        state: SessionState::Running,
    })
    .expect("create running child");
    repo.create_session(NewSessionRecord {
        session_id: "completed-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Completed".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create completed child");
    repo.create_session(NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create hidden root");
    repo.append_event(NewSessionEvent {
        session_id: "queued-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "queued work",
            "timeout_seconds": 60
        }),
    })
    .expect("append queued child event");
    repo.append_event(NewSessionEvent {
        session_id: "running-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "running work",
            "timeout_seconds": 60
        }),
    })
    .expect("append running queued event");
    repo.append_event(NewSessionEvent {
        session_id: "running-child".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "running work",
            "timeout_seconds": 60
        }),
    })
    .expect("append running started event");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_cancel".to_owned(),
            payload: json!({
                "session_ids": ["queued-child", "running-child", "completed-child", "hidden-root"],
                "dry_run": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_cancel batch dry_run outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_cancel");
    assert_eq!(outcome.payload["dry_run"], true);
    assert_eq!(outcome.payload["requested_count"], 4);
    assert_eq!(outcome.payload["result_counts"]["would_apply"], 2);
    assert_eq!(
        outcome.payload["result_counts"]["skipped_not_cancellable"],
        1
    );
    assert_eq!(outcome.payload["result_counts"]["skipped_not_visible"], 1);

    let queued = batch_result(&outcome.payload, "queued-child");
    assert_eq!(queued["result"], "would_apply");
    assert_eq!(queued["action"]["kind"], "queued_async_cancelled");
    assert_eq!(queued["inspection"]["session"]["state"], "ready");

    let running = batch_result(&outcome.payload, "running-child");
    assert_eq!(running["result"], "would_apply");
    assert_eq!(running["action"]["kind"], "running_async_cancel_requested");
    assert_eq!(running["inspection"]["session"]["state"], "running");

    let completed = batch_result(&outcome.payload, "completed-child");
    assert_eq!(completed["result"], "skipped_not_cancellable");
    assert_eq!(completed["inspection"]["session"]["state"], "completed");

    let hidden = batch_result(&outcome.payload, "hidden-root");
    assert_eq!(hidden["result"], "skipped_not_visible");
    assert!(hidden["inspection"].is_null());

    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("queued-child")
            .expect("load queued summary")
            .expect("queued session")
            .state,
        SessionState::Ready
    );
    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("running-child")
            .expect("load running summary")
            .expect("running session")
            .state,
        SessionState::Running
    );
    assert!(
        repo.load_terminal_outcome("queued-child")
            .expect("load queued outcome")
            .is_none()
    );
}

#[test]
fn session_cancel_batch_apply_reports_partial_success() {
    let config = isolated_memory_config("session-cancel-batch-apply");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "queued-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Queued".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create queued child");
    repo.create_session(NewSessionRecord {
        session_id: "running-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Running".to_owned()),
        state: SessionState::Running,
    })
    .expect("create running child");
    repo.create_session(NewSessionRecord {
        session_id: "completed-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Completed".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create completed child");
    repo.append_event(NewSessionEvent {
        session_id: "queued-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "queued work",
            "timeout_seconds": 60
        }),
    })
    .expect("append queued child event");
    repo.append_event(NewSessionEvent {
        session_id: "running-child".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "running work",
            "timeout_seconds": 60
        }),
    })
    .expect("append running queued event");
    repo.append_event(NewSessionEvent {
        session_id: "running-child".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "running work",
            "timeout_seconds": 60
        }),
    })
    .expect("append running started event");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_cancel".to_owned(),
            payload: json!({
                "session_ids": ["queued-child", "running-child", "completed-child"]
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_cancel batch apply outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_cancel");
    assert_eq!(outcome.payload["dry_run"], false);
    assert_eq!(outcome.payload["requested_count"], 3);
    assert_eq!(outcome.payload["result_counts"]["applied"], 2);
    assert_eq!(
        outcome.payload["result_counts"]["skipped_not_cancellable"],
        1
    );

    let queued = batch_result(&outcome.payload, "queued-child");
    assert_eq!(queued["result"], "applied");
    assert_eq!(queued["inspection"]["session"]["state"], "failed");
    assert_eq!(queued["action"]["kind"], "queued_async_cancelled");
    assert_eq!(
        queued["inspection"]["recent_events"]
            .as_array()
            .expect("queued recent events")
            .last()
            .expect("queued latest event")["event_kind"],
        "delegate_cancelled"
    );

    let running = batch_result(&outcome.payload, "running-child");
    assert_eq!(running["result"], "applied");
    assert_eq!(running["inspection"]["session"]["state"], "running");
    assert_eq!(running["action"]["kind"], "running_async_cancel_requested");
    assert_eq!(
        running["inspection"]["delegate_lifecycle"]["cancellation"]["state"],
        "requested"
    );

    let completed = batch_result(&outcome.payload, "completed-child");
    assert_eq!(completed["result"], "skipped_not_cancellable");
    assert_eq!(completed["inspection"]["session"]["state"], "completed");

    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("queued-child")
            .expect("load queued summary")
            .expect("queued session")
            .state,
        SessionState::Failed
    );
    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("running-child")
            .expect("load running summary")
            .expect("running session")
            .state,
        SessionState::Running
    );
    assert!(
        repo.load_terminal_outcome("queued-child")
            .expect("load queued outcome")
            .is_some()
    );
    let queued_outcome = repo
        .load_terminal_outcome("queued-child")
        .expect("load queued outcome")
        .expect("queued outcome row");
    assert!(queued_outcome.frozen_result.is_some());
    assert!(
        repo.load_terminal_outcome("running-child")
            .expect("load running outcome")
            .is_none()
    );
}

#[test]
fn session_cancel_requested_state_is_visible_in_session_status() {
    let config = isolated_memory_config("session-cancel-status");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append started event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_cancel_requested".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "reference": "running",
            "cancel_reason": "operator_requested"
        }),
    })
    .expect("append cancel requested event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(
        outcome.payload["delegate_lifecycle"]["cancellation"]["state"],
        "requested"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["cancellation"]["reference"],
        "running"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["cancellation"]["reason"],
        "operator_requested"
    );
}

#[test]
fn session_delegate_lifecycle_marks_overdue_queued_child() {
    let session = SessionSummaryRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
        created_at: 100,
        updated_at: 100,
        archived_at: None,
        turn_count: 0,
        last_turn_at: None,
        last_error: None,
    };
    let events = vec![SessionEventRecord {
        id: 1,
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 30
        }),
        ts: 100,
    }];

    let lifecycle =
        super::session_delegate_lifecycle_at(&session, &events, 140).expect("delegate lifecycle");

    assert_eq!(lifecycle.mode, "async");
    assert_eq!(lifecycle.phase, "queued");
    assert_eq!(lifecycle.queued_at, Some(100));
    assert_eq!(lifecycle.started_at, None);
    assert_eq!(lifecycle.timeout_seconds, Some(30));
    let staleness = lifecycle.staleness.expect("staleness");
    assert_eq!(staleness.state, "overdue");
    assert_eq!(staleness.reference, "queued");
    assert_eq!(staleness.elapsed_seconds, 40);
    assert_eq!(staleness.threshold_seconds, 30);
    assert_eq!(staleness.deadline_at, 130);
}

#[test]
fn session_status_includes_delegate_lifecycle_for_queued_child() {
    let config = isolated_memory_config("session-status-delegate-lifecycle");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "label": "Child",
            "profile": "research",
            "timeout_seconds": 60,
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 2,
                "active_children": 0,
                "max_active_children": 3,
                "timeout_seconds": 60,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["read", "write", "edit"],
                "kernel_bound": false,
                "runtime_narrowing": {
                    "web_fetch": {
                        "allowed_domains": ["docs.example.com"],
                        "allow_private_hosts": false
                    },
                    "browser": {
                        "max_sessions": 1
                    }
                }
            }
        }),
    })
    .expect("append queued event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(outcome.payload["delegate_lifecycle"]["profile"], "research");
    assert_eq!(outcome.payload["delegate_lifecycle"]["mode"], "async");
    assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "queued");
    assert_eq!(outcome.payload["delegate_lifecycle"]["timeout_seconds"], 60);
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["staleness"]["reference"],
        "queued"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["staleness"]["state"],
        "fresh"
    );
    assert!(outcome.payload["delegate_lifecycle"]["queued_at"].is_number());
    assert!(outcome.payload["delegate_lifecycle"]["started_at"].is_null());
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["mode"],
        "async"
    );
    assert_eq!(outcome.payload["subagent"]["session_id"], "child-session");
    assert_eq!(outcome.payload["subagent_identity"]["nickname"], "Child");
    assert_eq!(
        outcome.payload["subagent_contract"]["identity"]["nickname"],
        "Child"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["depth"],
        1
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["max_depth"],
        2
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["active_children"],
        0
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["max_active_children"],
        3
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["allow_shell_in_child"],
        false
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["child_tool_allowlist"],
        json!(["read", "write", "edit"])
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["kernel_bound"],
        false
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["runtime_narrowing"]["web_fetch"]["allowed_domains"],
        json!(["docs.example.com"])
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["runtime_narrowing"]["web_fetch"]["allow_private_hosts"],
        false
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["execution"]["runtime_narrowing"]["browser"]["max_sessions"],
        1
    );
}

#[test]
fn session_status_uses_delegate_lifecycle_anchor_events_when_recent_window_is_noisy() {
    let config = isolated_memory_config("session-status-lifecycle-noisy-window");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({ "timeout_seconds": 30 }),
    })
    .expect("append queued event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({ "timeout_seconds": 30 }),
    })
    .expect("append started event");
    overwrite_session_event_ts(
        &config,
        "child-session",
        "delegate_queued",
        super::current_unix_ts() - 120,
    );
    overwrite_session_event_ts(
        &config,
        "child-session",
        "delegate_started",
        super::current_unix_ts() - 90,
    );
    for step in 0..20 {
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: format!("delegate_progress_{step}"),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "step": step }),
        })
        .expect("append progress event");
    }

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(outcome.payload["delegate_lifecycle"]["mode"], "async");
    assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "running");
    assert_eq!(outcome.payload["delegate_lifecycle"]["timeout_seconds"], 30);
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["staleness"]["reference"],
        "started"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["staleness"]["state"],
        "overdue"
    );
    assert!(outcome.payload["delegate_lifecycle"]["started_at"].is_number());
}

#[test]
fn session_delegate_lifecycle_prefers_execution_mode_when_history_is_partial() {
    let session = SessionSummaryRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Completed,
        created_at: 100,
        updated_at: 120,
        archived_at: None,
        turn_count: 1,
        last_turn_at: Some(120),
        last_error: None,
    };
    let events = vec![
        SessionEventRecord {
            id: 1,
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 2,
                    "active_children": 0,
                    "max_active_children": 3,
                    "timeout_seconds": 60,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["read"],
                    "kernel_bound": false
                }
            }),
            ts: 110,
        },
        SessionEventRecord {
            id: 2,
            session_id: "child-session".to_owned(),
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "terminal_reason": "completed"
            }),
            ts: 120,
        },
    ];

    let lifecycle =
        super::session_delegate_lifecycle_at(&session, &events, 130).expect("delegate lifecycle");

    assert_eq!(
        lifecycle.mode, "async",
        "persisted execution.mode should win when queued metadata is absent"
    );
    assert_eq!(lifecycle.phase, "completed");
}

#[test]
fn session_tools_reject_invisible_sessions() {
    let config = isolated_memory_config("session-visibility");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "other-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Other".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create other");

    let error = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "other-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect_err("invisible session should be rejected");

    assert!(
        error.contains("visibility_denied"),
        "expected visibility_denied, got: {error}"
    );
}

#[test]
fn session_status_returns_inferred_legacy_current_session_without_backfill() {
    let config = isolated_memory_config("legacy-session-status");
    append_session_turn_direct("delegate:legacy-child", "user", "hello", &config)
        .expect("append user turn");
    append_session_turn_direct("delegate:legacy-child", "assistant", "done", &config)
        .expect("append assistant turn");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "delegate:legacy-child"
            }),
        },
        "delegate:legacy-child",
        &config,
    )
    .expect("legacy session_status outcome");

    assert_eq!(
        outcome.payload["session"]["session_id"],
        "delegate:legacy-child"
    );
    assert_eq!(outcome.payload["session"]["kind"], "delegate_child");
    assert_eq!(outcome.payload["session"]["state"], "ready");
    assert_eq!(outcome.payload["terminal_outcome_state"], "not_terminal");
    assert!(outcome.payload["terminal_outcome_missing_reason"].is_null());
    assert!(outcome.payload["delegate_lifecycle"].is_null());
    assert!(outcome.payload["terminal_outcome"].is_null());
    assert_eq!(
        outcome.payload["recent_events"]
            .as_array()
            .expect("recent_events array")
            .len(),
        0
    );

    let repo = SessionRepository::new(&config).expect("repository");
    assert!(
        repo.load_session("delegate:legacy-child")
            .expect("load legacy session")
            .is_none()
    );
}

#[test]
fn session_status_allows_visible_descendant_delegate_session() {
    let config = isolated_memory_config("descendant-session-status");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create child");
    repo.create_session(NewSessionRecord {
        session_id: "grandchild-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("child-session".to_owned()),
        label: Some("Grandchild".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create grandchild");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "grandchild-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("descendant session_status outcome");

    assert_eq!(
        outcome.payload["session"]["session_id"],
        "grandchild-session"
    );
    assert_eq!(outcome.payload["session"]["kind"], "delegate_child");
}

#[test]
fn session_status_batch_returns_mixed_visible_and_hidden_results() {
    let config = isolated_memory_config("session-status-batch");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.create_session(NewSessionRecord {
        session_id: "grandchild-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("child-session".to_owned()),
        label: Some("Grandchild".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create grandchild");
    repo.create_session(NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create hidden root");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_ids": ["hidden-root", "grandchild-session", "child-session"]
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status batch outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_status");
    assert_eq!(outcome.payload["requested_count"], 3);
    assert_eq!(outcome.payload["result_counts"]["ok"], 2);
    assert_eq!(outcome.payload["result_counts"]["skipped_not_visible"], 1);

    let results = outcome.payload["results"]
        .as_array()
        .expect("batch results array");
    let ids: Vec<&str> = results
        .iter()
        .filter_map(|item| item.get("session_id"))
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        ids,
        vec!["hidden-root", "grandchild-session", "child-session"]
    );

    let hidden = batch_result(&outcome.payload, "hidden-root");
    assert_eq!(hidden["result"], "skipped_not_visible");
    assert!(hidden["inspection"].is_null());
    assert!(
        hidden["message"]
            .as_str()
            .expect("hidden message")
            .contains("visibility_denied")
    );

    let grandchild = batch_result(&outcome.payload, "grandchild-session");
    assert_eq!(grandchild["result"], "ok");
    assert_eq!(grandchild["inspection"]["session"]["state"], "completed");
    assert_eq!(
        grandchild["inspection"]["session"]["session_id"],
        "grandchild-session"
    );

    let child = batch_result(&outcome.payload, "child-session");
    assert_eq!(child["result"], "ok");
    assert_eq!(child["inspection"]["session"]["state"], "running");
    assert_eq!(
        child["inspection"]["terminal_outcome_state"],
        "not_terminal"
    );
}

#[test]
fn session_archive_archives_terminal_visible_session() {
    let config = isolated_memory_config("session-archive-single");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");
    repo.finalize_session_terminal(
        "child-session",
        FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "result": "ok"
            }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "child-session",
                "result": "ok"
            }),
            frozen_result: None,
        },
    )
    .expect("finalize child");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_archive".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_archive outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["session"]["session_id"], "child-session");
    assert_eq!(outcome.payload["session"]["state"], "completed");
    assert_eq!(outcome.payload["session"]["archived"], true);
    assert!(outcome.payload["session"]["archived_at"].is_number());
    assert_eq!(
        outcome.payload["archive_action"]["kind"],
        "session_archived"
    );

    let status = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: json!({
                "session_id": "child-session"
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_status outcome");

    assert_eq!(status.payload["session"]["archived"], true);
    assert!(status.payload["session"]["archived_at"].is_number());
}

#[test]
fn session_archive_batch_dry_run_reports_mixed_results_without_mutation() {
    let config = isolated_memory_config("session-archive-batch-dry-run");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "ready-to-archive".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Ready".to_owned()),
        state: SessionState::Running,
    })
    .expect("create archivable child");
    repo.create_session(NewSessionRecord {
        session_id: "already-archived".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Archived".to_owned()),
        state: SessionState::Running,
    })
    .expect("create archived child");
    repo.create_session(NewSessionRecord {
        session_id: "running-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Running".to_owned()),
        state: SessionState::Running,
    })
    .expect("create running child");

    for session_id in ["ready-to-archive", "already-archived"] {
        repo.finalize_session_terminal(
            session_id,
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({ "result": "ok" }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({ "child_session_id": session_id }),
                frozen_result: None,
            },
        )
        .expect("finalize child");
    }
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_archive".to_owned(),
            payload: json!({
                "session_id": "already-archived"
            }),
        },
        "root-session",
        &config,
    )
    .expect("archive already-archived child");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_archive".to_owned(),
            payload: json!({
                "session_ids": ["ready-to-archive", "already-archived", "running-child"],
                "dry_run": true
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_archive batch dry_run outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_archive");
    assert_eq!(outcome.payload["dry_run"], true);
    assert_eq!(outcome.payload["requested_count"], 3);
    assert_eq!(outcome.payload["result_counts"]["would_apply"], 1);
    assert_eq!(
        outcome.payload["result_counts"]["skipped_already_archived"],
        1
    );
    assert_eq!(
        outcome.payload["result_counts"]["skipped_not_archivable"],
        1
    );

    let ready = batch_result(&outcome.payload, "ready-to-archive");
    assert_eq!(ready["result"], "would_apply");
    assert_eq!(ready["inspection"]["session"]["archived"], false);
    assert_eq!(ready["action"]["kind"], "session_archived");

    let archived = batch_result(&outcome.payload, "already-archived");
    assert_eq!(archived["result"], "skipped_already_archived");
    assert_eq!(archived["inspection"]["session"]["archived"], true);

    let running = batch_result(&outcome.payload, "running-child");
    assert_eq!(running["result"], "skipped_not_archivable");
    assert_eq!(running["inspection"]["session"]["state"], "running");

    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("ready-to-archive")
            .expect("load ready summary")
            .expect("ready session")
            .archived_at,
        None
    );
}

#[test]
fn session_archive_batch_apply_reports_partial_success() {
    let config = isolated_memory_config("session-archive-batch-apply");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "ready-to-archive".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Ready".to_owned()),
        state: SessionState::Running,
    })
    .expect("create archivable child");
    repo.create_session(NewSessionRecord {
        session_id: "already-archived".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Archived".to_owned()),
        state: SessionState::Running,
    })
    .expect("create archived child");
    repo.create_session(NewSessionRecord {
        session_id: "running-child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Running".to_owned()),
        state: SessionState::Running,
    })
    .expect("create running child");

    for session_id in ["ready-to-archive", "already-archived"] {
        repo.finalize_session_terminal(
            session_id,
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({ "result": "ok" }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({ "child_session_id": session_id }),
                frozen_result: None,
            },
        )
        .expect("finalize child");
    }
    execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_archive".to_owned(),
            payload: json!({
                "session_id": "already-archived"
            }),
        },
        "root-session",
        &config,
    )
    .expect("archive already-archived child");

    let outcome = execute_session_mutation_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_archive".to_owned(),
            payload: json!({
                "session_ids": ["ready-to-archive", "already-archived", "running-child"]
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_archive batch apply outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_archive");
    assert_eq!(outcome.payload["dry_run"], false);
    assert_eq!(outcome.payload["requested_count"], 3);
    assert_eq!(outcome.payload["result_counts"]["applied"], 1);
    assert_eq!(
        outcome.payload["result_counts"]["skipped_already_archived"],
        1
    );
    assert_eq!(
        outcome.payload["result_counts"]["skipped_not_archivable"],
        1
    );

    let ready = batch_result(&outcome.payload, "ready-to-archive");
    assert_eq!(ready["result"], "applied");
    assert_eq!(ready["inspection"]["session"]["archived"], true);
    assert_eq!(ready["action"]["kind"], "session_archived");
    assert_eq!(
        ready["inspection"]["recent_events"]
            .as_array()
            .expect("ready recent events")
            .last()
            .expect("ready latest event")["event_kind"],
        "session_archived"
    );

    let archived = batch_result(&outcome.payload, "already-archived");
    assert_eq!(archived["result"], "skipped_already_archived");
    assert_eq!(archived["inspection"]["session"]["archived"], true);

    let running = batch_result(&outcome.payload, "running-child");
    assert_eq!(running["result"], "skipped_not_archivable");
    assert_eq!(running["inspection"]["session"]["state"], "running");

    assert!(
        repo.load_session_summary_with_legacy_fallback("ready-to-archive")
            .expect("load ready summary")
            .expect("ready session")
            .archived_at
            .is_some()
    );
    assert!(
        repo.load_session_summary_with_legacy_fallback("already-archived")
            .expect("load archived summary")
            .expect("archived session")
            .archived_at
            .is_some()
    );
    assert_eq!(
        repo.load_session_summary_with_legacy_fallback("running-child")
            .expect("load running summary")
            .expect("running session")
            .archived_at,
        None
    );
}

#[tokio::test]
async fn session_wait_wakes_when_parent_mailbox_receives_delegate_result() {
    let config = isolated_memory_config("session-wait-mailbox-wake");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    let config_for_completion = config.clone();
    let completion = tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        let repo = SessionRepository::new(&config_for_completion).expect("completion repo");
        repo.finalize_session_terminal(
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "result": "ok"
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "child-session",
                    "result": "ok"
                }),
                frozen_result: None,
            },
        )
        .expect("finalize child");

        let mailbox = mailbox_for_session("root-session");
        let send_result = mailbox.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::DelegateResult {
                session_id: "child-session".to_owned(),
                frozen_result: json!({
                    "status": "ok"
                }),
            },
            trigger_turn: true,
        });
        assert!(send_result.is_ok());
    });

    let wait_timeout_ms = 1_000_u64;
    let poll_interval_ms = 10_usize;
    let outcome = wait_for_single_session_with_policies(
        "child-session",
        "root-session",
        &config,
        &ToolConfig::default(),
        None,
        wait_timeout_ms,
        poll_interval_ms,
    )
    .await
    .expect("session_wait outcome");
    completion.await.expect("completion task");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["session"]["state"], "completed");
}

#[tokio::test]
async fn task_wait_wakes_when_canonical_task_owner_session_completes() {
    let config = isolated_memory_config("task-wait-mailbox-wake");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "task-owner".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Task Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Active,
                intent_summary: Some("Mailbox wake for canonical task".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 1,
            },
        ),
    })
    .expect("append task progress event");

    let config_for_completion = config.clone();
    let completion = tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        let repo = SessionRepository::new(&config_for_completion).expect("completion repo");
        repo.finalize_session_terminal(
            "task-owner",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("task-owner".to_owned()),
                event_payload_json: json!({
                    "result": "ok"
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "task-owner",
                    "result": "ok"
                }),
                frozen_result: None,
            },
        )
        .expect("finalize task");

        let mailbox = mailbox_for_session("task-owner");
        let send_result = mailbox.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::DelegateResult {
                session_id: "task-owner".to_owned(),
                frozen_result: json!({
                    "status": "ok"
                }),
            },
            trigger_turn: true,
        });
        assert!(send_result.is_ok());
    });

    let outcome = crate::tools::wait_for_task_with_config(
        json!({
            "task_id": "task-root",
            "timeout_ms": 1_000
        }),
        "task-owner",
        &config,
        &ToolConfig::default(),
    )
    .await
    .expect("task_wait outcome");
    completion.await.expect("completion task");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "task_wait");
    assert_eq!(outcome.payload["task_id"], "task-root");
    assert_eq!(outcome.payload["owner_session_id"], "task-owner");
    assert_eq!(outcome.payload["task_session_id"], "task-owner");
    assert_eq!(outcome.payload["task_session_count"], 1);
    assert_eq!(
        outcome.payload["task_sessions"][0]["task_session_id"],
        "task-owner"
    );
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["task_state"], "completed");
    assert_eq!(outcome.payload["task_is_stable"], true);
}

#[tokio::test]
async fn task_wait_returns_immediately_for_waiting_canonical_task_state() {
    let config = isolated_memory_config("task-wait-waiting-state");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "task-owner".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Task Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    repo.append_event(NewSessionEvent {
        session_id: "task-owner".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-owner".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: Some("Await approval".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: vec![crate::task_progress::TaskActiveHandleRecord {
                    handle_kind: "approval_gate".to_owned(),
                    handle_id: "task-owner".to_owned(),
                    state: "waiting".to_owned(),
                    last_event_at: Some(123),
                    stop_condition: "approval_decision".to_owned(),
                }],
                resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                    recommended_tool: "task_status".to_owned(),
                    task_session_id: "task-owner".to_owned(),
                    note: Some("Inspect task status for the approval gate.".to_owned()),
                }),
                updated_at: 123,
            },
        ),
    })
    .expect("append task progress event");

    let started_at = Instant::now();
    let outcome = crate::tools::wait_for_task_with_config(
        json!({
            "task_id": "task-root",
            "timeout_ms": 1_000
        }),
        "task-owner",
        &config,
        &ToolConfig::default(),
    )
    .await
    .expect("task_wait outcome");
    let immediate_resolution_budget = Duration::from_millis(500);

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "waiting");
    assert_eq!(outcome.payload["owner_session_id"], "task-owner");
    assert_eq!(outcome.payload["task_session_id"], "task-owner");
    assert_eq!(outcome.payload["task_session_count"], 1);
    assert_eq!(
        outcome.payload["task_sessions"][0]["task_session_id"],
        "task-owner"
    );
    assert_eq!(outcome.payload["task_state"], "waiting");
    assert_eq!(outcome.payload["task_is_stable"], true);
    assert_eq!(outcome.payload["continuation"]["state"], "waiting");
    assert_eq!(outcome.payload["continuation"]["is_terminal"], false);
    assert!(
        started_at.elapsed() < immediate_resolution_budget,
        "waiting task state should resolve without waiting for terminal session state"
    );
}

#[tokio::test]
async fn session_wait_waiting_state_exposes_generic_continuation_metadata() {
    let config = isolated_memory_config("session-wait-continuation");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    let outcome = crate::tools::wait_for_session_with_config(
        json!({
            "session_id": "child-session",
            "timeout_ms": 100
        }),
        "root-session",
        &config,
        &ToolConfig::default(),
    )
    .await
    .expect("session_wait outcome");

    assert_eq!(outcome.payload["wait_status"], "timeout");
    assert_eq!(outcome.payload["continuation"]["state"], "timeout");
    assert_eq!(outcome.payload["continuation"]["is_terminal"], false);
}

#[tokio::test]
async fn task_wait_follows_latest_owner_session_for_reassigned_task() {
    let config = isolated_memory_config("task-wait-reassigned-owner");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    for session_id in ["owner-old", "owner-new"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
    }
    repo.append_event(NewSessionEvent {
        session_id: "owner-old".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("owner-old".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-root".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Active,
                intent_summary: Some("Initial owner".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::NotStarted),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 10,
            },
        ),
    })
    .expect("append old owner task progress");

    let config_for_completion = config.clone();
    let completion = tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        let repo = SessionRepository::new(&config_for_completion).expect("completion repo");
        repo.append_event(NewSessionEvent {
            session_id: "owner-new".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("owner-new".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Completed,
                    intent_summary: Some("Reassigned owner".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Passed),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 20,
                },
            ),
        })
        .expect("append new owner task progress");

        let mailbox = mailbox_for_session("root-session");
        let send_result = mailbox.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::DelegateResult {
                session_id: "owner-new".to_owned(),
                frozen_result: json!({
                    "status": "ok"
                }),
            },
            trigger_turn: true,
        });
        assert!(send_result.is_ok());
    });

    let outcome = crate::tools::wait_for_task_with_config(
        json!({
            "task_id": "task-root",
            "timeout_ms": 5_000
        }),
        "root-session",
        &config,
        &ToolConfig::default(),
    )
    .await
    .expect("task_wait outcome");
    completion.await.expect("completion task");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["task_id"], "task-root");
    assert_eq!(outcome.payload["owner_session_id"], "owner-new");
    assert_eq!(outcome.payload["task_session_id"], "owner-new");
    let task_sessions = outcome.payload["task_sessions"]
        .as_array()
        .expect("task sessions");
    assert_eq!(outcome.payload["task_session_count"], 2);
    assert_eq!(task_sessions.len(), 2);
    assert_eq!(task_sessions[0]["task_session_id"], "owner-old");
    assert_eq!(task_sessions[0]["is_current_owner"], false);
    assert_eq!(task_sessions[1]["task_session_id"], "owner-new");
    assert_eq!(task_sessions[1]["is_current_owner"], true);
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["task_state"], "completed");
    assert_eq!(outcome.payload["task_is_stable"], true);
}

#[test]
fn tasks_list_filters_by_task_state_and_stability() {
    let config = isolated_memory_config("tasks-list-filters-visible");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Running,
    })
    .expect("create root");
    for session_id in ["task-active", "task-waiting"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
    }
    repo.append_event(NewSessionEvent {
        session_id: "task-active".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-active".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-active".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Active,
                intent_summary: Some("Active task".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::NotStarted),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 100,
            },
        ),
    })
    .expect("append active task progress event");
    repo.append_event(NewSessionEvent {
        session_id: "task-waiting".to_owned(),
        event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some("task-waiting".to_owned()),
        payload_json: crate::task_progress::task_progress_event_payload(
            "unit_test",
            &crate::task_progress::TaskProgressRecord {
                task_id: "task-waiting".to_owned(),
                owner_kind: "conversation_turn".to_owned(),
                status: crate::task_progress::TaskProgressStatus::Waiting,
                intent_summary: Some("Waiting task".to_owned()),
                verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                active_handles: Vec::new(),
                resume_recipe: None,
                updated_at: 101,
            },
        ),
    })
    .expect("append waiting task progress event");

    let outcome = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "tasks_list".to_owned(),
            payload: json!({
                "stable_only": true,
                "task_state": "waiting"
            }),
        },
        "root-session",
        &config,
    )
    .expect("tasks_list outcome");

    assert_eq!(outcome.payload["tool"], "tasks_list");
    assert_eq!(outcome.payload["matched_count"], 1);
    assert_eq!(outcome.payload["tasks"][0]["task_id"], "task-waiting");
    assert_eq!(outcome.payload["tasks"][0]["task_state"], "waiting");
    assert_eq!(outcome.payload["tasks"][0]["task_is_stable"], true);
}

#[test]
fn session_events_returns_ordered_tail_and_respects_after_id() {
    let config = isolated_memory_config("session-events");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    let first = repo
        .append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({"step": 1}),
        })
        .expect("append first event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_progress".to_owned(),
        actor_session_id: Some("child-session".to_owned()),
        payload_json: json!({"step": 2}),
    })
    .expect("append second event");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_completed".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({"step": 3}),
    })
    .expect("append third event");

    let full = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_events".to_owned(),
            payload: json!({
                "session_id": "child-session",
                "limit": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("session_events outcome");
    let full_events = full.payload["events"].as_array().expect("events array");
    assert_eq!(full_events.len(), 3);
    assert_eq!(full_events[0]["event_kind"], "delegate_started");
    assert_eq!(full_events[1]["event_kind"], "delegate_progress");
    assert_eq!(full_events[2]["event_kind"], "delegate_completed");

    let incremental = execute_session_tool_with_config(
        ToolCoreRequest {
            tool_name: "session_events".to_owned(),
            payload: json!({
                "session_id": "child-session",
                "after_id": first.id,
                "limit": 10
            }),
        },
        "root-session",
        &config,
    )
    .expect("incremental session_events outcome");
    let incremental_events = incremental.payload["events"]
        .as_array()
        .expect("incremental events array");
    assert_eq!(incremental_events.len(), 2);
    assert_eq!(incremental_events[0]["event_kind"], "delegate_progress");
    assert_eq!(incremental_events[1]["event_kind"], "delegate_completed");
}
