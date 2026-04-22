use super::*;
use loong_app::internal_events::append_internal_event_to_journal;
use rusqlite::params;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

fn automation_integration_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_automation_integration() -> std::sync::MutexGuard<'static, ()> {
    automation_integration_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT_TEMP_DIR_SEED: AtomicUsize = AtomicUsize::new(1);
    let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let process_id = std::process::id();
    std::env::temp_dir().join(format!("{prefix}-{process_id}-{seed}-{nanos}"))
}

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        Self {
            path: unique_temp_dir(prefix),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}

fn write_automation_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let mut config = mvp::config::LoongConfig::default();
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    config.audit.mode = mvp::config::AuditMode::InMemory;
    config.tools.file_root = Some(root.display().to_string());
    config.tools.sessions.allow_mutation = true;

    let config_path = root.join("loong.toml");
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    config_path
}

fn load_session_repository(config_path: &Path) -> mvp::session::repository::SessionRepository {
    let (_, config) =
        mvp::config::load(Some(config_path.to_string_lossy().as_ref())).expect("load config");
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    mvp::session::repository::SessionRepository::new(&memory_config).expect("session repository")
}

fn wait_for_session_record(
    repo: &mvp::session::repository::SessionRepository,
    session_id: &str,
) -> mvp::session::repository::SessionRecord {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(snapshot) = repo
            .load_session(session_id)
            .expect("load queued child session")
        {
            return snapshot;
        }
        if std::time::Instant::now() >= deadline {
            panic!("timed out waiting for queued child session `{session_id}`");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

async fn wait_for_trigger_fire_count(
    config_path: &Path,
    trigger_id: &str,
    expected_fire_count: u64,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let show_payload = loong_daemon::automation_cli::execute_automation_command(
            loong_daemon::automation_cli::AutomationCommandOptions {
                config: Some(config_path.display().to_string()),
                json: false,
                command: loong_daemon::automation_cli::AutomationCommands::Show(
                    loong_daemon::automation_cli::AutomationShowCommandOptions {
                        id: trigger_id.to_owned(),
                    },
                ),
            },
        )
        .await
        .expect("show automation trigger while waiting");
        if show_payload["trigger"]["fire_count"].as_u64() == Some(expected_fire_count) {
            return show_payload;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for trigger `{trigger_id}` fire_count={expected_fire_count}; last payload={show_payload}"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn automation_cursor_path(loong_home: &Path) -> PathBuf {
    loong_home.join("automation").join("internal-events.cursor")
}

fn automation_serve_lock_path(loong_home: &Path) -> PathBuf {
    loong_home.join("automation").join("serve.lock")
}

fn wait_for_path(path: &Path, description: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if path.exists() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("timed out waiting for {description} at {}", path.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn wait_for_serve_lock(child: &mut Child, path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if path.exists() {
            return;
        }
        if let Some(status) = child.try_wait().expect("poll automation serve child") {
            let stderr = child
                .stderr
                .take()
                .map(|mut stream| {
                    let mut output = String::new();
                    std::io::Read::read_to_string(&mut stream, &mut output)
                        .expect("read automation serve stderr");
                    output
                })
                .unwrap_or_default();
            panic!(
                "automation serve exited before creating lock {}: status={status:?} stderr={stderr}",
                path.display()
            );
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for automation serve lock at {}",
                path.display()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn wait_for_cursor_value(path: &Path, expected: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let current = fs::read_to_string(path).ok();
        let current_line_cursor = current.as_deref().and_then(parse_cursor_line_cursor);
        if current_line_cursor.as_deref() == Some(expected) {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for automation cursor {expected} at {}; last value={current:?}",
                path.display()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn parse_cursor_line_cursor(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some("0".to_owned());
    }
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(trimmed.to_owned());
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|value| value.get("line_cursor").and_then(serde_json::Value::as_u64))
        .map(|value| value.to_string())
}

#[tokio::test]
async fn automation_cli_show_loads_legacy_store_without_run_history() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-cli-legacy-store");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    let automation_dir = loong_home.join("automation");
    fs::create_dir_all(&automation_dir).expect("create automation dir");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let raw = json!({
        "schema_version": 1,
        "triggers": [
            {
                "trigger_id": "atrg-legacy",
                "name": "Legacy Trigger",
                "status": "active",
                "source": {
                    "type": "event",
                    "event": {
                        "event_name": "session.cancelled",
                        "json_pointer": "/session_id",
                        "equals_json": "delegate:legacy",
                        "contains_text": null
                    }
                },
                "action": {
                    "type": "background_task",
                    "background_task": {
                        "session": "ops-root",
                        "task": "follow up on a legacy trigger",
                        "label": "Legacy Follow-up",
                        "timeout_seconds": 30
                    }
                },
                "created_at_ms": 10,
                "updated_at_ms": 10,
                "last_fired_at_ms": null,
                "last_task_id": null,
                "last_error": null,
                "fire_count": 0
            }
        ]
    });
    fs::write(
        automation_dir.join("triggers.json"),
        serde_json::to_vec_pretty(&raw).expect("serialize legacy store"),
    )
    .expect("write legacy trigger store");

    let show_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions {
                    id: "atrg-legacy".to_owned(),
                },
            ),
        },
    )
    .await
    .expect("show legacy automation trigger");

    assert_eq!(show_payload["trigger"]["trigger_id"], "atrg-legacy");
    assert_eq!(
        show_payload["trigger"]["source"]["event"]["event_name"],
        "session.cancelled"
    );
    assert_eq!(show_payload["trigger"]["fire_count"], 0);
    assert_eq!(show_payload["trigger"]["run_history"], json!([]));
    drop(guard);
}

#[tokio::test]
async fn automation_journal_inspect_surfaces_layout_and_cursor() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-inspect");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    fs::create_dir_all(
        loong_app::internal_events::internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        loong_app::internal_events::internal_event_journal_state_path(),
        concat!(
            "{\n",
            "  \"schema_version\": 1,\n",
            "  \"active_segment_id\": \"segment-000002\",\n",
            "  \"segments\": [\n",
            "    {\"segment_id\":\"legacy\",\"status\":\"legacy\"},\n",
            "    {\"segment_id\":\"segment-000001\",\"status\":\"sealed\",\"created_at_ms\":10,\"sealed_at_ms\":20},\n",
            "    {\"segment_id\":\"segment-000002\",\"status\":\"active\",\"created_at_ms\":30}\n",
            "  ]\n",
            "}\n"
        ),
    )
    .expect("write journal state");
    fs::write(
        automation_cursor_path(&loong_home),
        concat!(
            "{\n",
            "  \"segment_id\": \"segment-000002\",\n",
            "  \"line_cursor\": 3,\n",
            "  \"byte_offset\": 120,\n",
            "  \"journal_fingerprint\": \"abc\"\n",
            "}\n"
        ),
    )
    .expect("write cursor payload");

    let payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Journal(
                loong_daemon::automation_cli::AutomationJournalCommandOptions {
                    command: loong_daemon::automation_cli::AutomationJournalCommands::Inspect(
                        loong_daemon::automation_cli::AutomationJournalInspectCommandOptions::default(),
                    ),
                },
            ),
        },
    )
    .await
    .expect("inspect automation journal");

    assert_eq!(payload["command"], "journal_inspect");
    assert_eq!(payload["layout"]["active_segment_id"], "segment-000002");
    assert_eq!(payload["layout"]["segments"][0]["segment_id"], "legacy");
    assert_eq!(payload["layout"]["segments"][1]["status"], "sealed");
    assert_eq!(payload["layout"]["segments"][2]["status"], "active");
    assert_eq!(payload["cursor"]["segment_id"], "segment-000002");
    assert_eq!(payload["cursor"]["line_cursor"], 3);
    drop(guard);
}

