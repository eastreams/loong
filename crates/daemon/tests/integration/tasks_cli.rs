#![allow(unsafe_code)]

use super::*;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::MutexGuard,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();
    let canonical_temp_dir = dunce::canonicalize(&temp_dir).unwrap_or(temp_dir);
    canonical_temp_dir.join(format!("{prefix}-{nanos}"))
}

struct TasksCliEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<OsString>)>,
}

const TASKS_RUNTIME_ENV_KEYS: &[&str] = &[
    "LOONGCLAW_BROWSER_COMPANION_COMMAND",
    "LOONGCLAW_BROWSER_COMPANION_ENABLED",
    "LOONGCLAW_BROWSER_COMPANION_EXPECTED_VERSION",
    "LOONGCLAW_BROWSER_COMPANION_TIMEOUT_SECONDS",
    "LOONGCLAW_BROWSER_ENABLED",
    "LOONGCLAW_BROWSER_MAX_LINKS",
    "LOONGCLAW_BROWSER_MAX_SESSIONS",
    "LOONGCLAW_BROWSER_MAX_TEXT_CHARS",
    "LOONGCLAW_CONFIG_PATH",
    "LOONGCLAW_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
    "LOONGCLAW_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED",
    "LOONGCLAW_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
    "LOONGCLAW_EXTERNAL_SKILLS_ENABLED",
    "LOONGCLAW_EXTERNAL_SKILLS_INSTALL_ROOT",
    "LOONGCLAW_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
    "LOONGCLAW_FILE_ROOT",
    "LOONGCLAW_MEMORY_BACKEND",
    "LOONGCLAW_MEMORY_PROFILE",
    "LOONGCLAW_MEMORY_PROFILE_NOTE",
    "LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS",
    "LOONGCLAW_SHELL_ALLOWLIST",
    "LOONGCLAW_SHELL_DEFAULT_MODE",
    "LOONGCLAW_SHELL_DENY",
    "LOONGCLAW_SLIDING_WINDOW",
    "LOONGCLAW_SQLITE_PATH",
    "LOONGCLAW_TOOL_DELEGATE_ENABLED",
    "LOONGCLAW_TOOL_MESSAGES_ENABLED",
    "LOONGCLAW_TOOL_SESSIONS_ALLOW_MUTATION",
    "LOONGCLAW_TOOL_SESSIONS_ENABLED",
    "LOONGCLAW_WEB_FETCH_ALLOWED_DOMAINS",
    "LOONGCLAW_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
    "LOONGCLAW_WEB_FETCH_BLOCKED_DOMAINS",
    "LOONGCLAW_WEB_FETCH_ENABLED",
    "LOONGCLAW_WEB_FETCH_MAX_BYTES",
    "LOONGCLAW_WEB_FETCH_MAX_REDIRECTS",
    "LOONGCLAW_WEB_FETCH_TIMEOUT_SECONDS",
];

impl TasksCliEnvironmentGuard {
    fn set(pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = super::lock_daemon_test_environment();
        let mut saved = Vec::new();
        for key in TASKS_RUNTIME_ENV_KEYS {
            saved.push(((*key).to_owned(), std::env::var_os(key)));
        }
        for (key, value) in pairs {
            let already_saved = saved.iter().any(|(saved_key, _)| saved_key == key);
            if !already_saved {
                saved.push(((*key).to_owned(), std::env::var_os(key)));
            }
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for TasksCliEnvironmentGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(&key, value);
                },
                None => unsafe {
                    std::env::remove_var(&key);
                },
            }
        }
    }
}

fn write_tasks_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");
    let config_path = root.join("loongclaw.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    config.tools.file_root = Some(root.display().to_string());
    config.tools.sessions.allow_mutation = true;
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    config_path
}

fn seed_background_task(config_path: &Path, root_session_id: &str, task_id: &str) {
    let config = mvp::config::load(Some(config_path.to_string_lossy().as_ref()))
        .expect("load config")
        .1;
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: root_session_id.to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Ops Root".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: task_id.to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some(root_session_id.to_owned()),
        label: Some("Release Check".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: task_id.to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some(root_session_id.to_owned()),
        payload_json: json!({
            "task": "check release readiness",
            "label": "Release Check",
            "timeout_seconds": 60,
        }),
    })
    .expect("append delegate_queued event");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-task-1".to_owned(),
        session_id: task_id.to_owned(),
        turn_id: "turn-task-1".to_owned(),
        tool_call_id: "call-task-1".to_owned(),
        tool_name: "delegate_async".to_owned(),
        approval_key: "tool:delegate_async".to_owned(),
        request_payload_json: json!({
            "tool_name": "delegate_async",
        }),
        governance_snapshot_json: json!({
            "reason": "operator approval required",
            "rule_id": "test_delegate_async",
        }),
    })
    .expect("create approval request");
    repo.upsert_session_tool_policy(mvp::session::repository::NewSessionToolPolicyRecord {
        session_id: task_id.to_owned(),
        requested_tool_ids: vec!["file.read".to_owned()],
        runtime_narrowing: mvp::tools::runtime_config::ToolRuntimeNarrowing::default(),
    })
    .expect("upsert session tool policy");
}

