#[cfg(feature = "memory-sqlite")]
use std::path::PathBuf;

#[cfg(feature = "memory-sqlite")]
use rusqlite::Connection;

#[cfg(feature = "memory-sqlite")]
use crate::config::MemoryConfig;

#[cfg(feature = "memory-sqlite")]
/// Transitional session-store adapter over the existing memory SQLite substrate.
///
/// This layer intentionally gives session-core callers one stable namespace for
/// transcript and session durability while the underlying persistence backend
/// still lives in `memory::*`.
pub type SessionStoreConfig = crate::memory::runtime_config::MemoryRuntimeConfig;

#[cfg(feature = "memory-sqlite")]
pub type SessionTranscriptTurn = crate::memory::ConversationTurn;

#[cfg(feature = "memory-sqlite")]
pub type SessionWindowTurn = crate::memory::WindowTurn;

#[cfg(feature = "memory-sqlite")]
pub fn session_store_config_from_memory_config(config: &MemoryConfig) -> SessionStoreConfig {
    SessionStoreConfig::from_memory_config(config)
}

#[cfg(feature = "memory-sqlite")]
pub fn session_store_config_from_memory_config_without_env_overrides(
    config: &MemoryConfig,
) -> SessionStoreConfig {
    SessionStoreConfig::from_memory_config_without_env_overrides(config)
}

#[cfg(feature = "memory-sqlite")]
pub fn current_session_store_config() -> &'static SessionStoreConfig {
    crate::memory::runtime_config::get_memory_runtime_config()
}

#[cfg(feature = "memory-sqlite")]
pub fn ensure_session_store_ready(
    path: Option<PathBuf>,
    config: &SessionStoreConfig,
) -> Result<PathBuf, String> {
    crate::memory::ensure_memory_db_ready(path, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn append_session_turn_direct(
    session_id: &str,
    role: &str,
    content: &str,
    config: &SessionStoreConfig,
) -> Result<(), String> {
    crate::memory::append_turn_direct(session_id, role, content, config)
}

#[cfg(all(test, feature = "memory-sqlite"))]
pub fn replace_session_turns_direct(
    session_id: &str,
    turns: &[SessionWindowTurn],
    config: &SessionStoreConfig,
) -> Result<(), String> {
    crate::memory::replace_session_turns_direct(session_id, turns, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn window_session_turns(
    session_id: &str,
    limit: usize,
    config: &SessionStoreConfig,
) -> Result<Vec<SessionTranscriptTurn>, String> {
    crate::memory::window_direct(session_id, limit, config)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn window_session_turns_with_conn(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<Vec<SessionTranscriptTurn>, String> {
    crate::memory::window_direct_with_conn(conn, session_id, limit)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn transcript_session_turns_paged_with_conn(
    conn: &Connection,
    session_id: &str,
    page_size: usize,
) -> Result<Vec<SessionTranscriptTurn>, String> {
    crate::memory::transcript_direct_paged_with_conn(conn, session_id, page_size)
}

#[cfg(all(test, feature = "memory-sqlite"))]
mod tests {
    use crate::session::store::{
        SessionStoreConfig, append_session_turn_direct, ensure_session_store_ready,
        window_session_turns,
    };
    use crate::test_support::unique_temp_dir;

    #[test]
    fn session_store_facade_round_trips_transcript_turns() {
        let root = unique_temp_dir("session-store-facade");
        std::fs::create_dir_all(&root).expect("create session store test root");
        let sqlite_path = root.join("memory.sqlite3");
        let config = SessionStoreConfig {
            sqlite_path: Some(sqlite_path.clone()),
            ..SessionStoreConfig::default()
        };

        ensure_session_store_ready(Some(sqlite_path), &config).expect("initialize session store");
        append_session_turn_direct("session-store-test", "user", "hello", &config)
            .expect("append turn");
        let turns =
            window_session_turns("session-store-test", 8, &config).expect("load session turns");

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].content, "hello");
    }
}