#[tokio::test]
async fn automation_journal_health_reports_state_marker_drift_and_missing_cursor_segment() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-health");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    fs::create_dir_all(
        loong_app::internal_events::internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        loong_app::internal_events::internal_event_journal_state_path(),
        concat!(
            "{\n",
            "  \"schema_version\": 1,\n",
            "  \"active_segment_id\": \"segment-000003\",\n",
            "  \"segments\": [\n",
            "    {\"segment_id\":\"segment-000001\",\"status\":\"sealed\"},\n",
            "    {\"segment_id\":\"segment-000003\",\"status\":\"active\"}\n",
            "  ]\n",
            "}\n"
        ),
    )
    .expect("write journal state");
    fs::write(
        loong_app::internal_events::internal_event_active_segment_id_path(),
        "segment-000004\n",
    )
    .expect("write divergent active marker");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"health-old\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write first segment");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000003"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"health-active\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write active segment");
    fs::write(
        automation_cursor_path(&loong_home),
        concat!(
            "{\n",
            "  \"segment_id\": \"segment-000005\",\n",
            "  \"line_cursor\": 1,\n",
            "  \"byte_offset\": 10,\n",
            "  \"journal_fingerprint\": \"missing\"\n",
            "}\n"
        ),
    )
    .expect("write missing cursor payload");

    let payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Journal(
                loong_daemon::automation_cli::AutomationJournalCommandOptions {
                    command: loong_daemon::automation_cli::AutomationJournalCommands::Health(
                        loong_daemon::automation_cli::AutomationJournalHealthCommandOptions::default(),
                    ),
                },
            ),
        },
    )
    .await
    .expect("journal health");

    assert_eq!(payload["command"], "journal_health");
    assert_eq!(payload["state_active_segment_id"], "segment-000003");
    assert_eq!(payload["active_marker_segment_id"], "segment-000004");
    assert_eq!(payload["cursor_segment_id"], "segment-000005");
    assert_eq!(payload["active_marker_matches_state"], false);
    assert_eq!(payload["active_segment_exists"], true);
    assert_eq!(payload["cursor_segment_exists"], false);
    drop(guard);
}

#[tokio::test]
async fn automation_journal_rotate_updates_active_segment_for_future_appends() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-rotate");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    fs::create_dir_all(
        loong_app::internal_events::internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        loong_app::internal_events::internal_event_journal_state_path(),
        concat!(
            "{\n",
            "  \"schema_version\": 1,\n",
            "  \"active_segment_id\": \"segment-000001\",\n",
            "  \"segments\": [\n",
            "    {\"segment_id\":\"segment-000001\",\"status\":\"active\",\"created_at_ms\":10}\n",
            "  ]\n",
            "}\n"
        ),
    )
    .expect("write initial journal state");

    let payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Journal(
                loong_daemon::automation_cli::AutomationJournalCommandOptions {
                    command: loong_daemon::automation_cli::AutomationJournalCommands::Rotate(
                        loong_daemon::automation_cli::AutomationJournalRotateCommandOptions::default(),
                    ),
                },
            ),
        },
    )
    .await
    .expect("rotate automation journal");

    assert_eq!(payload["command"], "journal_rotate");
    assert_eq!(payload["next_segment_id"], "segment-000002");
    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:journal-rotate"
        }),
    )
    .expect("append after journal rotate");
    let active_contents = fs::read_to_string(
        loong_app::internal_events::internal_event_segment_path("segment-000002"),
    )
    .expect("read rotated active segment");
    assert!(
        active_contents.contains("delegate:journal-rotate"),
        "future appends should land in the rotated active segment: {active_contents}"
    );
    drop(guard);
}

#[tokio::test]
async fn automation_journal_prune_uses_current_cursor_segment_as_floor() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-prune");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    fs::create_dir_all(
        loong_app::internal_events::internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        loong_app::internal_events::internal_event_journal_state_path(),
        concat!(
            "{\n",
            "  \"schema_version\": 1,\n",
            "  \"active_segment_id\": \"segment-000003\",\n",
            "  \"segments\": [\n",
            "    {\"segment_id\":\"segment-000001\",\"status\":\"sealed\"},\n",
            "    {\"segment_id\":\"segment-000002\",\"status\":\"sealed\"},\n",
            "    {\"segment_id\":\"segment-000003\",\"status\":\"active\"}\n",
            "  ]\n",
            "}\n"
        ),
    )
    .expect("write journal state");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"prune-a\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write first sealed segment");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000002"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"prune-b\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write second sealed segment");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000003"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"prune-c\"},\"recorded_at_ms\":3}\n",
    )
    .expect("write active segment");
    fs::write(
        automation_cursor_path(&loong_home),
        concat!(
            "{\n",
            "  \"segment_id\": \"segment-000002\",\n",
            "  \"line_cursor\": 1,\n",
            "  \"byte_offset\": 10,\n",
            "  \"journal_fingerprint\": \"abc\"\n",
            "}\n"
        ),
    )
    .expect("write cursor payload");

    let payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Journal(
                loong_daemon::automation_cli::AutomationJournalCommandOptions {
                    command: loong_daemon::automation_cli::AutomationJournalCommands::Prune(
                        loong_daemon::automation_cli::AutomationJournalPruneCommandOptions {
                            retain_segment_id: None,
                        },
                    ),
                },
            ),
        },
    )
    .await
    .expect("prune automation journal");

    assert_eq!(payload["command"], "journal_prune");
    assert_eq!(payload["pruned_segments"][0], "segment-000001");
    assert!(!loong_app::internal_events::internal_event_segment_path("segment-000001").exists());
    assert!(loong_app::internal_events::internal_event_segment_path("segment-000002").exists());
    assert!(loong_app::internal_events::internal_event_segment_path("segment-000003").exists());
    drop(guard);
}

