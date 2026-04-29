use super::*;

fn create_delegate_session(
    repo: &mvp::session::repository::SessionRepository,
    parent_session_id: &str,
    session_id: &str,
    label: &str,
    state: mvp::session::repository::SessionState,
) {
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some(parent_session_id.to_owned()),
        label: Some(label.to_owned()),
        state,
    })
    .expect("create delegate session");
}

fn append_session_turn(
    root: &super::tasks_cli::TempDirGuard,
    session_id: &str,
    role: &str,
    content: &str,
) {
    let sqlite_path = root.path().join("memory.sqlite3");
    let memory_config = mvp::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(sqlite_path),
        ..mvp::memory::runtime_config::MemoryRuntimeConfig::default()
    };
    mvp::memory::append_turn_direct(session_id, role, content, &memory_config)
        .expect("append session turn");
}

fn append_session_conversation_event(
    root: &super::tasks_cli::TempDirGuard,
    session_id: &str,
    payload: Value,
) {
    let serialized_payload =
        serde_json::to_string(&payload).expect("serialize session conversation event");
    append_session_turn(root, session_id, "assistant", &serialized_payload);
}

fn overwrite_session_event_ts(
    root: &super::tasks_cli::TempDirGuard,
    session_id: &str,
    event_kind: &str,
    ts: i64,
) {
    let sqlite_path = root.path().join("memory.sqlite3");
    let connection = rusqlite::Connection::open(sqlite_path).expect("open sqlite db");
    let updated = connection
        .execute(
            "UPDATE session_events
             SET ts = ?3
             WHERE session_id = ?1 AND event_kind = ?2",
            rusqlite::params![session_id, event_kind, ts],
        )
        .expect("update session event timestamp");
    assert!(updated > 0, "expected at least one updated event row");
}

fn current_unix_ts() -> i64 {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .expect("current unix timestamp");
    duration.as_secs() as i64
}

#[test]
fn sessions_list_cli_parses_global_flags_after_subcommand() {
    let cli = try_parse_cli([
        "loong",
        "sessions",
        "list",
        "--kind",
        "delegate_child",
        "--limit",
        "25",
        "--session",
        "ops-root",
        "--json",
        "--config",
        "/tmp/loong.toml",
    ])
    .expect("sessions list CLI should parse");

    let Some(loong_daemon::Commands::Sessions {
        config,
        json,
        session,
        command,
    }) = cli.command
    else {
        panic!("expected sessions command, got: {:?}", cli.command);
    };

    assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
    assert!(json, "expected --json flag to be preserved");
    assert_eq!(session, "ops-root");

    let loong_daemon::sessions_cli::SessionsCommands::List {
        limit,
        state,
        kind,
        parent_session_id,
        overdue_only,
        include_archived,
        include_delegate_lifecycle,
    } = command
    else {
        panic!("expected sessions list command");
    };

    assert_eq!(limit, 25);
    assert_eq!(state, None);
    assert_eq!(kind.as_deref(), Some("delegate_child"));
    assert_eq!(parent_session_id, None);
    assert!(!overdue_only, "unexpected overdue_only flag");
    assert!(!include_archived, "unexpected include_archived flag");
    assert!(
        !include_delegate_lifecycle,
        "unexpected include_delegate_lifecycle flag"
    );
}

#[test]
fn cli_sessions_help_mentions_operator_facing_session_shell() {
    let help = render_cli_help(["sessions"]);

    assert!(
        help.contains("operator-facing session shell"),
        "sessions help should explain the operator-facing shell intent: {help}"
    );
    assert!(
        help.contains("history"),
        "sessions help should surface transcript inspection: {help}"
    );
    assert!(
        help.contains("recover"),
        "sessions help should surface recovery actions: {help}"
    );
    assert!(
        help.contains("heal"),
        "sessions help should surface self-heal actions: {help}"
    );
}

