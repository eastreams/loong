use super::*;
use rusqlite::{Connection, params};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static LATEST_SELECTOR_FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let counter = LATEST_SELECTOR_FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let process_id = std::process::id();
    let temp_dir = std::env::temp_dir();
    temp_dir.join(format!(
        "loongclaw-latest-selector-{label}-{process_id}-{nanos}-{counter}"
    ))
}

fn cleanup_sqlite_artifacts(sqlite_path: &Path) {
    let sqlite_path = sqlite_path.display().to_string();
    let wal_path = format!("{sqlite_path}-wal");
    let shm_path = format!("{sqlite_path}-shm");
    let _ = std::fs::remove_file(sqlite_path);
    let _ = std::fs::remove_file(wal_path);
    let _ = std::fs::remove_file(shm_path);
}

fn open_sqlite_connection(sqlite_path: &Path) -> Connection {
    Connection::open(sqlite_path).expect("open latest selector sqlite connection")
}

pub(super) struct LatestSelectorCliFixture {
    _lock: MutexGuard<'static, ()>,
    root: PathBuf,
    home_dir: PathBuf,
    config_path: PathBuf,
    sqlite_path: PathBuf,
    memory_config: mvp::memory::runtime_config::MemoryRuntimeConfig,
}

impl LatestSelectorCliFixture {
    pub(super) fn new(label: &str) -> Self {
        let lock = lock_daemon_test_environment();
        let root = unique_temp_path(label);
        let home_dir = root.join("home");
        let config_path = root.join("loongclaw.toml");
        let sqlite_path = root.join("memory.sqlite3");
        std::fs::create_dir_all(&home_dir).expect("create latest selector fixture home");
        cleanup_sqlite_artifacts(&sqlite_path);

        let mut config = mvp::config::LoongClawConfig::default();
        config.memory.sqlite_path = sqlite_path.display().to_string();
        let memory_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let resolved_sqlite_path = config.memory.resolved_sqlite_path();
        mvp::memory::ensure_memory_db_ready(Some(resolved_sqlite_path), &memory_config)
            .expect("initialize latest selector sqlite memory");

        Self {
            _lock: lock,
            root,
            home_dir,
            config_path,
            sqlite_path,
            memory_config,
        }
    }

    pub(super) fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub(super) fn sqlite_path(&self) -> &Path {
        &self.sqlite_path
    }

    pub(super) fn write_config_with(
        &self,
        configure: impl FnOnce(&mut mvp::config::LoongClawConfig),
    ) -> mvp::config::LoongClawConfig {
        let mut config = mvp::config::LoongClawConfig::default();
        config.memory.sqlite_path = self.sqlite_path.display().to_string();
        config.memory.sliding_window = 8;
        config.tools.file_root = Some(self.root.display().to_string());
        configure(&mut config);
        let config_path_text = self.config_path.to_string_lossy();
        let config_path_text = config_path_text.as_ref();
        mvp::config::write(Some(config_path_text), &config, true).expect("write config fixture");
        config
    }

    fn session_repository(&self) -> mvp::session::repository::SessionRepository {
        mvp::session::repository::SessionRepository::new(&self.memory_config)
            .expect("session repository")
    }

    pub(super) fn create_root_session(&self, session_id: &str) {
        let repo = self.session_repository();
        let record = mvp::session::repository::NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: mvp::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some(session_id.to_owned()),
            state: mvp::session::repository::SessionState::Ready,
        };
        repo.create_session(record).expect("create root session");
    }

    pub(super) fn create_delegate_child_session(&self, session_id: &str, parent_session_id: &str) {
        let repo = self.session_repository();
        let record = mvp::session::repository::NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: mvp::session::repository::SessionKind::DelegateChild,
            parent_session_id: Some(parent_session_id.to_owned()),
            label: Some(session_id.to_owned()),
            state: mvp::session::repository::SessionState::Ready,
        };
        repo.create_session(record)
            .expect("create delegate child session");
    }

    pub(super) fn append_session_turn(&self, session_id: &str, role: &str, content: &str) {
        mvp::memory::append_turn_direct(session_id, role, content, &self.memory_config)
            .expect("append session turn");
    }

    pub(super) fn set_session_updated_at(&self, session_id: &str, updated_at: i64) {
        let conn = open_sqlite_connection(&self.sqlite_path);
        conn.execute(
            "UPDATE sessions
             SET updated_at = ?2
             WHERE session_id = ?1",
            params![session_id, updated_at],
        )
        .expect("set session updated_at");
    }

    pub(super) fn set_turn_timestamps(&self, session_id: &str, ts: i64) {
        let conn = open_sqlite_connection(&self.sqlite_path);
        conn.execute(
            "UPDATE turns
             SET ts = ?2
             WHERE session_id = ?1",
            params![session_id, ts],
        )
        .expect("set turn timestamps");
    }

    pub(super) fn archive_session(&self, session_id: &str, archived_at: i64) {
        let conn = open_sqlite_connection(&self.sqlite_path);
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
        .expect("insert session archive event");
    }

    pub(super) fn run_process(&self, args: &[&str], stdin_bytes: Option<&[u8]>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_loongclaw"));
        command
            .current_dir(&self.root)
            .env("HOME", &self.home_dir)
            .env_remove("LOONGCLAW_CONFIG_PATH")
            .env_remove("USERPROFILE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.args(args);
        command.arg("--config");
        command.arg(&self.config_path);

        let mut child = command.spawn().expect("spawn latest selector CLI");
        if let Some(stdin_bytes) = stdin_bytes {
            let stdin = child.stdin.as_mut().expect("latest selector stdin");
            stdin
                .write_all(stdin_bytes)
                .expect("write latest selector stdin");
        }
        drop(child.stdin.take());
        child
            .wait_with_output()
            .expect("wait for latest selector CLI output")
    }
}

impl Drop for LatestSelectorCliFixture {
    fn drop(&mut self) {
        cleanup_sqlite_artifacts(&self.sqlite_path);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
