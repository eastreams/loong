#![allow(unsafe_code)]

use super::*;
use rusqlite::{Connection, params};
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

pub(super) struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    pub(super) fn new(prefix: &str) -> Self {
        let path = unique_temp_dir(prefix);
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}

pub(super) struct TasksCliEnvironmentGuard {
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
    pub(super) fn set(pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = super::lock_daemon_test_environment();
        Self::set_with_lock(lock, &[], pairs)
    }

    fn set_with_seeded_env(seeded_pairs: &[(&str, &str)], pairs: &[(&str, Option<&str>)]) -> Self {
        let lock = super::lock_daemon_test_environment();
        Self::set_with_lock(lock, seeded_pairs, pairs)
    }

    fn set_with_lock(
        lock: MutexGuard<'static, ()>,
        seeded_pairs: &[(&str, &str)],
        pairs: &[(&str, Option<&str>)],
    ) -> Self {
        let mut saved = Vec::new();
        for (key, value) in seeded_pairs {
            saved.push(((*key).to_owned(), std::env::var_os(key)));
            unsafe {
                std::env::set_var(key, value);
            }
        }
        for key in TASKS_RUNTIME_ENV_KEYS {
            let already_saved = saved.iter().any(|(saved_key, _)| saved_key == key);
            if !already_saved {
                saved.push(((*key).to_owned(), std::env::var_os(key)));
            }
            unsafe {
                std::env::remove_var(key);
            }
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

pub(super) fn write_tasks_config_with(
    root: &Path,
    configure: impl FnOnce(&mut mvp::config::LoongClawConfig),
) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");
    let config_path = root.join("loongclaw.toml");
    let mut config = mvp::config::LoongClawConfig::default();
    config.audit.path = root
        .join("audit")
        .join("events.jsonl")
        .display()
        .to_string();
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    config.tools.file_root = Some(root.display().to_string());
    config.tools.sessions.allow_mutation = true;
    configure(&mut config);
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    config_path
}

pub(super) fn write_tasks_config(root: &Path) -> PathBuf {
    write_tasks_config_with(root, |_| {})
}

fn seed_background_task(config_path: &Path, root_session_id: &str, task_id: &str) {
    let repo = load_session_repository(config_path);
    seed_background_task_record(&repo, root_session_id, task_id, true);
}

fn seed_background_task_record(
    repo: &mvp::session::repository::SessionRepository,
    root_session_id: &str,
    task_id: &str,
    include_runtime_metadata: bool,
) {
    ensure_root_session(repo, root_session_id);
    let task_label = "Release Check";
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: task_id.to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some(root_session_id.to_owned()),
        label: Some(task_label.to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: task_id.to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some(root_session_id.to_owned()),
        payload_json: json!({
            "task": "check release readiness",
            "label": task_label,
            "timeout_seconds": 60,
        }),
    })
    .expect("append delegate_queued event");
    if !include_runtime_metadata {
        return;
    }
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

pub(super) fn ensure_root_session(
    repo: &mvp::session::repository::SessionRepository,
    root_session_id: &str,
) {
    let existing_root = repo
        .load_session(root_session_id)
        .expect("load root session");
    if existing_root.is_some() {
        return;
    }

    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: root_session_id.to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Ops Root".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create root session");
}

pub(super) fn load_session_repository(
    config_path: &Path,
) -> mvp::session::repository::SessionRepository {
    let loaded =
        mvp::config::load(Some(config_path.to_string_lossy().as_ref())).expect("load config");
    let config = loaded.1;
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    mvp::session::repository::SessionRepository::new(&memory_config).expect("session repository")
}

fn load_memory_runtime_config(
    config_path: &Path,
) -> mvp::memory::runtime_config::MemoryRuntimeConfig {
    let loaded =
        mvp::config::load(Some(config_path.to_string_lossy().as_ref())).expect("load config");
    let config = loaded.1;
    mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory)
}

fn append_tasks_session_turn(config_path: &Path, session_id: &str, role: &str, content: &str) {
    let memory_config = load_memory_runtime_config(config_path);
    mvp::memory::append_turn_direct(session_id, role, content, &memory_config)
        .expect("append tasks session turn");
}

fn open_tasks_test_connection(config_path: &Path) -> Connection {
    let loaded =
        mvp::config::load(Some(config_path.to_string_lossy().as_ref())).expect("load config");
    let config = loaded.1;
    let sqlite_path = PathBuf::from(config.memory.sqlite_path);
    Connection::open(sqlite_path).expect("open tasks test sqlite connection")
}