#[tokio::test]
async fn execute_sessions_command_list_returns_visible_sessions_with_workflow_metadata() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-list");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "delegate:session-1".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("ops-root".to_owned()),
        label: Some("Release Research".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "delegate:session-1".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("ops-root".to_owned()),
        payload_json: json!({
            "task": "research release readiness",
            "label": "Release Research",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 60,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["file.read"],
                "workspace_root": "/tmp/loong/sessions-cli/delegate-session-1",
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
        }),
    })
    .expect("append queued event");

    let execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::List {
                limit: 20,
                state: None,
                kind: Some("delegate_child".to_owned()),
                parent_session_id: None,
                overdue_only: false,
                include_archived: false,
                include_delegate_lifecycle: false,
            },
        },
    )
    .await
    .expect("sessions list should succeed");

    assert_eq!(execution.payload["command"], "list");
    assert_eq!(execution.payload["matched_count"], 1);
    assert_eq!(execution.payload["returned_count"], 1);
    assert_eq!(
        execution.payload["sessions"][0]["workflow"]["task"],
        "research release readiness"
    );
    assert_eq!(
        execution.payload["sessions"][0]["workflow"]["workflow_id"],
        "ops-root"
    );
    assert_eq!(
        execution.payload["sessions"][0]["workflow"]["phase"],
        "execute"
    );
    assert_eq!(
        execution.payload["sessions"][0]["workflow"]["binding"]["mode"],
        "advisory_only"
    );

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions list");
    assert!(
        rendered.contains("task=research release readiness"),
        "list render should surface workflow task: {rendered}"
    );
    assert!(
        rendered.contains("workflow_phase=execute"),
        "list render should surface workflow phase: {rendered}"
    );
}