fn load_session_repository(config_path: &Path) -> mvp::session::repository::SessionRepository {
    let loaded =
        mvp::config::load(Some(config_path.to_string_lossy().as_ref())).expect("load config");
    let config = loaded.1;
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    mvp::session::repository::SessionRepository::new(&memory_config).expect("session repository")
}

#[test]
fn tasks_create_cli_parses_global_flags_after_subcommand() {
    let cli = try_parse_cli([
        "loongclaw",
        "tasks",
        "create",
        "research release status",
        "--label",
        "release-scan",
        "--timeout-seconds",
        "90",
        "--session",
        "ops-root",
        "--json",
        "--config",
        "/tmp/loongclaw.toml",
    ])
    .expect("tasks create CLI should parse");

    match cli.command {
        Some(Commands::Tasks {
            config,
            json,
            session,
            command,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loongclaw.toml"));
            assert!(json);
            assert_eq!(session, "ops-root");
            match command {
                loongclaw_daemon::tasks_cli::TasksCommands::Create {
                    task,
                    label,
                    timeout_seconds,
                } => {
                    assert_eq!(task, "research release status");
                    assert_eq!(label.as_deref(), Some("release-scan"));
                    assert_eq!(timeout_seconds, Some(90));
                }
                loongclaw_daemon::tasks_cli::TasksCommands::List { .. } => {
                    panic!("unexpected tasks subcommand parsed: List")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Status { .. } => {
                    panic!("unexpected tasks subcommand parsed: Status")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Events { .. } => {
                    panic!("unexpected tasks subcommand parsed: Events")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Wait { .. } => {
                    panic!("unexpected tasks subcommand parsed: Wait")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Cancel { .. } => {
                    panic!("unexpected tasks subcommand parsed: Cancel")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Recover { .. } => {
                    panic!("unexpected tasks subcommand parsed: Recover")
                }
            }
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn tasks_wait_cli_parses_session_and_timeout_flags() {
    let cli = try_parse_cli([
        "loongclaw",
        "tasks",
        "wait",
        "delegate:abc123",
        "--after-id",
        "10",
        "--timeout-ms",
        "2500",
        "--session",
        "ops-root",
        "--json",
        "--config",
        "/tmp/loongclaw.toml",
    ])
    .expect("tasks wait CLI should parse");

    match cli.command {
        Some(Commands::Tasks {
            config,
            json,
            session,
            command,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loongclaw.toml"));
            assert!(json);
            assert_eq!(session, "ops-root");
            match command {
                loongclaw_daemon::tasks_cli::TasksCommands::Wait {
                    task_id,
                    after_id,
                    timeout_ms,
                } => {
                    assert_eq!(task_id, "delegate:abc123");
                    assert_eq!(after_id, Some(10));
                    assert_eq!(timeout_ms, 2500);
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Create { .. } => {
                    panic!("unexpected tasks subcommand parsed: Create")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::List { .. } => {
                    panic!("unexpected tasks subcommand parsed: List")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Status { .. } => {
                    panic!("unexpected tasks subcommand parsed: Status")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Events { .. } => {
                    panic!("unexpected tasks subcommand parsed: Events")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Cancel { .. } => {
                    panic!("unexpected tasks subcommand parsed: Cancel")
                }
                loongclaw_daemon::tasks_cli::TasksCommands::Recover { .. } => {
                    panic!("unexpected tasks subcommand parsed: Recover")
                }
            }
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[tokio::test]
async fn execute_tasks_command_list_returns_visible_background_tasks() {
    let root = unique_temp_dir("loongclaw-tasks-cli-list");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(&root);
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::List {
                limit: 20,
                state: None,
                overdue_only: false,
                include_archived: false,
            },
        },
    )
    .await
    .expect("tasks list should succeed");

    assert_eq!(execution.payload["command"], "list");
    assert_eq!(execution.payload["matched_count"], 1);
    assert_eq!(execution.payload["returned_count"], 1);
    assert_eq!(execution.payload["tasks"][0]["task_id"], "delegate:task-1");
    assert_eq!(execution.payload["tasks"][0]["phase"], "queued");

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_tasks_command_status_surfaces_approval_and_tool_policy() {
    let root = unique_temp_dir("loongclaw-tasks-cli-status");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(&root);
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::Status {
                task_id: "delegate:task-1".to_owned(),
            },
        },
    )
    .await
    .expect("tasks status should succeed");

    assert_eq!(execution.payload["command"], "status");
    assert_eq!(execution.payload["task"]["task_id"], "delegate:task-1");
    assert_eq!(execution.payload["task"]["approval"]["matched_count"], 1);
    assert_eq!(
        execution.payload["task"]["tool_policy"]["effective_tool_ids"][0],
        "file.read"
    );

    let rendered = loongclaw_daemon::tasks_cli::render_tasks_cli_text(&execution)
        .expect("render tasks status");
    assert!(
        rendered.contains("approval_requests: 1"),
        "status render should surface approval count: {rendered}"
    );
    assert!(
        rendered.contains("effective_tool_ids: file.read"),
        "status render should surface effective tool ids: {rendered}"
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_tasks_command_create_queues_background_task_and_surfaces_follow_up_recipes() {
    let root = unique_temp_dir("loongclaw-tasks-cli-create");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(&root);

    let execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::Create {
                task: "research release readiness".to_owned(),
                label: Some("Release Check".to_owned()),
                timeout_seconds: Some(45),
            },
        },
    )
    .await
    .expect("tasks create should succeed");

    let task_id = execution.payload["task"]["task_id"]
        .as_str()
        .expect("task id");
    let recipes = execution.payload["recipes"]
        .as_array()
        .expect("recipes array");

    assert_eq!(execution.payload["command"], "create");
    assert_eq!(execution.payload["current_session_id"], "ops-root");
    assert_eq!(execution.payload["task"]["scope_session_id"], "ops-root");
    assert_eq!(execution.payload["task"]["label"], "Release Check");
    assert_eq!(execution.payload["task"]["timeout_seconds"], 45);
    assert_eq!(
        execution.payload["task"]["session"]["kind"],
        "delegate_child"
    );
    assert!(task_id.starts_with("delegate:"));
    assert_eq!(recipes.len(), 3);
    assert!(
        recipes[0]
            .as_str()
            .expect("status recipe")
            .contains("tasks status"),
        "expected status follow-up recipe"
    );
    assert_eq!(
        execution.payload["next_steps"]
            .as_array()
            .expect("next steps array")
            .len(),
        3
    );

    let repo = load_session_repository(&config_path);
    let root_session = repo
        .load_session("ops-root")
        .expect("load root session")
        .expect("root session");
    let child_session = repo
        .load_session(task_id)
        .expect("load child session")
        .expect("child session");

    assert_eq!(
        root_session.kind,
        mvp::session::repository::SessionKind::Root
    );
    assert_eq!(child_session.parent_session_id.as_deref(), Some("ops-root"));
    assert_eq!(
        child_session.kind,
        mvp::session::repository::SessionKind::DelegateChild
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_tasks_command_events_and_wait_surface_incremental_payloads() {
    let root = unique_temp_dir("loongclaw-tasks-cli-events");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(&root);
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let events_execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::Events {
                task_id: "delegate:task-1".to_owned(),
                after_id: None,
                limit: 20,
            },
        },
    )
    .await
    .expect("tasks events should succeed");

    let next_after_id = events_execution.payload["next_after_id"]
        .as_i64()
        .expect("next_after_id");

    assert_eq!(events_execution.payload["command"], "events");
    assert_eq!(events_execution.payload["task_id"], "delegate:task-1");
    assert_eq!(
        events_execution.payload["events"][0]["event_kind"],
        "delegate_queued"
    );
    assert!(next_after_id >= 1);

    let wait_execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::Wait {
                task_id: "delegate:task-1".to_owned(),
                after_id: Some(next_after_id),
                timeout_ms: 1,
            },
        },
    )
    .await
    .expect("tasks wait should succeed");

    assert_eq!(wait_execution.payload["command"], "wait");
    assert_eq!(wait_execution.payload["task_id"], "delegate:task-1");
    assert_eq!(wait_execution.payload["wait_status"], "timeout");
    assert_eq!(wait_execution.payload["events"], json!([]));
    assert_eq!(wait_execution.payload["task"]["task_id"], "delegate:task-1");

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_tasks_command_cancel_dry_run_surfaces_cancel_action() {
    let root = unique_temp_dir("loongclaw-tasks-cli-cancel");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(&root);
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::Cancel {
                task_id: "delegate:task-1".to_owned(),
                dry_run: true,
            },
        },
    )
    .await
    .expect("tasks cancel dry run should succeed");

    assert_eq!(execution.payload["command"], "cancel");
    assert_eq!(execution.payload["dry_run"], true);
    assert_eq!(
        execution.payload["action"]["kind"],
        "queued_async_cancelled"
    );
    assert_eq!(execution.payload["task"]["task_id"], "delegate:task-1");
    assert_eq!(execution.payload["task"]["phase"], "queued");

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_tasks_command_recover_dry_run_surfaces_non_recoverable_result() {
    let root = unique_temp_dir("loongclaw-tasks-cli-recover");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(&root);
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::Recover {
                task_id: "delegate:task-1".to_owned(),
                dry_run: true,
            },
        },
    )
    .await
    .expect("tasks recover dry run should succeed");

    assert_eq!(execution.payload["command"], "recover");
    assert_eq!(execution.payload["dry_run"], true);
    assert_eq!(execution.payload["result"], "skipped_not_recoverable");
    assert!(
        execution.payload["message"]
            .as_str()
            .expect("recover message")
            .contains("session_recover_not_recoverable"),
        "expected recover dry run to surface root cause, got: {:?}",
        execution.payload
    );
    assert!(execution.payload["action"].is_null());

    fs::remove_dir_all(&root).ok();
}