fn set_tasks_test_session_updated_at(config_path: &Path, session_id: &str, updated_at: i64) {
    let conn = open_tasks_test_connection(config_path);
    conn.execute(
        "UPDATE sessions
         SET updated_at = ?2
         WHERE session_id = ?1",
        params![session_id, updated_at],
    )
    .expect("set tasks test session updated_at");
}

fn set_tasks_test_turn_timestamps(config_path: &Path, session_id: &str, ts: i64) {
    let conn = open_tasks_test_connection(config_path);
    conn.execute(
        "UPDATE turns
         SET ts = ?2
         WHERE session_id = ?1",
        params![session_id, ts],
    )
    .expect("set tasks test turn timestamps");
}

fn archive_tasks_test_session(config_path: &Path, session_id: &str, archived_at: i64) {
    let conn = open_tasks_test_connection(config_path);
    conn.execute(
        "INSERT INTO session_events(
            session_id,
            event_kind,
            actor_session_id,
            payload_json,
            ts
         ) VALUES (?1, ?2, NULL, ?3, ?4)",
        params![session_id, "session_archived", "{}", archived_at],
    )
    .expect("insert tasks test archive event");
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

#[test]
fn tasks_cli_environment_guard_clears_tracked_env_vars_before_applying_overrides() {
    let _guard = TasksCliEnvironmentGuard::set_with_seeded_env(
        &[("LOONGCLAW_SQLITE_PATH", "/tmp/host-value.sqlite3")],
        &[],
    );
    let cleared_value = std::env::var_os("LOONGCLAW_SQLITE_PATH");
    assert_eq!(cleared_value, None);
}

#[tokio::test]
async fn execute_tasks_command_list_returns_visible_background_tasks() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-list");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
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
}

#[tokio::test]
async fn execute_tasks_command_status_surfaces_approval_and_tool_policy() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-status");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
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
}

#[tokio::test]
async fn execute_tasks_command_create_queues_background_task_and_surfaces_follow_up_recipes() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-create");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());

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
}

#[tokio::test]
async fn execute_tasks_command_create_returns_queued_outcome_when_task_hydration_fails() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-create-best-effort");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config_with(root.path(), |config| {
        config.tools.sessions.enabled = false;
    });

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
    .expect("tasks create should still succeed");

    let queued_task_id = execution.payload["queued_outcome"]["child_session_id"]
        .as_str()
        .expect("queued task id");
    let rendered = loongclaw_daemon::tasks_cli::render_tasks_cli_text(&execution)
        .expect("render tasks create");

    assert_eq!(execution.payload["command"], "create");
    assert_eq!(execution.payload["task"]["task_id"], queued_task_id);
    assert_eq!(execution.payload["task"]["scope_session_id"], "ops-root");
    assert!(
        execution.payload["task_lookup_error"]
            .as_str()
            .expect("task lookup error")
            .contains("session tools are disabled"),
        "expected hydration failure to surface lookup error, got: {:?}",
        execution.payload
    );
    assert!(
        rendered.contains("task_lookup_error:"),
        "rendered create output should surface hydration warning: {rendered}"
    );
}

#[tokio::test]
async fn execute_tasks_command_create_latest_session_selector_resolves_newest_resumable_root() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-create-latest");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    let repo = load_session_repository(&config_path);

    ensure_root_session(&repo, "ops-root-old");
    append_tasks_session_turn(&config_path, "ops-root-old", "user", "older root");
    set_tasks_test_session_updated_at(&config_path, "ops-root-old", 100);
    set_tasks_test_turn_timestamps(&config_path, "ops-root-old", 100);

    ensure_root_session(&repo, "ops-root-selected");
    append_tasks_session_turn(&config_path, "ops-root-selected", "user", "selected root");
    set_tasks_test_session_updated_at(&config_path, "ops-root-selected", 200);
    set_tasks_test_turn_timestamps(&config_path, "ops-root-selected", 200);

    ensure_root_session(&repo, "ops-root-empty");
    set_tasks_test_session_updated_at(&config_path, "ops-root-empty", 300);

    ensure_root_session(&repo, "ops-root-archived");
    append_tasks_session_turn(
        &config_path,
        "ops-root-archived",
        "assistant",
        "archived root",
    );
    set_tasks_test_session_updated_at(&config_path, "ops-root-archived", 400);
    set_tasks_test_turn_timestamps(&config_path, "ops-root-archived", 400);
    archive_tasks_test_session(&config_path, "ops-root-archived", 500);

    let execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "latest".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::Create {
                task: "research release readiness".to_owned(),
                label: Some("Release Check".to_owned()),
                timeout_seconds: Some(45),
            },
        },
    )
    .await
    .expect("tasks create with latest selector should succeed");

    let created_task_id = execution.payload["task"]["task_id"]
        .as_str()
        .expect("created task id");
    let created_task = repo
        .load_session(created_task_id)
        .expect("load created task session")
        .expect("created task session");

    assert_eq!(execution.payload["command"], "create");
    assert_eq!(execution.current_session_id, "ops-root-selected");
    assert_eq!(execution.payload["current_session_id"], "ops-root-selected");
    assert_eq!(
        execution.payload["task"]["scope_session_id"],
        "ops-root-selected"
    );
    assert_eq!(
        created_task.parent_session_id.as_deref(),
        Some("ops-root-selected")
    );
}