#[tokio::test]
async fn execute_sessions_command_status_surfaces_workflow_recipes_and_rendered_summary() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-status");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "delegate:session-1".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("ops-root".to_owned()),
        label: Some("Continuity Child".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "delegate:session-1".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("ops-root".to_owned()),
        payload_json: json!({
            "task": "research continuity",
            "label": "Continuity Child",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 90,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["file.read"],
                "workspace_root": "/tmp/loong/sessions-cli/delegate-session-1",
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
    .expect("append started event");
    mvp::memory::append_turn_direct(
        "delegate:session-1",
        "user",
        "hello",
        &mvp::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(root.path().join("memory.sqlite3")),
            ..mvp::memory::runtime_config::MemoryRuntimeConfig::default()
        },
    )
    .expect("append user turn");
    mvp::memory::append_turn_direct(
        "delegate:session-1",
        "assistant",
        "world",
        &mvp::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(root.path().join("memory.sqlite3")),
            ..mvp::memory::runtime_config::MemoryRuntimeConfig::default()
        },
    )
    .expect("append assistant turn");
    let prompt_frame_event = json!({
        "type": "conversation_event",
        "event": "provider_prompt_frame_snapshot",
        "payload": {
            "provider_round": 1,
            "phase": "initial",
            "prompt_frame": {
                "schema_version": 1,
                "total_estimated_tokens": 42,
                "stable_runtime_segment_count": 1,
                "stable_runtime_estimated_tokens": 12,
                "session_latched_segment_count": 1,
                "session_latched_estimated_tokens": 8,
                "advisory_profile_segment_count": 1,
                "advisory_profile_estimated_tokens": 6,
                "session_local_recall_segment_count": 1,
                "session_local_recall_estimated_tokens": 5,
                "recent_window_segment_count": 1,
                "recent_window_estimated_tokens": 7,
                "turn_ephemeral_segment_count": 0,
                "turn_ephemeral_estimated_tokens": 0,
                "stable_runtime_hash": "stable-a",
                "session_latched_hash": "latched-a",
                "stable_prefix_hash_sha256": "prefix-a",
                "cached_prefix_sha256": "cached-a",
                "advisory_profile_hash": "profile-a",
                "session_local_recall_hash": "recall-a",
                "recent_window_hash": "window-a",
                "turn_ephemeral_hash": null
            }
        }
    });
    append_session_conversation_event(&root, "delegate:session-1", prompt_frame_event);
    append_session_conversation_event(
        &root,
        "delegate:session-1",
        json!({
            "type": "conversation_event",
            "event": "plan_round_started",
            "payload": {
                "metrics": {
                    "rounds_started": 1,
                    "rounds_succeeded": 0,
                    "rounds_failed": 0,
                    "verify_failures": 0,
                    "replans_triggered": 0,
                    "total_attempts_used": 1
                },
                "health_signal": {
                    "severity": "warn",
                    "flags": ["warmup"]
                }
            }
        }),
    );
    append_session_conversation_event(
        &root,
        "delegate:session-1",
        json!({
            "type": "conversation_event",
            "event": "final_status",
            "payload": {
                "status": "succeeded",
                "route_decision": "return",
                "route_reason": "completed",
                "metrics": {
                    "rounds_started": 1,
                    "rounds_succeeded": 1,
                    "rounds_failed": 0,
                    "verify_failures": 0,
                    "replans_triggered": 0,
                    "total_attempts_used": 1
                },
                "health_signal": {
                    "severity": "warn",
                    "flags": ["warmup"]
                }
            }
        }),
    );
    append_session_conversation_event(
        &root,
        "delegate:session-1",
        json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalized",
                "checkpoint": {
                    "lane": {
                        "lane": "safe",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success"
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "completed"
                },
                "compaction_diagnostics": {
                    "summary_turn_count": 6,
                    "retained_turn_count": 3,
                    "demoted_recent_turn_count": 1,
                    "total_turns": 9,
                    "assistant_turns": 4,
                    "low_signal_turns": 2,
                    "tool_result_line_prunes": 1,
                    "tool_outcome_record_prunes": 0
                },
                "failure": null
            }
        }),
    );

    let execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Status {
                session_id: "delegate:session-1".to_owned(),
            },
        },
    )
    .await
    .expect("sessions status should succeed");

    assert_eq!(execution.payload["command"], "status");
    assert_eq!(
        execution.payload["detail"]["workflow"]["task"],
        "research continuity"
    );
    assert_eq!(
        execution.payload["detail"]["workflow"]["workflow_id"],
        "ops-root"
    );
    assert_eq!(execution.payload["detail"]["workflow"]["phase"], "execute");
    assert_eq!(
        execution.payload["detail"]["workflow"]["lineage_root_session_id"],
        "ops-root"
    );
    assert_eq!(
        execution.payload["detail"]["workflow"]["binding"]["execution_surface"],
        "delegate.async"
    );
    assert_eq!(
        execution.payload["detail"]["workflow"]["binding"]["worktree"]["worktree_id"],
        "delegate:session-1"
    );
    assert_eq!(execution.payload["detail"]["session"]["turn_count"], 6);
    assert_eq!(
        execution.payload["detail"]["prompt_frame"]["summary"]["latest_phase"],
        "initial"
    );
    assert_eq!(
        execution.payload["detail"]["prompt_frame"]["summary"]["latest_total_estimated_tokens"],
        42
    );
    assert_eq!(
        execution.payload["detail"]["safe_lane"]["summary"]["final_status"],
        "succeeded"
    );
    assert_eq!(
        execution.payload["detail"]["safe_lane"]["summary"]["round_started_events"],
        1
    );
    assert_eq!(
        execution.payload["detail"]["turn_checkpoint"]["summary"]["session_state"],
        "finalized"
    );
    assert_eq!(
        execution.payload["detail"]["turn_checkpoint"]["summary"]["checkpoint_durable"],
        true
    );
    let recipes = execution.payload["recipes"]
        .as_array()
        .expect("recipes array");
    let recipe_values = recipes
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        recipe_values
            .iter()
            .any(|recipe| recipe.contains("sessions status")),
        "expected status recipe in {recipe_values:?}"
    );
    assert!(
        recipe_values
            .iter()
            .any(|recipe| recipe.contains("sessions history")),
        "expected history recipe in {recipe_values:?}"
    );
    assert!(
        recipe_values
            .iter()
            .any(|recipe| recipe.contains("sessions wait")),
        "expected wait recipe in {recipe_values:?}"
    );
    assert!(
        recipe_values
            .iter()
            .any(|recipe| recipe.contains("sessions events")),
        "expected events recipe in {recipe_values:?}"
    );

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions status");
    assert!(
        rendered.contains("workflow_id: ops-root"),
        "status render should surface workflow id: {rendered}"
    );
    assert!(
        rendered.contains("workflow_phase: execute"),
        "status render should surface workflow phase: {rendered}"
    );
    assert!(
        rendered.contains("task: research continuity"),
        "status render should surface workflow task: {rendered}"
    );
    assert!(
        rendered.contains("lineage_root_session_id: ops-root"),
        "status render should surface lineage root: {rendered}"
    );
    assert!(
        rendered.contains("workflow_binding_mode: advisory_only"),
        "status render should surface workflow binding mode: {rendered}"
    );
    assert!(
        rendered.contains("workflow_worktree_id: delegate:session-1"),
        "status render should surface workflow worktree id: {rendered}"
    );
    assert!(
        rendered.contains("runtime_self_continuity: present"),
        "status render should surface continuity summary: {rendered}"
    );
    assert!(
        rendered.contains("prompt_frame: phase=initial total_tokens=42"),
        "status render should surface prompt-frame summary: {rendered}"
    );
    assert!(
        rendered.contains("stable_prefix=prefix-a"),
        "status render should surface prompt-frame stable prefix hash: {rendered}"
    );
    assert!(
        rendered.contains("safe_lane: status=succeeded rounds_started=1"),
        "status render should surface safe-lane summary: {rendered}"
    );
    assert!(
        rendered.contains("health=warn"),
        "status render should surface safe-lane health: {rendered}"
    );
    assert!(
        rendered.contains("turn_checkpoint: session_state=finalized durable=yes"),
        "status render should surface turn-checkpoint durability: {rendered}"
    );
    assert!(
        rendered.contains("stage=finalized after_turn=completed"),
        "status render should surface turn-checkpoint stage detail: {rendered}"
    );
    assert!(
        rendered.contains("compaction=completed"),
        "status render should surface turn-checkpoint compaction detail: {rendered}"
    );
    assert!(
        rendered.contains("compaction_diag=summary:6 retained:3 demoted:1"),
        "status render should surface compact diagnostics detail: {rendered}"
    );
}

