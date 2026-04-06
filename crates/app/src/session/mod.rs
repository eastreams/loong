#[cfg(feature = "memory-sqlite")]
pub mod recovery;

#[cfg(feature = "memory-sqlite")]
pub mod repository;

#[cfg(feature = "memory-sqlite")]
pub mod frozen_result;

#[cfg(feature = "memory-sqlite")]
pub const LATEST_SESSION_SELECTOR: &str = "latest";

#[cfg(feature = "memory-sqlite")]
pub fn latest_resumable_root_session_id(
    memory_config: &crate::memory::runtime_config::MemoryRuntimeConfig,
) -> crate::CliResult<Option<String>> {
    let repo = repository::SessionRepository::new(memory_config)?;
    let latest_session = repo.latest_resumable_root_session_summary()?;
    let latest_session_id = latest_session.map(|summary| summary.session_id);
    Ok(latest_session_id)
}

#[allow(dead_code)]
pub(crate) const DELEGATE_CANCEL_REQUESTED_EVENT_KIND: &str = "delegate_cancel_requested";
#[allow(dead_code)]
pub(crate) const DELEGATE_CANCELLED_EVENT_KIND: &str = "delegate_cancelled";
#[allow(dead_code)]
pub(crate) const DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED: &str = "operator_requested";
#[allow(dead_code)]
pub(crate) const DELEGATE_CANCELLED_ERROR_PREFIX: &str = "delegate_cancelled:";

#[allow(dead_code)]
pub(crate) fn delegate_cancelled_error(reason: &str) -> String {
    format!(
        "{DELEGATE_CANCELLED_ERROR_PREFIX} {}",
        reason.trim().trim_matches(':')
    )
}

#[allow(dead_code)]
pub(crate) fn parse_delegate_cancelled_reason(error: &str) -> Option<String> {
    error
        .strip_prefix(DELEGATE_CANCELLED_ERROR_PREFIX)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(all(test, feature = "memory-sqlite"))]
#[allow(clippy::expect_used)]
mod latest_cli_session_selector_tests {
    use super::LATEST_SESSION_SELECTOR;
    use super::latest_resumable_root_session_id;
    use crate::memory;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::NewSessionRecord;
    use crate::session::repository::SessionKind;
    use crate::session::repository::SessionRepository;
    use crate::session::repository::SessionState;
    use crate::test_support::unique_temp_dir;
    use rusqlite::Connection;
    use rusqlite::params;
    use std::path::Path;
    use std::path::PathBuf;

    fn init_selector_test_memory(label: &str) -> (PathBuf, MemoryRuntimeConfig) {
        let root = unique_temp_dir(label);
        std::fs::create_dir_all(&root).expect("create selector test workspace");

        let sqlite_path = root.join("memory.sqlite3");
        let config = MemoryRuntimeConfig {
            sqlite_path: Some(sqlite_path.clone()),
            ..MemoryRuntimeConfig::default()
        };

        memory::ensure_memory_db_ready(Some(sqlite_path), &config)
            .expect("initialize selector test memory");

        (root, config)
    }

    fn cleanup_selector_test_memory(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    fn create_root_session(repo: &SessionRepository, session_id: &str) {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
    }

    fn append_session_turn(
        memory_config: &MemoryRuntimeConfig,
        session_id: &str,
        role: &str,
        content: &str,
    ) {
        memory::append_turn_direct(session_id, role, content, memory_config)
            .expect("append selector test turn");
    }

    fn set_session_updated_at(sqlite_path: &Path, session_id: &str, updated_at: i64) {
        let conn = Connection::open(sqlite_path).expect("open selector sqlite connection");
        conn.execute(
            "UPDATE sessions
             SET updated_at = ?2
             WHERE session_id = ?1",
            params![session_id, updated_at],
        )
        .expect("set selector updated_at");
    }

    fn archive_session(sqlite_path: &Path, session_id: &str, archived_at: i64) {
        let conn = Connection::open(sqlite_path).expect("open selector sqlite connection");
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
        .expect("archive selector session");
    }

    #[test]
    fn latest_cli_session_selector_returns_newest_resumable_root_session_id() {
        let (root, memory_config) = init_selector_test_memory("latest-cli-selector");
        let sqlite_path = memory_config
            .sqlite_path
            .clone()
            .expect("selector sqlite path");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        assert_eq!(LATEST_SESSION_SELECTOR, "latest");

        create_root_session(&repo, "root-old");
        append_session_turn(&memory_config, "root-old", "user", "old");
        set_session_updated_at(&sqlite_path, "root-old", 100);

        create_root_session(&repo, "root-new");
        append_session_turn(&memory_config, "root-new", "user", "new");
        set_session_updated_at(&sqlite_path, "root-new", 200);

        create_root_session(&repo, "root-archived");
        append_session_turn(&memory_config, "root-archived", "assistant", "archived");
        set_session_updated_at(&sqlite_path, "root-archived", 300);
        archive_session(&sqlite_path, "root-archived", 400);

        let selected_session_id = latest_resumable_root_session_id(&memory_config)
            .expect("resolve latest session id")
            .expect("selected session id");

        assert_eq!(selected_session_id, "root-new");

        cleanup_selector_test_memory(&root);
    }

    #[test]
    fn latest_cli_session_selector_returns_none_without_resumable_root_session() {
        let (root, memory_config) = init_selector_test_memory("latest-cli-selector-none");
        let sqlite_path = memory_config
            .sqlite_path
            .clone()
            .expect("selector sqlite path");
        let repo = SessionRepository::new(&memory_config).expect("selector repository");

        create_root_session(&repo, "root-empty");
        set_session_updated_at(&sqlite_path, "root-empty", 100);

        create_root_session(&repo, "root-archived");
        append_session_turn(&memory_config, "root-archived", "assistant", "archived");
        set_session_updated_at(&sqlite_path, "root-archived", 200);
        archive_session(&sqlite_path, "root-archived", 300);

        let selected_session_id =
            latest_resumable_root_session_id(&memory_config).expect("resolve latest session id");

        assert!(selected_session_id.is_none());

        cleanup_selector_test_memory(&root);
    }
}