#[tokio::test]
async fn automation_journal_repair_reconciles_state_and_reports_updated_layout() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-repair");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    fs::create_dir_all(
        loong_app::internal_events::internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        loong_app::internal_events::internal_event_journal_state_path(),
        concat!(
            "{\n",
            "  \"schema_version\": 1,\n",
            "  \"active_segment_id\": \"segment-000003\",\n",
            "  \"segments\": [\n",
            "    {\"segment_id\":\"segment-000001\",\"status\":\"sealed\",\"created_at_ms\":10,\"sealed_at_ms\":20},\n",
            "    {\"segment_id\":\"segment-000003\",\"status\":\"active\",\"created_at_ms\":30}\n",
            "  ]\n",
            "}\n"
        ),
    )
    .expect("write stale journal state");
    fs::write(
        loong_app::internal_events::internal_event_active_segment_id_path(),
        "segment-000004\n",
    )
    .expect("write newer active marker");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"sealed\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write sealed segment");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000004"),
        "{\"event_name\":\"session.archived\",\"payload\":{\"session_id\":\"active\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write recovered active segment");

    let payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Journal(
                loong_daemon::automation_cli::AutomationJournalCommandOptions {
                    command: loong_daemon::automation_cli::AutomationJournalCommands::Repair(
                        loong_daemon::automation_cli::AutomationJournalRepairCommandOptions::default(),
                    ),
                },
            ),
        },
    )
    .await
    .expect("repair automation journal");

    assert_eq!(payload["command"], "journal_repair");
    assert_eq!(payload["layout"]["active_segment_id"], "segment-000004");
    assert_eq!(
        payload["layout"]["segments"][0]["segment_id"],
        "segment-000001"
    );
    assert_eq!(payload["layout"]["segments"][0]["status"], "sealed");
    assert_eq!(
        payload["layout"]["segments"][1]["segment_id"],
        "segment-000004"
    );
    assert_eq!(payload["layout"]["segments"][1]["status"], "active");
    drop(guard);
}

fn set_session_event_ts(config_path: &Path, session_id: &str, event_kind: &str, ts: i64) {
    let (_, config) =
        mvp::config::load(Some(config_path.to_string_lossy().as_ref())).expect("load config");
    let conn =
        rusqlite::Connection::open(&config.memory.sqlite_path).expect("open automation sqlite");
    conn.execute(
        "UPDATE session_events
         SET ts = ?3
         WHERE session_id = ?1 AND event_kind = ?2",
        params![session_id, event_kind, ts],
    )
    .expect("update session event ts");
}

fn seed_overdue_background_task(config_path: &Path, root_session_id: &str, task_id: &str) {
    let repo = load_session_repository(config_path);
    super::tasks_cli::ensure_root_session(&repo, root_session_id);
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: task_id.to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some(root_session_id.to_owned()),
        label: Some("Recover Me".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create overdue child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: task_id.to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some(root_session_id.to_owned()),
        payload_json: serde_json::json!({
            "task": "recover overdue task",
            "label": "Recover Me",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs() as i64;
    set_session_event_ts(config_path, task_id, "delegate_queued", now_ts - 90);
}

#[tokio::test]
async fn automation_cli_emit_matches_event_trigger_and_queues_background_task() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-cli-emit");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let create_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Build Ready Follow-up".to_owned(),
                    event: "build.ready".to_owned(),
                    json_pointer: None,
                    equals_json: None,
                    equals_text: None,
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "review the completed build".to_owned(),
                    label: Some("Automation Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create automation event trigger");

    let trigger_id = create_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();
    assert_eq!(create_payload["command"], "create_event");
    assert_eq!(
        create_payload["trigger"]["source"]["event"]["event_name"],
        "build.ready"
    );

    let emit_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Emit(
                loong_daemon::automation_cli::AutomationEmitCommandOptions {
                    event: "BUILD.READY".to_owned(),
                    payload_json: Some(r#"{"reason":"ready to ship"}"#.to_owned()),
                },
            ),
        },
    )
    .await
    .expect("emit automation event");

    let queued_task_id = emit_payload["results"][0]["queued_task_id"]
        .as_str()
        .expect("queued task id")
        .to_owned();
    assert_eq!(emit_payload["command"], "emit");
    assert_eq!(emit_payload["event_name"], "build.ready");
    assert_eq!(emit_payload["matched_count"], 1);
    assert_eq!(emit_payload["payload"]["reason"], "ready to ship");
    assert!(queued_task_id.starts_with("delegate:"));
    assert!(emit_payload["results"][0]["error"].is_null());

    let show_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show automation trigger");

    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert_eq!(show_payload["trigger"]["last_task_id"], queued_task_id);
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(
        show_payload["trigger"]["run_history"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        show_payload["trigger"]["run_history"][0]["queued_task_id"],
        queued_task_id
    );

    let repo = load_session_repository(&config_path);
    let child_session = wait_for_session_record(&repo, &queued_task_id);
    assert_eq!(child_session.parent_session_id.as_deref(), Some("ops-root"));
    assert_eq!(
        child_session.kind,
        mvp::session::repository::SessionKind::DelegateChild
    );
    drop(guard);
}

#[tokio::test]
async fn work_unit_create_emits_automation_event_and_queues_background_task() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-work-unit-automation");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "New Work Unit Follow-up".to_owned(),
                    event: "work_unit.created".to_owned(),
                    json_pointer: None,
                    equals_json: None,
                    equals_text: None,
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "triage the new work unit".to_owned(),
                    label: Some("Work Unit Automation".to_owned()),
                    timeout_seconds: Some(45),
                },
            ),
        },
    )
    .await
    .expect("create work-unit automation trigger");

    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let create_output = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "work-unit",
            "create",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--id",
            "wu-automation",
            "--kind",
            "feature",
            "--title",
            "Automation-covered work unit",
            "--description",
            "Verify work-unit automation event emission",
            "--status",
            "ready",
            "--priority",
            "high",
            "--max-attempts",
            "3",
            "--initial-backoff-ms",
            "1000",
            "--max-backoff-ms",
            "60000",
            "--next-run-at-ms",
            "1000",
            "--actor",
            "operator",
            "--source-kind",
            "manual",
            "--json",
        ])
        .output()
        .expect("spawn work-unit create process");
    assert!(
        create_output.status.success(),
        "create work unit with automation emission: status={:?} stdout={} stderr={}",
        create_output.status.code(),
        String::from_utf8_lossy(&create_output.stdout),
        String::from_utf8_lossy(&create_output.stderr),
    );

    let show_payload = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    let queued_task_id = show_payload["trigger"]["last_task_id"]
        .as_str()
        .expect("queued task id")
        .to_owned();
    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert!(queued_task_id.starts_with("delegate:"));
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(
        show_payload["trigger"]["run_history"][0]["source_kind"],
        "event"
    );

    let work_unit_repo = {
        let (_, config) =
            mvp::config::load(Some(config_path.to_string_lossy().as_ref())).expect("load config");
        let memory_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        mvp::work::repository::WorkUnitRepository::new(&memory_config)
            .expect("work unit repository")
    };
    let snapshot = work_unit_repo
        .load_work_unit_snapshot("wu-automation")
        .expect("load work unit snapshot")
        .expect("created work unit snapshot");
    assert_eq!(snapshot.work_unit.work_unit_id, "wu-automation");
    assert!(queued_task_id.starts_with("delegate:"));
    drop(guard);
}