#[tokio::test]
async fn execute_sessions_command_heal_plans_overdue_session_recovery() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-heal-plan");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    create_delegate_session(
        &repo,
        "ops-root",
        "delegate:session-1",
        "Recovery Child",
        mvp::session::repository::SessionState::Ready,
    );
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "delegate:session-1".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("ops-root".to_owned()),
        payload_json: json!({
            "task": "recover the child",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    overwrite_session_event_ts(
        &root,
        "delegate:session-1",
        "delegate_queued",
        current_unix_ts() - 90,
    );

    let execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Heal {
                session_id: "delegate:session-1".to_owned(),
                apply: false,
            },
        },
    )
    .await
    .expect("sessions heal plan should succeed");

    assert_eq!(execution.payload["command"], "heal");
    assert_eq!(execution.payload["apply"], false);
    assert_eq!(execution.payload["plan"]["action_count"], 1);
    assert_eq!(
        execution.payload["plan"]["actions"][0]["tool_name"],
        "session_recover"
    );
    assert_eq!(execution.payload["plan"]["actions"][0]["can_apply"], true);

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions heal plan");
    assert!(
        rendered.contains("recommended_action: tool=session_recover"),
        "heal render should surface recommended action: {rendered}"
    );
    assert!(
        rendered.contains("self-heal plan"),
        "heal render should surface self-heal section: {rendered}"
    );
}

