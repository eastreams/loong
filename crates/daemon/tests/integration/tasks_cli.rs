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
    "ANTHROPIC_API_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AZURE_OPENAI_API_KEY",
    "DEEPSEEK_API_KEY",
    "GEMINI_API_KEY",
    "LOONG_BROWSER_COMPANION_COMMAND",
    "LOONG_BROWSER_COMPANION_ENABLED",
    "LOONG_BROWSER_COMPANION_EXPECTED_VERSION",
    "LOONG_BROWSER_COMPANION_TIMEOUT_SECONDS",
    "LOONG_BROWSER_ENABLED",
    "LOONG_BROWSER_MAX_LINKS",
    "LOONG_BROWSER_MAX_SESSIONS",
    "LOONG_BROWSER_MAX_TEXT_CHARS",
    "LOONG_CONFIG_PATH",
    "LOONG_EXTERNAL_SKILLS_ALLOWED_DOMAINS",
    "LOONG_EXTERNAL_SKILLS_AUTO_EXPOSE_INSTALLED",
    "LOONG_EXTERNAL_SKILLS_BLOCKED_DOMAINS",
    "LOONG_EXTERNAL_SKILLS_ENABLED",
    "LOONG_EXTERNAL_SKILLS_INSTALL_ROOT",
    "LOONG_EXTERNAL_SKILLS_REQUIRE_DOWNLOAD_APPROVAL",
    "LOONG_FILE_ROOT",
    "LOONG_MEMORY_BACKEND",
    "LOONG_MEMORY_PROFILE",
    "LOONG_MEMORY_PROFILE_NOTE",
    "LOONG_MEMORY_SUMMARY_MAX_CHARS",
    "LOONG_SHELL_ALLOWLIST",
    "LOONG_SHELL_DEFAULT_MODE",
    "LOONG_SHELL_DENY",
    "LOONG_SLIDING_WINDOW",
    "LOONG_SQLITE_PATH",
    "LOONG_TOOL_DELEGATE_ENABLED",
    "LOONG_TOOL_MESSAGES_ENABLED",
    "LOONG_TOOL_SESSIONS_ALLOW_MUTATION",
    "LOONG_TOOL_SESSIONS_ENABLED",
    "LOONG_WEB_FETCH_ALLOWED_DOMAINS",
    "LOONG_WEB_FETCH_ALLOW_PRIVATE_HOSTS",
    "LOONG_WEB_FETCH_BLOCKED_DOMAINS",
    "LOONG_WEB_FETCH_ENABLED",
    "LOONG_WEB_FETCH_MAX_BYTES",
    "LOONG_WEB_FETCH_MAX_REDIRECTS",
    "LOONG_WEB_FETCH_TIMEOUT_SECONDS",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
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

fn tasks_cli_create_environment_guard() -> TasksCliEnvironmentGuard {
    let current_executable = std::env::current_exe().expect("current test executable");
    let deps_directory = current_executable.parent().expect("deps directory");
    let target_directory = deps_directory.parent().expect("target directory");
    let detached_delegate_binary = target_directory.join("loong");
    let detached_delegate_binary = detached_delegate_binary
        .to_str()
        .expect("detached delegate binary path should be utf-8")
        .to_owned();
    let seeded_pairs = [("CARGO_BIN_EXE_loong", detached_delegate_binary.as_str())];

    TasksCliEnvironmentGuard::set_with_seeded_env(&seeded_pairs, &[])
}

pub(super) fn write_tasks_config_with(
    root: &Path,
    configure: impl FnOnce(&mut mvp::config::LoongConfig),
) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");
    let config_path = root.join("loong.toml");
    let mut config = mvp::config::LoongConfig::default();
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    config.audit.mode = mvp::config::AuditMode::InMemory;
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
    let workspace_root = format!("/tmp/loong/tasks-cli/{task_id}");
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
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 60,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["file.read"],
                "workspace_root": workspace_root,
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
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
    let session_store_config = mvp::session::store::SessionStoreConfig::from(&memory_config);
    mvp::session::repository::SessionRepository::new(&session_store_config)
        .expect("session repository")
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