#[tokio::test]
async fn work_unit_create_can_filter_on_app_layer_source_surface_metadata() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-work-unit-source-surface");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Work Unit App Surface Filter".to_owned(),
                    event: "work_unit.created".to_owned(),
                    json_pointer: Some("/_automation/source_surface".to_owned()),
                    equals_json: None,
                    equals_text: Some("app.work.repository".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on app-layer work unit event".to_owned(),
                    label: Some("Work Unit App Surface".to_owned()),
                    timeout_seconds: Some(45),
                },
            ),
        },
    )
    .await
    .expect("create work-unit app-surface trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let create_output = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "work-unit",
            "create",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--id",
            "wu-app-surface",
            "--kind",
            "feature",
            "--title",
            "App layer source surface",
            "--description",
            "Verify work-unit source surface metadata",
            "--status",
            "ready",
            "--priority",
            "high",
            "--max-attempts",
            "3",
            "--initial-backoff-ms",
            "1000",
            "--max-backoff-ms",
            "60000",
            "--next-run-at-ms",
            "1000",
            "--actor",
            "operator",
            "--source-kind",
            "manual",
            "--json",
        ])
        .output()
        .expect("spawn work-unit create process");
    assert!(
        create_output.status.success(),
        "work-unit create app-surface case failed: status={:?} stdout={} stderr={}",
        create_output.status.code(),
        String::from_utf8_lossy(&create_output.stdout),
        String::from_utf8_lossy(&create_output.stderr),
    );

    let show_payload = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(
        show_payload["trigger"]["run_history"][0]["source_kind"],
        "event"
    );
    drop(guard);
}

#[tokio::test]
async fn automation_cli_create_cron_persists_next_fire_cursor() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-cli-cron");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let before_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_millis() as i64;

    let create_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateCron(
                loong_daemon::automation_cli::AutomationCreateCronCommandOptions {
                    name: "Nightly Ops Sweep".to_owned(),
                    cron: "0 0 * * *".to_owned(),
                    session: "ops-root".to_owned(),
                    task: "run nightly ops sweep".to_owned(),
                    label: Some("Nightly Sweep".to_owned()),
                    timeout_seconds: Some(60),
                },
            ),
        },
    )
    .await
    .expect("create cron automation trigger");

    assert_eq!(create_payload["command"], "create_cron");
    assert_eq!(create_payload["trigger"]["source"]["type"], "cron");
    assert_eq!(
        create_payload["trigger"]["source"]["cron"]["expression"],
        "0 0 * * *"
    );
    let next_fire_at_ms = create_payload["trigger"]["source"]["cron"]["next_fire_at_ms"]
        .as_i64()
        .expect("next fire at ms");
    assert!(
        next_fire_at_ms > before_ms,
        "cron cursor should point into the future, got {next_fire_at_ms} <= {before_ms}"
    );

    let trigger_id = create_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();
    let show_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show cron automation trigger");

    assert_eq!(show_payload["trigger"]["source"]["type"], "cron");
    assert_eq!(
        show_payload["trigger"]["source"]["cron"]["next_fire_at_ms"],
        next_fire_at_ms
    );
    assert_eq!(show_payload["trigger"]["fire_count"], 0);
    drop(guard);
}