#[tokio::test]
async fn execute_sessions_command_heal_apply_runs_session_recover() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-heal-apply");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    create_delegate_session(
        &repo,
        "ops-root",
        "delegate:session-1",
        "Recovery Child",
        mvp::session::repository::SessionState::Ready,
    );
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "delegate:session-1".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("ops-root".to_owned()),
        payload_json: json!({
            "task": "recover the child",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    overwrite_session_event_ts(
        &root,
        "delegate:session-1",
        "delegate_queued",
        current_unix_ts() - 90,
    );

    let execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Heal {
                session_id: "delegate:session-1".to_owned(),
                apply: true,
            },
        },
    )
    .await
    .expect("sessions heal apply should succeed");

    assert_eq!(execution.payload["command"], "heal");
    assert_eq!(execution.payload["apply"], true);
    assert_eq!(
        execution.payload["applied_actions"][0]["tool_name"],
        "session_recover"
    );
    assert_eq!(execution.payload["detail"]["session"]["state"], "failed");
    assert_eq!(
        execution.payload["detail"]["terminal_outcome_state"],
        "present"
    );

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions heal apply");
    assert!(
        rendered.contains("applied actions"),
        "heal apply render should surface applied actions: {rendered}"
    );
    assert!(
        rendered.contains("status=applied"),
        "heal apply render should surface applied action status: {rendered}"
    );
}

#[tokio::test]
async fn execute_sessions_command_events_history_and_wait_surface_incremental_payloads() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-events");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    create_delegate_session(
        &repo,
        "ops-root",
        "delegate:session-1",
        "Release Research",
        mvp::session::repository::SessionState::Running,
    );
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "delegate:session-1".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("ops-root".to_owned()),
        payload_json: json!({
            "task": "research release readiness",
            "label": "Release Research",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 60,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["file.read"],
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
        }),
    })
    .expect("append delegate_started event");
    append_session_turn(&root, "delegate:session-1", "user", "hello");
    append_session_turn(&root, "delegate:session-1", "assistant", "world");

    let events_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Events {
                session_id: "delegate:session-1".to_owned(),
                after_id: None,
                limit: 20,
            },
        },
    )
    .await
    .expect("sessions events should succeed");

    let next_after_id = events_execution.payload["next_after_id"]
        .as_i64()
        .expect("next_after_id");
    let rendered_events = loong_daemon::sessions_cli::render_sessions_cli_text(&events_execution)
        .expect("render sessions events");

    assert_eq!(events_execution.payload["command"], "events");
    assert_eq!(events_execution.payload["session_id"], "delegate:session-1");
    assert_eq!(
        events_execution.payload["events"][0]["event_kind"],
        "delegate_started"
    );
    assert!(next_after_id >= 1);
    assert!(
        rendered_events.contains("delegate_started"),
        "events render should surface event kind: {rendered_events}"
    );

    let history_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::History {
                session_id: "delegate:session-1".to_owned(),
                limit: 20,
            },
        },
    )
    .await
    .expect("sessions history should succeed");

    let rendered_history = loong_daemon::sessions_cli::render_sessions_cli_text(&history_execution)
        .expect("render sessions history");

    assert_eq!(history_execution.payload["command"], "history");
    assert_eq!(
        history_execution.payload["session_id"],
        "delegate:session-1"
    );
    assert_eq!(history_execution.payload["turns"][0]["role"], "user");
    assert_eq!(history_execution.payload["turns"][1]["role"], "assistant");
    assert!(
        rendered_history.contains("user: hello"),
        "history render should surface transcript turns: {rendered_history}"
    );

    let wait_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Wait {
                session_id: "delegate:session-1".to_owned(),
                after_id: Some(next_after_id),
                timeout_ms: 1,
            },
        },
    )
    .await
    .expect("sessions wait should succeed");

    let rendered_wait = loong_daemon::sessions_cli::render_sessions_cli_text(&wait_execution)
        .expect("render sessions wait");

    assert_eq!(wait_execution.payload["command"], "wait");
    assert_eq!(wait_execution.payload["session_id"], "delegate:session-1");
    assert_eq!(wait_execution.payload["wait_status"], "timeout");
    assert_eq!(wait_execution.payload["detail"]["events"], json!([]));
    assert_eq!(
        wait_execution.payload["detail"]["session"]["session_id"],
        "delegate:session-1"
    );
    assert!(
        rendered_wait.contains("wait result: timeout"),
        "wait render should surface timeout result: {rendered_wait}"
    );
}

