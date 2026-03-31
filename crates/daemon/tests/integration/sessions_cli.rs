use super::*;

#[test]
fn sessions_list_cli_parses_global_flags_after_subcommand() {
    let cli = try_parse_cli([
        "loongclaw",
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
        "/tmp/loongclaw.toml",
    ])
    .expect("sessions list CLI should parse");

    let rendered = format!("{:?}", cli.command);
    assert!(
        rendered.contains("Sessions"),
        "expected parsed command to contain Sessions variant: {rendered}"
    );
    assert!(
        rendered.contains("delegate_child"),
        "expected parsed command to include delegate_child filter: {rendered}"
    );
    assert!(
        rendered.contains("ops-root"),
        "expected parsed command to include session scope: {rendered}"
    );
    assert!(
        rendered.contains("/tmp/loongclaw.toml"),
        "expected parsed command to include config path: {rendered}"
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
}

#[tokio::test]
async fn execute_sessions_command_list_returns_visible_sessions_with_workflow_metadata() {
    let root = super::tasks_cli::TempDirGuard::new("loongclaw-sessions-cli-list");
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
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
        }),
    })
    .expect("append queued event");

    let execution = loongclaw_daemon::sessions_cli::execute_sessions_command(
        loongclaw_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::sessions_cli::SessionsCommands::List {
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

    let rendered = loongclaw_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions list");
    assert!(
        rendered.contains("task=research release readiness"),
        "list render should surface workflow task: {rendered}"
    );
}

#[tokio::test]
async fn execute_sessions_command_status_surfaces_workflow_recipes_and_rendered_summary() {
    let root = super::tasks_cli::TempDirGuard::new("loongclaw-sessions-cli-status");
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

    let execution = loongclaw_daemon::sessions_cli::execute_sessions_command(
        loongclaw_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::sessions_cli::SessionsCommands::Status {
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
        execution.payload["detail"]["workflow"]["lineage_root_session_id"],
        "ops-root"
    );
    assert_eq!(execution.payload["detail"]["session"]["turn_count"], 2);
    assert_eq!(
        execution.payload["recipes"]
            .as_array()
            .expect("recipes array")
            .len(),
        4
    );

    let rendered = loongclaw_daemon::sessions_cli::render_sessions_cli_text(&execution)
        .expect("render sessions status");
    assert!(
        rendered.contains("task: research continuity"),
        "status render should surface workflow task: {rendered}"
    );
    assert!(
        rendered.contains("lineage_root_session_id: ops-root"),
        "status render should surface lineage root: {rendered}"
    );
    assert!(
        rendered.contains("runtime_self_continuity: present"),
        "status render should surface continuity summary: {rendered}"
    );
}