#[tokio::test]
async fn automation_cli_event_filter_exists_and_contains_text_gate_delivery() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-cli-filter");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let exists_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Exists Filter".to_owned(),
                    event: "build.ready".to_owned(),
                    json_pointer: Some("/reason".to_owned()),
                    equals_json: None,
                    equals_text: None,
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "exists filter fired".to_owned(),
                    label: Some("Exists Filter".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create exists filter trigger");
    let exists_trigger_id = exists_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("exists trigger id")
        .to_owned();

    let contains_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Contains Filter".to_owned(),
                    event: "build.ready".to_owned(),
                    json_pointer: Some("/reason".to_owned()),
                    equals_json: None,
                    equals_text: None,
                    contains_text: Some("ship".to_owned()),
                    session: "ops-root".to_owned(),
                    task: "contains filter fired".to_owned(),
                    label: Some("Contains Filter".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create contains filter trigger");
    let contains_trigger_id = contains_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("contains trigger id")
        .to_owned();

    let miss_emit = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Emit(
                loong_daemon::automation_cli::AutomationEmitCommandOptions {
                    event: "build.ready".to_owned(),
                    payload_json: Some(r#"{"other":"value"}"#.to_owned()),
                },
            ),
        },
    )
    .await
    .expect("emit non-matching exists payload");
    assert_eq!(miss_emit["matched_count"], 0);

    let partial_emit = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Emit(
                loong_daemon::automation_cli::AutomationEmitCommandOptions {
                    event: "build.ready".to_owned(),
                    payload_json: Some(r#"{"reason":"ready to review"}"#.to_owned()),
                },
            ),
        },
    )
    .await
    .expect("emit exists-only matching payload");
    assert_eq!(partial_emit["matched_count"], 1);

    let full_emit = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Emit(
                loong_daemon::automation_cli::AutomationEmitCommandOptions {
                    event: "build.ready".to_owned(),
                    payload_json: Some(r#"{"reason":"ready to ship"}"#.to_owned()),
                },
            ),
        },
    )
    .await
    .expect("emit full matching payload");
    assert_eq!(full_emit["matched_count"], 2);

    let exists_show = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions {
                    id: exists_trigger_id,
                },
            ),
        },
    )
    .await
    .expect("show exists trigger");
    assert_eq!(exists_show["trigger"]["fire_count"], 2);
    assert_eq!(
        exists_show["trigger"]["source"]["event"]["json_pointer"],
        "/reason"
    );

    let contains_show = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions {
                    id: contains_trigger_id,
                },
            ),
        },
    )
    .await
    .expect("show contains trigger");
    assert_eq!(contains_show["trigger"]["fire_count"], 1);
    assert_eq!(
        contains_show["trigger"]["source"]["event"]["contains_text"],
        "ship"
    );
    drop(guard);
}

#[tokio::test]
async fn background_task_cancel_emits_automation_event_and_queues_followup() {
    let guard = lock_automation_integration();
    let root = super::tasks_cli::TempDirGuard::new("loong-background-task-cancel-automation");
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);
    super::tasks_cli::seed_background_task(&config_path, "ops-root", "delegate:cancel-me");

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Cancelled Follow-up".to_owned(),
                    event: "background_task.cancelled".to_owned(),
                    json_pointer: Some("/task/task_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:cancel-me".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on cancelled background task".to_owned(),
                    label: Some("Cancellation Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create background-task cancelled automation trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let cancel_payload = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Cancel {
                task_id: "delegate:cancel-me".to_owned(),
                dry_run: false,
            },
        },
    )
    .await
    .expect("cancel background task");

    assert_eq!(cancel_payload.payload["command"], "cancel");
    assert_eq!(
        cancel_payload.payload["action"]["kind"],
        "queued_async_cancelled"
    );

    let show_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show cancelled background-task trigger");

    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(
        show_payload["trigger"]["run_history"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let repo = load_session_repository(&config_path);
    let sessions = repo
        .list_sessions()
        .expect("list sessions after cancellation");
    let follow_up_sessions = sessions
        .iter()
        .filter(|session| {
            session.parent_session_id.as_deref() == Some("ops-root")
                && session.kind == mvp::session::repository::SessionKind::DelegateChild
                && session.session_id != "delegate:cancel-me"
        })
        .collect::<Vec<_>>();
    assert_eq!(follow_up_sessions.len(), 1);
    assert!(follow_up_sessions[0].session_id.starts_with("delegate:"));
    drop(guard);
}

#[tokio::test]
async fn background_task_cancel_can_filter_on_internal_source_surface_metadata() {
    let guard = lock_automation_integration();
    let root = super::tasks_cli::TempDirGuard::new("loong-background-task-cancel-source-surface");
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);
    super::tasks_cli::seed_background_task(&config_path, "ops-root", "delegate:meta-cancel");

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Tasks CLI Metadata Filter".to_owned(),
                    event: "background_task.cancelled".to_owned(),
                    json_pointer: Some("/_automation/source_surface".to_owned()),
                    equals_json: None,
                    equals_text: Some("tasks_cli".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on tasks cli sourced cancellation".to_owned(),
                    label: Some("Tasks CLI Metadata Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create metadata filtered trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let _cancel_payload = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Cancel {
                task_id: "delegate:meta-cancel".to_owned(),
                dry_run: false,
            },
        },
    )
    .await
    .expect("cancel metadata-filtered background task");

    let show_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show metadata filtered trigger");

    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(
        show_payload["trigger"]["run_history"][0]["source_kind"],
        "event"
    );
    drop(guard);
}

#[tokio::test]
async fn background_task_recover_emits_automation_event_and_queues_followup() {
    let guard = lock_automation_integration();
    let root = super::tasks_cli::TempDirGuard::new("loong-background-task-recover-automation");
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);
    seed_overdue_background_task(&config_path, "ops-root", "delegate:recover-me");

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Recovered Follow-up".to_owned(),
                    event: "background_task.recovered".to_owned(),
                    json_pointer: Some("/task/task_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:recover-me".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on recovered background task".to_owned(),
                    label: Some("Recovery Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create background-task recovered automation trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let recover_payload = loong_daemon::tasks_cli::execute_tasks_command(
        loong_daemon::tasks_cli::TasksCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::tasks_cli::TasksCommands::Recover {
                task_id: "delegate:recover-me".to_owned(),
                dry_run: false,
            },
        },
    )
    .await
    .expect("recover background task");

    assert_eq!(recover_payload.payload["command"], "recover");
    assert_eq!(
        recover_payload.payload["action"]["kind"],
        "queued_async_overdue_marked_failed"
    );

    let show_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show recovered background-task trigger");

    let queued_task_id = show_payload["trigger"]["last_task_id"]
        .as_str()
        .expect("queued task id")
        .to_owned();
    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert!(queued_task_id.starts_with("delegate:"));
    drop(guard);
}