#[tokio::test]
async fn execute_sessions_tree_commands_surface_head_modes_paths_and_artifacts() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-tree");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    append_session_turn(&root, "ops-root", "user", "hello");
    append_session_turn(&root, "ops-root", "assistant", "world");

    let fork_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::ForkHead {
                session_id: "ops-root".to_owned(),
                node_id: "session-turn:ops-root:1".to_owned(),
                head_name: "thread/alpha".to_owned(),
            },
        },
    )
    .await
    .expect("fork head should succeed");

    assert_eq!(fork_execution.payload["command"], "fork-head");
    assert_eq!(fork_execution.payload["detail"]["head"]["mode"], "live");
    let rendered_fork = loong_daemon::sessions_cli::render_sessions_cli_text(&fork_execution)
        .expect("render fork head");
    assert!(
        rendered_fork.contains("head: thread/alpha -> session-turn:ops-root:1 mode=live"),
        "fork render should expose head mode: {rendered_fork}"
    );

    let pin_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::PinHead {
                session_id: "ops-root".to_owned(),
                head_name: "thread/alpha".to_owned(),
            },
        },
    )
    .await
    .expect("pin head should succeed");

    assert_eq!(pin_execution.payload["command"], "pin-head");
    assert_eq!(pin_execution.payload["detail"]["head"]["mode"], "pinned");

    let heads_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Heads {
                session_id: "ops-root".to_owned(),
            },
        },
    )
    .await
    .expect("heads should succeed");

    assert_eq!(heads_execution.payload["command"], "heads");
    assert_eq!(
        heads_execution.payload["detail"]["heads"][0]["head_name"],
        "active"
    );
    assert_eq!(
        heads_execution.payload["detail"]["heads"][0]["mode"],
        "live"
    );
    assert_eq!(
        heads_execution.payload["detail"]["heads"][1]["head_name"],
        "thread/alpha"
    );
    assert_eq!(
        heads_execution.payload["detail"]["heads"][1]["mode"],
        "pinned"
    );
    let rendered_heads = loong_daemon::sessions_cli::render_sessions_cli_text(&heads_execution)
        .expect("render heads");
    assert!(
        rendered_heads.contains("thread/alpha -> session-turn:ops-root:1 mode=pinned"),
        "heads render should expose pinned mode: {rendered_heads}"
    );

    let path_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Path {
                session_id: "ops-root".to_owned(),
                head_name: Some("thread/alpha".to_owned()),
            },
        },
    )
    .await
    .expect("path should succeed");

    assert_eq!(path_execution.payload["command"], "path");
    assert_eq!(path_execution.payload["detail"]["node_count"], 2);
    assert_eq!(
        path_execution.payload["detail"]["path"][1]["content"],
        "hello"
    );

    let checkpoint_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Checkpoint {
                session_id: "ops-root".to_owned(),
                label: "draft-a".to_owned(),
                node_id: None,
            },
        },
    )
    .await
    .expect("checkpoint should succeed");

    assert_eq!(checkpoint_execution.payload["command"], "checkpoint");
    assert_eq!(
        checkpoint_execution.payload["detail"]["head"]["head_name"],
        "checkpoint/draft-a"
    );
    assert_eq!(
        checkpoint_execution.payload["detail"]["head"]["mode"],
        "pinned"
    );
    let rendered_checkpoint =
        loong_daemon::sessions_cli::render_sessions_cli_text(&checkpoint_execution)
            .expect("render checkpoint");
    assert!(
        rendered_checkpoint.contains("head: checkpoint/draft-a"),
        "checkpoint render should include checkpoint head: {rendered_checkpoint}"
    );
    assert!(
        rendered_checkpoint.contains("mode=pinned"),
        "checkpoint render should include pinned mode: {rendered_checkpoint}"
    );

    let artifacts_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Artifacts {
                session_id: "ops-root".to_owned(),
            },
        },
    )
    .await
    .expect("artifacts should succeed");

    assert_eq!(artifacts_execution.payload["command"], "artifacts");
    assert_eq!(
        artifacts_execution.payload["detail"]["artifacts"][0]["kind"],
        "checkpoint"
    );
    assert_eq!(
        artifacts_execution.payload["detail"]["artifacts"][0]["summary_text"],
        "draft-a"
    );

    let unpin_execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::UnpinHead {
                session_id: "ops-root".to_owned(),
                head_name: "thread/alpha".to_owned(),
            },
        },
    )
    .await
    .expect("unpin head should succeed");

    assert_eq!(unpin_execution.payload["command"], "unpin-head");
    assert_eq!(unpin_execution.payload["detail"]["head"]["mode"], "live");
}