#[tokio::test]
async fn execute_tasks_command_latest_session_selector_rejects_missing_resumable_root() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-latest-missing");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    let repo = load_session_repository(&config_path);

    ensure_root_session(&repo, "ops-root-empty");
    set_tasks_test_session_updated_at(&config_path, "ops-root-empty", 100);

    let result = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "latest".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::List {
                limit: 20,
                state: None,
                overdue_only: false,
                include_archived: false,
            },
        },
    )
    .await;

    let error = match result {
        Ok(_) => panic!("latest selector should fail without a resumable root session"),
        Err(error) => error,
    };

    assert!(
        error.contains("latest"),
        "expected latest selector error, got: {error}"
    );
}

#[tokio::test]
async fn execute_tasks_command_list_latest_session_selector_uses_selected_root_scope() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-list-latest");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    let repo = load_session_repository(&config_path);

    ensure_root_session(&repo, "ops-root-old");
    append_tasks_session_turn(&config_path, "ops-root-old", "user", "older root");
    set_tasks_test_session_updated_at(&config_path, "ops-root-old", 100);
    set_tasks_test_turn_timestamps(&config_path, "ops-root-old", 100);
    seed_background_task_record(&repo, "ops-root-old", "delegate:old-task", false);

    ensure_root_session(&repo, "ops-root-selected");
    append_tasks_session_turn(&config_path, "ops-root-selected", "user", "selected root");
    set_tasks_test_session_updated_at(&config_path, "ops-root-selected", 200);
    set_tasks_test_turn_timestamps(&config_path, "ops-root-selected", 200);
    seed_background_task_record(&repo, "ops-root-selected", "delegate:selected-task", false);

    ensure_root_session(&repo, "ops-root-empty");
    set_tasks_test_session_updated_at(&config_path, "ops-root-empty", 300);

    let execution = loongclaw_daemon::tasks_cli::execute_tasks_command(
        loongclaw_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "latest".to_owned(),
            command: loongclaw_daemon::tasks_cli::TasksCommands::List {
                limit: 20,
                state: None,
                overdue_only: false,
                include_archived: false,
            },
        },
    )
    .await
    .expect("tasks list with latest selector should succeed");

    assert_eq!(execution.payload["command"], "list");
    assert_eq!(execution.current_session_id, "ops-root-selected");
    assert_eq!(execution.payload["current_session_id"], "ops-root-selected");
    assert_eq!(execution.payload["matched_count"], 1);
    assert_eq!(execution.payload["returned_count"], 1);
    assert_eq!(
        execution.payload["tasks"][0]["task_id"],
        "delegate:selected-task"
    );
}

#[tokio::test]
async fn execute_tasks_command_list_counts_background_tasks_beyond_session_tool_limit() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-list-many");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    let repo = load_session_repository(&config_path);
    ensure_root_session(&repo, "ops-root");
    for index in 0..101 {
        let task_id = format!("delegate:list-{index:03}");
        seed_background_task_record(&repo, "ops-root", &task_id, false);
    }

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
    assert_eq!(execution.payload["matched_count"], 101);
    assert_eq!(execution.payload["returned_count"], 20);
}

#[tokio::test]
async fn execute_tasks_command_events_and_wait_surface_incremental_payloads() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-events");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
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
}

#[tokio::test]
async fn execute_tasks_command_cancel_dry_run_surfaces_cancel_action() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-cancel");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
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
}

#[tokio::test]
async fn execute_tasks_command_recover_dry_run_surfaces_non_recoverable_result() {
    let root = TempDirGuard::new("loongclaw-tasks-cli-recover");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
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
}