#[tokio::test]
async fn session_cancel_emits_automation_event_and_queues_followup() {
    let guard = lock_automation_integration();
    let root = super::tasks_cli::TempDirGuard::new("loong-session-cancel-automation");
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);
    super::tasks_cli::seed_background_task(&config_path, "ops-root", "delegate:session-cancel");

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Session Cancelled Follow-up".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:session-cancel".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on cancelled session".to_owned(),
                    label: Some("Session Cancel Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create session cancelled automation trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let cancel_payload = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Cancel {
                session_id: "delegate:session-cancel".to_owned(),
                dry_run: false,
            },
        },
    )
    .await
    .expect("cancel session through sessions CLI");

    assert_eq!(cancel_payload.payload["command"], "cancel");
    assert_eq!(
        cancel_payload.payload["session_id"],
        "delegate:session-cancel"
    );
    assert_eq!(
        cancel_payload.payload["action"]["kind"],
        "queued_async_cancelled"
    );

    let show_payload = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;

    let queued_task_id = show_payload["trigger"]["last_task_id"]
        .as_str()
        .expect("queued task id")
        .to_owned();
    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(
        show_payload["trigger"]["run_history"][0]["source_kind"],
        "event"
    );
    assert!(queued_task_id.starts_with("delegate:"));
    drop(guard);
}

#[tokio::test]
async fn session_cancel_can_filter_on_app_layer_source_surface_metadata() {
    let guard = lock_automation_integration();
    let root = super::tasks_cli::TempDirGuard::new("loong-session-source-surface");
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);
    super::tasks_cli::seed_background_task(&config_path, "ops-root", "delegate:session-meta");

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Session App Surface Filter".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/_automation/source_surface".to_owned()),
                    equals_json: None,
                    equals_text: Some("app.tools.session".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on app-layer session event".to_owned(),
                    label: Some("Session App Surface".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create session app-surface trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let _cancel_payload = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Cancel {
                session_id: "delegate:session-meta".to_owned(),
                dry_run: false,
            },
        },
    )
    .await
    .expect("cancel session for app-surface case");

    let show_payload = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(
        show_payload["trigger"]["run_history"][0]["source_kind"],
        "event"
    );
    drop(guard);
}

#[tokio::test]
async fn session_archive_emits_automation_event_and_queues_followup() {
    let guard = lock_automation_integration();
    let root = super::tasks_cli::TempDirGuard::new("loong-session-archive-automation");
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);
    let repo = super::tasks_cli::load_session_repository(&config_path);
    super::tasks_cli::ensure_root_session(&repo, "ops-root");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "delegate:archive-me".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("ops-root".to_owned()),
        label: Some("Archive Me".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create archivable child session");
    repo.finalize_session_terminal(
        "delegate:archive-me",
        mvp::session::repository::FinalizeSessionTerminalRequest {
            state: mvp::session::repository::SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("ops-root".to_owned()),
            event_payload_json: json!({ "result": "ok" }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "delegate:archive-me",
                "result": "ok"
            }),
            frozen_result: None,
        },
    )
    .expect("finalize archivable child session");

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Session Archived Follow-up".to_owned(),
                    event: "session.archived".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:archive-me".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on archived session".to_owned(),
                    label: Some("Session Archive Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create session archived automation trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let archive_payload = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Archive {
                session_id: "delegate:archive-me".to_owned(),
                dry_run: false,
            },
        },
    )
    .await
    .expect("archive session through sessions CLI");

    assert_eq!(archive_payload.payload["command"], "archive");
    assert_eq!(archive_payload.payload["session_id"], "delegate:archive-me");
    assert_eq!(
        archive_payload.payload["action"]["kind"],
        "session_archived"
    );

    let show_payload = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;

    let queued_task_id = show_payload["trigger"]["last_task_id"]
        .as_str()
        .expect("queued task id")
        .to_owned();
    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert!(queued_task_id.starts_with("delegate:"));
    drop(guard);
}

#[tokio::test]
async fn session_recover_emits_automation_event_and_queues_followup() {
    let guard = lock_automation_integration();
    let root = super::tasks_cli::TempDirGuard::new("loong-session-recover-automation");
    let config_path = super::tasks_cli::write_tasks_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = super::tasks_cli::TasksCliEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);
    seed_overdue_background_task(&config_path, "ops-root", "delegate:session-recover");

    let create_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Session Recovered Follow-up".to_owned(),
                    event: "session.recovered".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:session-recover".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on recovered session".to_owned(),
                    label: Some("Session Recover Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create session recovered automation trigger");
    let trigger_id = create_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let recover_payload = loong_daemon::sessions_cli::execute_sessions_command(
        loong_daemon::sessions_cli::SessionsCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            session: "ops-root".to_owned(),
            command: loong_daemon::sessions_cli::SessionsCommands::Recover {
                session_id: "delegate:session-recover".to_owned(),
                dry_run: false,
            },
        },
    )
    .await
    .expect("recover session through sessions CLI");

    assert_eq!(recover_payload.payload["command"], "recover");
    assert_eq!(
        recover_payload.payload["session_id"],
        "delegate:session-recover"
    );
    assert_eq!(
        recover_payload.payload["action"]["kind"],
        "queued_async_overdue_marked_failed"
    );

    let show_payload = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    let queued_task_id = show_payload["trigger"]["last_task_id"]
        .as_str()
        .expect("queued task id")
        .to_owned();
    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert!(queued_task_id.starts_with("delegate:"));
    drop(guard);
}

#[tokio::test]
async fn automation_serve_processes_internal_journal_once_and_advances_cursor() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-cursor");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let create_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Journal Cursor Follow-up".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:cursor-test".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on journal cursor event".to_owned(),
                    label: Some("Journal Cursor Follow-up".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create journal cursor trigger");
    let trigger_id = create_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let mut serve = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "automation",
            "serve",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--poll-ms",
            "250",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn automation serve");

    wait_for_serve_lock(
        &mut serve,
        automation_serve_lock_path(&loong_home).as_path(),
    );

    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:cursor-test"
        }),
    )
    .expect("append internal journal row");

    let first_show = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    assert!(first_show["trigger"]["last_error"].is_null());
    wait_for_cursor_value(automation_cursor_path(&loong_home).as_path(), "1");
    let cursor_payload: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(automation_cursor_path(&loong_home)).expect("read cursor payload"),
    )
    .expect("parse cursor payload");
    assert_eq!(cursor_payload["line_cursor"], 1);
    assert!(
        cursor_payload["byte_offset"].as_u64().unwrap_or_default() > 0,
        "byte_offset should advance after the first consumed journal record"
    );

    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let second_show = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show trigger after replay window");
    assert_eq!(second_show["trigger"]["fire_count"], 1);

    serve.kill().expect("stop automation serve");
    let _ = serve.wait();
    drop(guard);
}