#[tokio::test]
async fn execute_sessions_command_cancel_dry_run_surfaces_cancel_action() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-cancel");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    create_delegate_session(
        &repo,
        "ops-root",
        "delegate:session-1",
        "Release Check",
        mvp::session::repository::SessionState::Ready,
    );
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "delegate:session-1".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("ops-root".to_owned()),
        payload_json: json!({
            "task": "check release readiness",
            "label": "Release Check",
            "timeout_seconds": 60
        }),
    })
    .expect("append delegate_queued event");

    let execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Cancel {
                session_id: "delegate:session-1".to_owned(),
                dry_run: true,
            },
        },
    )
    .await
    .expect("sessions cancel dry run should succeed");

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions cancel");

    assert_eq!(execution.payload["command"], "cancel");
    assert_eq!(execution.payload["dry_run"], true);
    assert_eq!(execution.payload["result"], "would_apply");
    assert_eq!(
        execution.payload["action"]["kind"],
        "queued_async_cancelled"
    );
    assert!(
        rendered.contains("queued_async_cancelled"),
        "cancel render should surface cancel action: {rendered}"
    );
}

#[tokio::test]
async fn execute_sessions_command_recover_dry_run_surfaces_non_recoverable_result() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-recover");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    create_delegate_session(
        &repo,
        "ops-root",
        "delegate:session-1",
        "Release Check",
        mvp::session::repository::SessionState::Ready,
    );
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "delegate:session-1".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("ops-root".to_owned()),
        payload_json: json!({
            "task": "check release readiness",
            "label": "Release Check",
            "timeout_seconds": 60
        }),
    })
    .expect("append delegate_queued event");

    let execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Recover {
                session_id: "delegate:session-1".to_owned(),
                dry_run: true,
            },
        },
    )
    .await
    .expect("sessions recover dry run should succeed");

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions recover");

    assert_eq!(execution.payload["command"], "recover");
    assert_eq!(execution.payload["dry_run"], true);
    assert_eq!(execution.payload["result"], "skipped_not_recoverable");
    assert!(
        execution.payload["message"]
            .as_str()
            .expect("recover message")
            .contains("session_recover_not_recoverable"),
        "recover dry run should surface root cause: {:?}",
        execution.payload
    );
    assert!(
        rendered.contains("skipped_not_recoverable"),
        "recover render should surface non-recoverable result: {rendered}"
    );
}