fn append_tasks_conversation_event(config_path: &Path, session_id: &str, payload: Value) {
    let serialized_payload =
        serde_json::to_string(&payload).expect("serialize tasks conversation event");
    append_tasks_session_turn(config_path, session_id, "assistant", &serialized_payload);
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
        "loong",
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
        "/tmp/loong.toml",
    ])
    .expect("tasks create CLI should parse");

    match cli.command {
        Some(Commands::Tasks {
            config,
            json,
            session,
            command,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
            assert!(json);
            assert_eq!(session, "ops-root");
            match command {
                loong_daemon::tasks_cli::TasksCommands::Create {
                    task,
                    label,
                    timeout_seconds,
                } => {
                    assert_eq!(task, "research release status");
                    assert_eq!(label.as_deref(), Some("release-scan"));
                    assert_eq!(timeout_seconds, Some(90));
                }
                loong_daemon::tasks_cli::TasksCommands::List { .. } => {
                    panic!("unexpected tasks subcommand parsed: List")
                }
                loong_daemon::tasks_cli::TasksCommands::Status { .. } => {
                    panic!("unexpected tasks subcommand parsed: Status")
                }
                loong_daemon::tasks_cli::TasksCommands::Events { .. } => {
                    panic!("unexpected tasks subcommand parsed: Events")
                }
                loong_daemon::tasks_cli::TasksCommands::Wait { .. } => {
                    panic!("unexpected tasks subcommand parsed: Wait")
                }
                loong_daemon::tasks_cli::TasksCommands::Cancel { .. } => {
                    panic!("unexpected tasks subcommand parsed: Cancel")
                }
                loong_daemon::tasks_cli::TasksCommands::Recover { .. } => {
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
        "loong",
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
        "/tmp/loong.toml",
    ])
    .expect("tasks wait CLI should parse");

    match cli.command {
        Some(Commands::Tasks {
            config,
            json,
            session,
            command,
        }) => {
            assert_eq!(config.as_deref(), Some("/tmp/loong.toml"));
            assert!(json);
            assert_eq!(session, "ops-root");
            match command {
                loong_daemon::tasks_cli::TasksCommands::Wait {
                    task_id,
                    after_id,
                    timeout_ms,
                } => {
                    assert_eq!(task_id, "delegate:abc123");
                    assert_eq!(after_id, Some(10));
                    assert_eq!(timeout_ms, 2500);
                }
                loong_daemon::tasks_cli::TasksCommands::Create { .. } => {
                    panic!("unexpected tasks subcommand parsed: Create")
                }
                loong_daemon::tasks_cli::TasksCommands::List { .. } => {
                    panic!("unexpected tasks subcommand parsed: List")
                }
                loong_daemon::tasks_cli::TasksCommands::Status { .. } => {
                    panic!("unexpected tasks subcommand parsed: Status")
                }
                loong_daemon::tasks_cli::TasksCommands::Events { .. } => {
                    panic!("unexpected tasks subcommand parsed: Events")
                }
                loong_daemon::tasks_cli::TasksCommands::Cancel { .. } => {
                    panic!("unexpected tasks subcommand parsed: Cancel")
                }
                loong_daemon::tasks_cli::TasksCommands::Recover { .. } => {
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
        &[("LOONG_SQLITE_PATH", "/tmp/host-value.sqlite3")],
        &[],
    );
    let cleared_value = std::env::var_os("LOONG_SQLITE_PATH");
    assert_eq!(cleared_value, None);
}

#[tokio::test]
async fn execute_tasks_command_list_returns_visible_background_tasks() {
    let root = TempDirGuard::new("loong-tasks-cli-list");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::List {
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
    assert_eq!(
        execution.payload["tasks"][0]["workflow"]["workflow_id"],
        "ops-root"
    );
    assert_eq!(
        execution.payload["tasks"][0]["workflow"]["phase"],
        "execute"
    );
    assert_eq!(
        execution.payload["tasks"][0]["workflow"]["binding"]["mode"],
        "advisory_only"
    );
    assert_eq!(
        execution.payload["tasks"][0]["task_status"]["kind"],
        "approval_pending"
    );
    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&execution).expect("render tasks list");
    assert!(
        rendered.contains("status=approval_pending"),
        "list render should surface derived task status: {rendered}"
    );
    assert!(
        rendered.contains("workflow_phase=execute"),
        "list render should surface workflow phase: {rendered}"
    );
}

#[tokio::test]
async fn execute_tasks_command_status_surfaces_approval_and_tool_policy() {
    let root = TempDirGuard::new("loong-tasks-cli-status");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    seed_background_task(&config_path, "ops-root", "delegate:task-1");
    let prompt_frame_event = json!({
        "type": "conversation_event",
        "event": "provider_prompt_frame_snapshot",
        "payload": {
            "provider_round": 1,
            "phase": "initial",
            "prompt_frame": {
                "schema_version": 1,
                "total_estimated_tokens": 64,
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
                "stable_prefix_hash_sha256": "prefix-task",
                "cached_prefix_sha256": "cached-task",
                "advisory_profile_hash": "profile-a",
                "session_local_recall_hash": "recall-a",
                "recent_window_hash": "window-a",
                "turn_ephemeral_hash": null
            }
        }
    });
    append_tasks_conversation_event(&config_path, "delegate:task-1", prompt_frame_event);
    append_tasks_conversation_event(
        &config_path,
        "delegate:task-1",
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
    append_tasks_conversation_event(
        &config_path,
        "delegate:task-1",
        json!({
            "type": "conversation_event",
            "event": "verify_failed",
            "payload": {
                "metrics": {
                    "rounds_started": 1,
                    "rounds_succeeded": 0,
                    "rounds_failed": 0,
                    "verify_failures": 1,
                    "replans_triggered": 0,
                    "total_attempts_used": 1
                },
                "health_signal": {
                    "severity": "critical",
                    "flags": ["verify"]
                }
            }
        }),
    );
    append_tasks_conversation_event(
        &config_path,
        "delegate:task-1",
        json!({
            "type": "conversation_event",
            "event": "replan_triggered",
            "payload": {
                "metrics": {
                    "rounds_started": 1,
                    "rounds_succeeded": 0,
                    "rounds_failed": 0,
                    "verify_failures": 1,
                    "replans_triggered": 1,
                    "total_attempts_used": 1
                },
                "health_signal": {
                    "severity": "critical",
                    "flags": ["verify"]
                }
            }
        }),
    );
    append_tasks_conversation_event(
        &config_path,
        "delegate:task-1",
        json!({
            "type": "conversation_event",
            "event": "final_status",
            "payload": {
                "status": "failed",
                "failure_code": "safe_lane_backpressure_limit",
                "route_decision": "terminal",
                "route_reason": "session_governor_failed_threshold",
                "metrics": {
                    "rounds_started": 1,
                    "rounds_succeeded": 0,
                    "rounds_failed": 1,
                    "verify_failures": 1,
                    "replans_triggered": 1,
                    "total_attempts_used": 2
                },
                "health_signal": {
                    "severity": "critical",
                    "flags": ["verify"]
                }
            }
        }),
    );
    append_tasks_conversation_event(
        &config_path,
        "delegate:task-1",
        json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "lane": {
                        "lane": "safe",
                        "result_kind": "tool_call"
                    },
                    "finalization": {
                        "persistence_mode": "inline_provider_error"
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "compaction_diagnostics": {
                    "summary_turn_count": 4,
                    "retained_turn_count": 3,
                    "demoted_recent_turn_count": 1,
                    "total_turns": 7,
                    "assistant_turns": 3,
                    "low_signal_turns": 1,
                    "tool_result_line_prunes": 1,
                    "tool_outcome_record_prunes": 0
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failure"
                }
            }
        }),
    );

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Status {
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
        execution.payload["task"]["workflow"]["workflow_id"],
        "ops-root"
    );
    assert_eq!(execution.payload["task"]["workflow"]["phase"], "execute");
    assert_eq!(
        execution.payload["task"]["workflow"]["operation_kind"],
        "task"
    );
    assert_eq!(
        execution.payload["task"]["workflow"]["binding"]["execution_surface"],
        "delegate.async"
    );
    assert_eq!(
        execution.payload["task"]["workflow"]["binding"]["worktree"]["worktree_id"],
        "delegate:task-1"
    );
    assert_eq!(
        execution.payload["task"]["task_status"]["kind"],
        "approval_pending"
    );
    assert_eq!(
        execution.payload["task"]["task_status"]["next_action"],
        "resolve_request"
    );
    assert_eq!(
        execution.payload["task"]["tool_policy"]["requested_tool_ids"][0],
        "file.read"
    );
    assert_eq!(
        execution.payload["task"]["tool_policy"]["visible_requested_tool_ids"][0],
        "read"
    );
    assert_eq!(
        execution.payload["task"]["tool_policy"]["effective_tool_ids"][0],
        "file.read"
    );
    assert_eq!(
        execution.payload["task"]["tool_policy"]["visible_effective_tool_ids"][0],
        "read"
    );
    assert_eq!(
        execution.payload["task"]["prompt_frame"]["summary"]["latest_phase"],
        "initial"
    );
    assert_eq!(
        execution.payload["task"]["prompt_frame"]["summary"]["latest_total_estimated_tokens"],
        64
    );
    assert_eq!(
        execution.payload["task"]["safe_lane"]["summary"]["final_status"],
        "failed"
    );
    assert_eq!(
        execution.payload["task"]["safe_lane"]["summary"]["verify_failed_events"],
        1
    );
    assert_eq!(
        execution.payload["task"]["safe_lane"]["summary"]["replan_triggered_events"],
        1
    );
    assert_eq!(
        execution.payload["task"]["turn_checkpoint"]["summary"]["session_state"],
        "finalization_failed"
    );
    assert_eq!(
        execution.payload["task"]["turn_checkpoint"]["summary"]["requires_recovery"],
        true
    );

    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&execution).expect("render tasks status");
    assert!(
        rendered.contains("approval_requests: 1"),
        "status render should surface approval count: {rendered}"
    );
    assert!(
        rendered.contains("task_status: approval_pending"),
        "status render should surface derived task status: {rendered}"
    );
    assert!(
        rendered.contains("task_next_action: resolve_request"),
        "status render should surface next action: {rendered}"
    );
    assert!(
        rendered.contains("workflow_phase: execute"),
        "status render should surface workflow phase: {rendered}"
    );
    assert!(
        rendered.contains("workflow_binding_mode: advisory_only"),
        "status render should surface workflow binding mode: {rendered}"
    );
    assert!(
        rendered.contains("workflow_worktree_id: delegate:task-1"),
        "status render should surface workflow worktree id: {rendered}"
    );
    assert!(
        rendered.contains("requested_tool_ids: read"),
        "status render should surface visible requested tool ids: {rendered}"
    );
    assert!(
        rendered.contains("effective_tool_ids: read"),
        "status render should surface visible effective tool ids: {rendered}"
    );
    assert!(
        rendered.contains("prompt_frame: phase=initial total_tokens=64"),
        "status render should surface prompt-frame summary: {rendered}"
    );
    assert!(
        rendered.contains("stable_prefix=prefix-task"),
        "status render should surface prompt-frame stable prefix: {rendered}"
    );
    assert!(
        rendered.contains("safe_lane: status=failed rounds_started=1 verify_failed=1 replans=1"),
        "status render should surface safe-lane summary: {rendered}"
    );
    assert!(
        rendered.contains("failure_code=safe_lane_backpressure_limit"),
        "status render should surface safe-lane failure code: {rendered}"
    );
    assert!(
        rendered.contains("route=terminal/session_governor_failed_threshold health=critical"),
        "status render should surface safe-lane failure route and health: {rendered}"
    );
    assert!(
        rendered.contains("turn_checkpoint: session_state=finalization_failed durable=yes"),
        "status render should surface turn-checkpoint recovery context: {rendered}"
    );
    assert!(
        rendered.contains("reply_durable=yes requires_recovery=yes"),
        "status render should surface turn-checkpoint recovery flags: {rendered}"
    );
    assert!(
        rendered.contains("stage=finalization_failed"),
        "status render should surface turn-checkpoint stage: {rendered}"
    );
    assert!(
        rendered.contains("after_turn=completed compaction=failed"),
        "status render should surface turn-checkpoint progress detail: {rendered}"
    );
    assert!(
        rendered.contains("compaction_diag=summary:4"),
        "status render should surface compact diagnostics detail: {rendered}"
    );
    assert!(
        rendered.contains("tool_results:1"),
        "status render should surface compact diagnostics prune counts: {rendered}"
    );
}

#[tokio::test]
async fn execute_tasks_command_create_queues_background_task_and_surfaces_follow_up_recipes() {
    let root = TempDirGuard::new("loong-tasks-cli-create");
    let _env = tasks_cli_create_environment_guard();
    let config_path = write_tasks_config(root.path());

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Create {
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
    let task_status = execution.payload["task"]["task_status"]["status"]
        .as_str()
        .expect("task status");
    let task_next_action = execution.payload["task"]["task_status"]["next_action"]
        .as_str()
        .expect("task next action");

    assert_eq!(execution.payload["command"], "create");
    assert_eq!(execution.payload["current_session_id"], "ops-root");
    assert_eq!(execution.payload["task"]["scope_session_id"], "ops-root");
    assert_eq!(execution.payload["task"]["label"], "Release Check");
    assert_eq!(execution.payload["task"]["timeout_seconds"], 45);
    assert_eq!(
        execution.payload["task"]["owner_kind"],
        "background_task_host"
    );
    assert_eq!(
        execution.payload["task"]["session"]["kind"],
        "delegate_child"
    );
    assert!(task_id.starts_with("delegate:"));
    assert_eq!(recipes.len(), 3);
    let task_started_immediately = task_status == "running";
    assert!(
        matches!(task_status, "queued" | "running" | "failed"),
        "create should surface truthful immediate task status, got: {task_status}"
    );
    assert!(
        !task_started_immediately || task_next_action == "wait",
        "running tasks should still point operators at wait as the immediate follow-up"
    );
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
    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&execution).expect("render tasks create");
    let expected_status_line = format!("task_status: {task_status}");
    let expected_next_action_line = format!("task_next_action: {task_next_action}");
    assert!(
        rendered.contains(&expected_status_line),
        "create render should surface derived task status: {rendered}"
    );
    assert!(
        rendered.contains(&expected_next_action_line),
        "create render should surface next action: {rendered}"
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

    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&execution).expect("render tasks create");
    assert!(
        rendered.contains("owner_kind: background_task_host"),
        "tasks create render should surface owner kind: {rendered}"
    );
}

#[tokio::test]
async fn execute_tasks_command_create_returns_queued_outcome_when_task_hydration_fails() {
    let root = TempDirGuard::new("loong-tasks-cli-create-best-effort");
    let _env = tasks_cli_create_environment_guard();
    let config_path = write_tasks_config_with(root.path(), |config| {
        config.tools.sessions.enabled = false;
    });

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Create {
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
    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&execution).expect("render tasks create");

    assert_eq!(execution.payload["command"], "create");
    assert_eq!(execution.payload["task"]["task_id"], queued_task_id);
    assert_eq!(execution.payload["task"]["scope_session_id"], "ops-root");
    assert!(execution.payload["task"]["session"].is_null());
    assert!(execution.payload["task"]["delegate"].is_null());
    assert_eq!(execution.payload["task"]["recent_events"], json!([]));
    assert!(execution.payload["task"]["prompt_frame"].is_null());
    assert!(execution.payload["task"]["safe_lane"].is_null());
    assert!(execution.payload["task"]["turn_checkpoint"].is_null());
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
    let root = TempDirGuard::new("loong-tasks-cli-create-latest");
    let _env = tasks_cli_create_environment_guard();
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

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "latest".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Create {
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
    let root = TempDirGuard::new("loong-tasks-cli-latest-missing");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    let repo = load_session_repository(&config_path);

    ensure_root_session(&repo, "ops-root-empty");
    set_tasks_test_session_updated_at(&config_path, "ops-root-empty", 100);

    let result = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "latest".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::List {
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
    let root = TempDirGuard::new("loong-tasks-cli-list-latest");
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

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "latest".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::List {
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
    let root = TempDirGuard::new("loong-tasks-cli-list-many");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    let repo = load_session_repository(&config_path);
    ensure_root_session(&repo, "ops-root");
    for index in 0..101 {
        let task_id = format!("delegate:list-{index:03}");
        seed_background_task_record(&repo, "ops-root", &task_id, false);
    }

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::List {
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
    let root = TempDirGuard::new("loong-tasks-cli-events");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let events_execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Events {
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

    let wait_execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Wait {
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
    assert_eq!(
        wait_execution.payload["task"]["task_status"]["status"],
        "approval_pending"
    );
    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&wait_execution).expect("render tasks wait");
    assert!(
        rendered.contains("task_status: approval_pending"),
        "wait render should surface derived task status: {rendered}"
    );
    assert!(
        rendered.contains("task_next_action: resolve_request"),
        "wait render should surface next action: {rendered}"
    );
}

#[test]
fn render_tasks_status_text_escapes_control_characters() {
    let execution = loong_daemon::tasks_cli::TasksCommandExecution {
        resolved_config_path: "/tmp/loong.toml".to_owned(),
        current_session_id: "ops-root".to_owned(),
        payload: json!({
            "command": "status",
            "task": {
                "task_id": "delegate:\u{1b}[31mchild",
                "scope_session_id": "ops-root\nnext",
                "label": "line1\nline2",
                "session_state": "running",
                "phase": "queued",
                "owner_kind": "background_task_host",
                "timeout_seconds": 60,
                "last_error": "boom\u{1b}[0m",
                "approval": {
                    "matched_count": 1,
                    "attention_summary": {
                        "needs_attention_count": 1
                    }
                },
                "approval_lookup_error": "approval\nmissing",
                "tool_policy": {
                    "effective_tool_ids": ["file.read\nnext"],
                    "effective_runtime_narrowing": {
                        "allow": "line1\nline2"
                    }
                },
                "tool_policy_lookup_error": "policy\nmissing",
                "prompt_frame": {
                    "available": false,
                    "error": "prompt\nmissing"
                },
                "safe_lane": {
                    "available": false,
                    "error": "lane\nmissing"
                },
                "turn_checkpoint": {
                    "available": false,
                    "error": "checkpoint\nmissing"
                }
            }
        }),
    };

    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&execution).expect("render tasks");

    assert!(
        !rendered.contains('\u{1b}'),
        "rendered status should not contain raw escape characters: {rendered:?}"
    );
    assert!(
        rendered.contains("scope_session_id: ops-root\\nnext"),
        "expected escaped scope session id: {rendered}"
    );
    assert!(
        rendered.contains("label: line1\\nline2"),
        "expected escaped label: {rendered}"
    );
    assert!(
        rendered.contains("last_error: boom\\u{1b}[0m"),
        "expected escaped last_error: {rendered}"
    );
    assert!(
        rendered.contains("prompt_frame: unavailable error=prompt\\nmissing"),
        "expected escaped prompt-frame error: {rendered}"
    );
    assert!(
        rendered.contains("safe_lane: unavailable error=lane\\nmissing"),
        "expected escaped safe-lane error: {rendered}"
    );
    assert!(
        rendered.contains("turn_checkpoint: unavailable error=checkpoint\\nmissing"),
        "expected escaped turn-checkpoint error: {rendered}"
    );
    assert!(
        rendered.contains("approval_lookup_error: approval\\nmissing"),
        "expected escaped approval lookup error: {rendered}"
    );
    assert!(
        rendered.contains("tool_policy_lookup_error: policy\\nmissing"),
        "expected escaped tool-policy lookup error: {rendered}"
    );
}

#[test]
fn render_tasks_events_text_escapes_control_characters() {
    let execution = loong_daemon::tasks_cli::TasksCommandExecution {
        resolved_config_path: "/tmp/loong.toml".to_owned(),
        current_session_id: "ops-root".to_owned(),
        payload: json!({
            "command": "events",
            "task_id": "delegate:\u{1b}[31mchild",
            "next_after_id": 7,
            "events": [
                {
                    "id": 1,
                    "event_kind": "delegate_queued\nnext",
                    "ts": 123
                }
            ]
        }),
    };

    let rendered =
        loong_daemon::tasks_cli::render_tasks_cli_text(&execution).expect("render events");

    assert!(
        !rendered.contains('\u{1b}'),
        "rendered events should not contain raw escape characters: {rendered:?}"
    );
    assert!(
        rendered.contains("events for `delegate:\\u{1b}[31mchild`"),
        "expected escaped task id: {rendered}"
    );
    assert!(
        rendered.contains("- #1 delegate_queued\\nnext ts=123"),
        "expected escaped event kind: {rendered}"
    );
}

#[tokio::test]
async fn execute_tasks_command_cancel_dry_run_surfaces_cancel_action() {
    let root = TempDirGuard::new("loong-tasks-cli-cancel");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Cancel {
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
    let root = TempDirGuard::new("loong-tasks-cli-recover");
    let _env = TasksCliEnvironmentGuard::set(&[]);
    let config_path = write_tasks_config(root.path());
    seed_background_task(&config_path, "ops-root", "delegate:task-1");

    let execution = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Recover {
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