#[tokio::test]
async fn automation_serve_migrates_legacy_numeric_cursor_without_replaying_consumed_rows() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-legacy-cursor");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");
    let automation_dir = loong_home.join("automation");
    fs::create_dir_all(&automation_dir).expect("create automation dir");
    fs::write(automation_dir.join("internal-events.cursor"), "1\n")
        .expect("seed legacy numeric cursor");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let create_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Legacy Cursor Migration".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: None,
                    equals_json: None,
                    equals_text: None,
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on migrated cursor event".to_owned(),
                    label: Some("Legacy Cursor Migration".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create legacy cursor trigger");
    let trigger_id = create_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:first"
        }),
    )
    .expect("append first journal row");
    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:second"
        }),
    )
    .expect("append second journal row");

    let mut serve = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "automation",
            "serve",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--poll-ms",
            "250",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn automation serve");

    wait_for_serve_lock(
        &mut serve,
        automation_serve_lock_path(&loong_home).as_path(),
    );
    let show_payload = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    assert!(show_payload["trigger"]["last_error"].is_null());
    assert_eq!(show_payload["trigger"]["fire_count"], 1);
    let cursor_payload: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(automation_cursor_path(&loong_home)).expect("read cursor payload"),
    )
    .expect("parse cursor payload");
    assert_eq!(cursor_payload["line_cursor"], 2);
    assert!(
        cursor_payload["byte_offset"].as_u64().unwrap_or_default() > 0,
        "migrated cursor should persist a nonzero byte offset"
    );

    serve.kill().expect("stop automation serve");
    let _ = serve.wait();
    drop(guard);
}

#[tokio::test]
async fn automation_serve_matches_internal_journal_source_surface_metadata() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-source-surface");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let create_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Journal App Surface Filter".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/_automation/source_surface".to_owned()),
                    equals_json: None,
                    equals_text: Some("app.tools.session".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on journal source-surface event".to_owned(),
                    label: Some("Journal App Surface Filter".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create source-surface trigger");
    let trigger_id = create_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let mut serve = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "automation",
            "serve",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--poll-ms",
            "250",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn automation serve");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:wrong-surface",
            "_automation": {
                "event_name": "session.cancelled",
                "source_surface": "tasks_cli"
            }
        }),
    )
    .expect("append non-matching metadata journal row");
    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:matched-surface",
            "_automation": {
                "event_name": "session.cancelled",
                "source_surface": "app.tools.session"
            }
        }),
    )
    .expect("append matching metadata journal row");

    wait_for_cursor_value(automation_cursor_path(&loong_home).as_path(), "2");
    let cursor_payload: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(automation_cursor_path(&loong_home)).expect("read cursor payload"),
    )
    .expect("parse cursor payload");
    assert_eq!(cursor_payload["line_cursor"], 2);
    assert!(
        cursor_payload["byte_offset"].as_u64().unwrap_or_default() > 0,
        "byte_offset should advance after consuming journal metadata rows"
    );
    let show_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show metadata trigger after cursor advance");
    assert_eq!(
        show_payload["trigger"]["fire_count"], 1,
        "journal cursor advanced but metadata-filtered trigger did not fire"
    );
    assert!(show_payload["trigger"]["last_error"].is_null());

    serve.kill().expect("stop automation serve");
    let _ = serve.wait();
    drop(guard);
}

#[tokio::test]
async fn automation_serve_recovers_after_internal_journal_rotation_and_processes_new_rows_once() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-journal-rotation");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    let create_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Rotation Recovery".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: None,
                    contains_text: Some("rotation".to_owned()),
                    session: "ops-root".to_owned(),
                    task: "follow up on rotated journal event".to_owned(),
                    label: Some("Rotation Recovery".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create rotation recovery trigger");
    let trigger_id = create_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let mut serve = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "automation",
            "serve",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--poll-ms",
            "250",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn automation serve");

    wait_for_serve_lock(
        &mut serve,
        automation_serve_lock_path(&loong_home).as_path(),
    );

    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:rotation-first"
        }),
    )
    .expect("append first rotation row");

    let first_show = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    assert!(first_show["trigger"]["last_error"].is_null());

    let cursor_before_rotation: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(automation_cursor_path(&loong_home)).expect("read cursor payload"),
    )
    .expect("parse cursor payload");
    let fingerprint_before_rotation = cursor_before_rotation["journal_fingerprint"].clone();

    let journal_path = loong_app::internal_events::internal_event_journal_path();
    fs::write(
        &journal_path,
        concat!(
            "{\"event_name\":\"session.archived\",\"payload\":{\"session_id\":\"rotation-ignored-padding-with-longer-content\"},\"recorded_at_ms\":4}\n",
            "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"delegate:rotation-second\"},\"recorded_at_ms\":5}\n"
        ),
    )
    .expect("replace journal with rotated content");

    let second_show = wait_for_trigger_fire_count(&config_path, &trigger_id, 2).await;
    assert!(second_show["trigger"]["last_error"].is_null());
    let cursor_after_rotation: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(automation_cursor_path(&loong_home)).expect("read cursor payload"),
    )
    .expect("parse cursor payload");
    assert_eq!(cursor_after_rotation["line_cursor"], 2);
    assert!(
        cursor_after_rotation["byte_offset"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "rotation recovery should persist a nonzero byte offset"
    );
    assert_ne!(
        cursor_after_rotation["journal_fingerprint"], fingerprint_before_rotation,
        "rotation recovery should rewrite the cursor fingerprint"
    );

    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let third_show = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show trigger after rotation replay window");
    assert_eq!(third_show["trigger"]["fire_count"], 2);

    serve.kill().expect("stop automation serve");
    let _ = serve.wait();
    drop(guard);
}