#[tokio::test]
async fn execute_sessions_command_archive_dry_run_surfaces_archive_action() {
    let root = super::tasks_cli::TempDirGuard::new("loong-sessions-cli-archive");
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[]);
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    create_delegate_session(
        &repo,
        "ops-root",
        "delegate:session-1",
        "Release Archive",
        mvp::session::repository::SessionState::Running,
    );
    repo.finalize_session_terminal(
        "delegate:session-1",
        mvp::session::repository::FinalizeSessionTerminalRequest {
            state: mvp::session::repository::SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("ops-root".to_owned()),
            event_payload_json: json!({
                "result": "ok"
            }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "delegate:session-1",
                "result": "ok"
            }),
            frozen_result: None,
        },
    )
    .expect("finalize child session");

    let execution = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Archive {
                session_id: "delegate:session-1".to_owned(),
                dry_run: true,
            },
        },
    )
    .await
    .expect("sessions archive dry run should succeed");

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions archive");

    assert_eq!(execution.payload["command"], "archive");
    assert_eq!(execution.payload["dry_run"], true);
    assert_eq!(execution.payload["result"], "would_apply");
    assert_eq!(execution.payload["action"]["kind"], "session_archived");
    assert_eq!(
        execution.payload["inspection"]["session"]["state"],
        "completed"
    );
    assert!(
        rendered.contains("session_archived"),
        "archive render should surface archive action: {rendered}"
    );
}

#[test]
fn render_sessions_status_text_escapes_control_characters() {
    let execution = loong_daemon::sessions_cli::SessionsCommandExecution {
        resolved_config_path: "/tmp/loong.toml".to_owned(),
        current_session_id: "ops-root".to_owned(),
        payload: json!({
            "command": "status",
            "detail": {
                "session": {
                    "session_id": "delegate:\u{1b}[31mchild",
                    "kind": "delegate_child",
                    "state": "running",
                    "parent_session_id": "ops-root\nnext",
                    "label": "line1\nline2",
                    "turn_count": 2,
                    "last_turn_at": 123,
                    "last_error": "boom\u{1b}[0m"
                },
                "workflow": {
                    "task": "research\ncontinuity",
                    "lineage_root_session_id": "ops-root",
                    "lineage_depth": 1,
                    "runtime_self_continuity": {
                        "present": true,
                        "resolved_identity_present": true,
                        "session_profile_projection_present": false
                    }
                },
                "delegate_lifecycle": {
                    "mode": "async",
                    "phase": "running",
                    "timeout_seconds": 90
                },
                "terminal_outcome_state": "present",
                "terminal_outcome": {
                    "status": "ok"
                },
                "recent_events": []
            },
            "recipes": [],
            "next_steps": []
        }),
    };

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions status");

    assert!(
        !rendered.contains('\u{1b}'),
        "rendered status should not contain raw escape characters: {rendered:?}"
    );
    assert!(
        rendered.contains("label: line1\\nline2"),
        "expected escaped newlines in label: {rendered}"
    );
    assert!(
        rendered.contains("last_error: boom\\u{1b}[0m"),
        "expected escaped escape sequence in last_error: {rendered}"
    );
    assert!(
        rendered.contains("task: research\\ncontinuity"),
        "expected escaped newlines in task: {rendered}"
    );
}

#[test]
fn render_sessions_history_text_escapes_control_characters() {
    let execution = loong_daemon::sessions_cli::SessionsCommandExecution {
        resolved_config_path: "/tmp/loong.toml".to_owned(),
        current_session_id: "ops-root".to_owned(),
        payload: json!({
            "command": "history",
            "session_id": "delegate:\u{1b}[31mchild",
            "limit": 20,
            "turns": [
                {
                    "role": "assistant",
                    "content": "hello\nworld\u{1b}[0m"
                }
            ]
        }),
    };

    let rendered = loong_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions history");

    assert!(
        !rendered.contains('\u{1b}'),
        "rendered history should not contain raw escape characters: {rendered:?}"
    );
    assert!(
        rendered.contains("history for `delegate:\\u{1b}[31mchild`"),
        "expected escaped session id in history header: {rendered}"
    );
    assert!(
        rendered.contains("- assistant: hello\\nworld\\u{1b}[0m"),
        "expected escaped turn content in history output: {rendered}"
    );
}