#[tokio::test]
async fn automation_serve_processes_segment_rollover_once_without_skipping_or_replaying() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-segment-rollover");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    fs::create_dir_all(loong_app::internal_events::internal_event_segments_dir())
        .expect("create internal event segments dir");
    fs::write(
        loong_app::internal_events::internal_event_active_segment_id_path(),
        "segment-000001\n",
    )
    .expect("seed active segment id");

    let create_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Segment Rollover".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: None,
                    contains_text: Some("segment-rollover".to_owned()),
                    session: "ops-root".to_owned(),
                    task: "follow up on segment rollover event".to_owned(),
                    label: Some("Segment Rollover".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create segment rollover trigger");
    let trigger_id = create_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("trigger id")
        .to_owned();

    let mut serve = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "automation",
            "serve",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--poll-ms",
            "250",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn automation serve");

    wait_for_serve_lock(
        &mut serve,
        automation_serve_lock_path(&loong_home).as_path(),
    );

    append_internal_event_to_journal(
        "session.cancelled",
        &serde_json::json!({
            "session_id": "delegate:segment-rollover-old"
        }),
    )
    .expect("append first segment row");

    let first_show = wait_for_trigger_fire_count(&config_path, &trigger_id, 1).await;
    assert!(first_show["trigger"]["last_error"].is_null());

    fs::write(
        loong_app::internal_events::internal_event_active_segment_id_path(),
        "segment-000002\n",
    )
    .expect("promote second segment to active");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000002"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"delegate:segment-rollover-new\"},\"recorded_at_ms\":9}\n",
    )
    .expect("write second segment");

    let second_show = wait_for_trigger_fire_count(&config_path, &trigger_id, 2).await;
    assert!(second_show["trigger"]["last_error"].is_null());
    let cursor_payload: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(automation_cursor_path(&loong_home)).expect("read cursor payload"),
    )
    .expect("parse cursor payload");
    assert_eq!(cursor_payload["segment_id"], "segment-000002");
    assert_eq!(cursor_payload["line_cursor"], 1);
    assert!(
        !loong_app::internal_events::internal_event_segment_path("segment-000001").exists(),
        "serve should prune fully consumed sealed segments once the cursor is persisted on the newer segment"
    );
    assert!(
        loong_app::internal_events::internal_event_segment_path("segment-000002").exists(),
        "active segment should remain after pruning older sealed segments"
    );

    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let third_show = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions { id: trigger_id },
            ),
        },
    )
    .await
    .expect("show trigger after segment replay window");
    assert_eq!(third_show["trigger"]["fire_count"], 2);

    serve.kill().expect("stop automation serve");
    let _ = serve.wait();
    drop(guard);
}

#[tokio::test]
async fn automation_serve_missing_segment_cursor_skips_older_surviving_segment() {
    let guard = lock_automation_integration();
    let root = TempDirGuard::new("loong-automation-missing-segment-cursor");
    let config_path = write_automation_config(root.path());
    let loong_home = root.path().join("loong-home");
    fs::create_dir_all(&loong_home).expect("create loong home");

    let loong_home_text = loong_home.display().to_string();
    let detached_binary = env!("CARGO_BIN_EXE_loong").to_owned();
    let _env = MigrationEnvironmentGuard::set(&[
        ("LOONG_HOME", Some(loong_home_text.as_str())),
        ("CARGO_BIN_EXE_loong", Some(detached_binary.as_str())),
    ]);

    fs::create_dir_all(loong_app::internal_events::internal_event_segments_dir())
        .expect("create internal event segments dir");
    fs::write(
        loong_app::internal_events::internal_event_active_segment_id_path(),
        "segment-000003\n",
    )
    .expect("seed active segment id");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"delegate:missing-segment-older\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write older surviving segment");
    fs::write(
        loong_app::internal_events::internal_event_segment_path("segment-000003"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"delegate:missing-segment-newer\"},\"recorded_at_ms\":3}\n",
    )
    .expect("write newer surviving segment");
    fs::write(
        automation_cursor_path(&loong_home),
        serde_json::to_string_pretty(&serde_json::json!({
            "segment_id": "segment-000002",
            "line_cursor": 5,
            "byte_offset": 99,
            "journal_fingerprint": "stale"
        }))
        .expect("serialize missing segment cursor"),
    )
    .expect("seed missing segment cursor");

    let older_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Older Missing Segment".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:missing-segment-older".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on older missing-segment event".to_owned(),
                    label: Some("Older Missing Segment".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create older missing segment trigger");
    let older_trigger_id = older_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("older trigger id")
        .to_owned();

    let newer_trigger_payload = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::CreateEvent(
                loong_daemon::automation_cli::AutomationCreateEventCommandOptions {
                    name: "Newer Missing Segment".to_owned(),
                    event: "session.cancelled".to_owned(),
                    json_pointer: Some("/session_id".to_owned()),
                    equals_json: None,
                    equals_text: Some("delegate:missing-segment-newer".to_owned()),
                    contains_text: None,
                    session: "ops-root".to_owned(),
                    task: "follow up on newer missing-segment event".to_owned(),
                    label: Some("Newer Missing Segment".to_owned()),
                    timeout_seconds: Some(30),
                },
            ),
        },
    )
    .await
    .expect("create newer missing segment trigger");
    let newer_trigger_id = newer_trigger_payload["trigger"]["trigger_id"]
        .as_str()
        .expect("newer trigger id")
        .to_owned();

    let mut serve = Command::new(env!("CARGO_BIN_EXE_loong"))
        .env("LOONG_HOME", loong_home_text.as_str())
        .env("CARGO_BIN_EXE_loong", detached_binary.as_str())
        .args([
            "automation",
            "serve",
            "--config",
            config_path.to_string_lossy().as_ref(),
            "--poll-ms",
            "250",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn automation serve");

    wait_for_serve_lock(
        &mut serve,
        automation_serve_lock_path(&loong_home).as_path(),
    );

    let newer_show = wait_for_trigger_fire_count(&config_path, &newer_trigger_id, 1).await;
    assert!(newer_show["trigger"]["last_error"].is_null());
    let older_show = loong_daemon::automation_cli::execute_automation_command(
        loong_daemon::automation_cli::AutomationCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: loong_daemon::automation_cli::AutomationCommands::Show(
                loong_daemon::automation_cli::AutomationShowCommandOptions {
                    id: older_trigger_id,
                },
            ),
        },
    )
    .await
    .expect("show older trigger after missing segment normalization");
    assert_eq!(older_show["trigger"]["fire_count"], 0);

    let cursor_payload: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(automation_cursor_path(&loong_home)).expect("read cursor payload"),
    )
    .expect("parse cursor payload");
    assert_eq!(cursor_payload["segment_id"], "segment-000003");
    assert_eq!(cursor_payload["line_cursor"], 1);

    serve.kill().expect("stop automation serve");
    let _ = serve.wait();
    drop(guard);
}
