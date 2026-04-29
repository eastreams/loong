#[cfg(feature = "memory-sqlite")]
use std::path::PathBuf;
#[cfg(feature = "memory-sqlite")]
use std::sync::OnceLock;

#[cfg(feature = "memory-sqlite")]
use rusqlite::Connection;

#[cfg(feature = "memory-sqlite")]
use crate::config::MemoryConfig;
#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::memory::{HydratedMemoryContext, MemoryContextEntry, StageDiagnostics};

#[cfg(feature = "memory-sqlite")]
/// Transitional session-store adapter over the existing memory SQLite substrate.
///
/// This layer intentionally gives session-core callers one stable namespace for
/// transcript and session durability while the underlying persistence backend
/// still lives in `memory::*`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionStoreConfig {
    /// Session-owned persistence location override.
    ///
    /// When `runtime_config` is present this path still wins, so session-facing
    /// callers can redirect durability without losing the rest of the selected
    /// memory runtime policy.
    pub sqlite_path: Option<PathBuf>,
    /// Session-facing snapshot of the effective memory runtime policy.
    ///
    /// This intentionally preserves more than the SQLite path so summary,
    /// retrieval, and fail-open behavior stay stable across the session-store
    /// boundary.
    pub runtime_config: Option<MemoryRuntimeConfig>,
}

#[cfg(feature = "memory-sqlite")]
impl SessionStoreConfig {
    pub fn for_sqlite_path(sqlite_path: impl Into<PathBuf>) -> Self {
        let sqlite_path = sqlite_path.into();
        let runtime_config = MemoryRuntimeConfig::for_sqlite_path(sqlite_path.clone());
        Self {
            sqlite_path: Some(sqlite_path),
            runtime_config: Some(runtime_config),
        }
    }

    pub fn from_memory_config(config: &MemoryConfig) -> Self {
        Self {
            sqlite_path: Some(config.resolved_sqlite_path()),
            runtime_config: Some(MemoryRuntimeConfig::from_memory_config(config)),
        }
    }

    pub fn from_memory_config_without_env_overrides(config: &MemoryConfig) -> Self {
        Self {
            sqlite_path: Some(config.resolved_sqlite_path()),
            runtime_config: Some(
                MemoryRuntimeConfig::from_memory_config_without_env_overrides(config),
            ),
        }
    }

    pub fn from_memory_runtime_config(config: &MemoryRuntimeConfig) -> Self {
        Self {
            sqlite_path: config.sqlite_path.clone(),
            runtime_config: Some(config.clone()),
        }
    }

    fn as_memory_runtime_config(&self) -> MemoryRuntimeConfig {
        match self.runtime_config.as_ref() {
            Some(runtime_config) => {
                let mut runtime_config = runtime_config.clone();
                runtime_config.sqlite_path = self.sqlite_path.clone();
                runtime_config
            }
            None => match self.sqlite_path.as_ref() {
                Some(sqlite_path) => MemoryRuntimeConfig::for_sqlite_path(sqlite_path.clone()),
                None => MemoryRuntimeConfig::default(),
            },
        }
    }
}

#[cfg(feature = "memory-sqlite")]
impl From<&SessionStoreConfig> for MemoryRuntimeConfig {
    fn from(config: &SessionStoreConfig) -> Self {
        config.as_memory_runtime_config()
    }
}

#[cfg(feature = "memory-sqlite")]
impl From<&MemoryRuntimeConfig> for SessionStoreConfig {
    fn from(config: &MemoryRuntimeConfig) -> Self {
        SessionStoreConfig::from_memory_runtime_config(config)
    }
}

#[cfg(feature = "memory-sqlite")]
static SESSION_STORE_CONFIG: OnceLock<SessionStoreConfig> = OnceLock::new();

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
    SESSION_STORE_CONFIG.get_or_init(|| {
        SessionStoreConfig::from_memory_runtime_config(
            crate::memory::runtime_config::get_memory_runtime_config(),
        )
    })
}

#[cfg(feature = "memory-sqlite")]
pub fn ensure_session_store_ready(
    path: Option<PathBuf>,
    config: &SessionStoreConfig,
) -> Result<PathBuf, String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::ensure_memory_db_ready(path, &runtime_config)
}

#[cfg(feature = "memory-sqlite")]
pub fn append_session_turn_direct(
    session_id: &str,
    role: &str,
    content: &str,
    config: &SessionStoreConfig,
) -> Result<(), String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::append_turn_direct(session_id, role, content, &runtime_config)
}

#[cfg(all(test, feature = "memory-sqlite"))]
pub fn replace_session_turns_direct(
    session_id: &str,
    turns: &[SessionWindowTurn],
    config: &SessionStoreConfig,
) -> Result<(), String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::replace_session_turns_direct(session_id, turns, &runtime_config)
}

#[cfg(feature = "memory-sqlite")]
pub fn window_session_turns(
    session_id: &str,
    limit: usize,
    config: &SessionStoreConfig,
) -> Result<Vec<SessionTranscriptTurn>, String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::window_direct(session_id, limit, &runtime_config)
}

#[cfg(feature = "memory-sqlite")]
pub fn transcript_session_turns_paged(
    session_id: &str,
    page_size: usize,
    config: &SessionStoreConfig,
) -> Result<Vec<SessionTranscriptTurn>, String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::transcript_direct_paged(session_id, page_size, &runtime_config)
}

#[cfg(feature = "memory-sqlite")]
pub fn load_session_prompt_context(
    session_id: &str,
    config: &SessionStoreConfig,
) -> Result<Vec<MemoryContextEntry>, String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::load_prompt_context(session_id, &runtime_config)
}

#[cfg(feature = "memory-sqlite")]
pub fn hydrate_session_memory_context_with_workspace_root(
    session_id: &str,
    workspace_root: Option<&std::path::Path>,
    config: &SessionStoreConfig,
) -> Result<HydratedMemoryContext, String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::hydrate_memory_context_with_workspace_root(
        session_id,
        workspace_root,
        &runtime_config,
    )
}

#[cfg(feature = "memory-sqlite")]
pub async fn run_session_compact_stage(
    session_id: &str,
    workspace_root: Option<&std::path::Path>,
    config: &SessionStoreConfig,
) -> Result<StageDiagnostics, String> {
    let runtime_config = config.as_memory_runtime_config();
    crate::memory::run_compact_stage(session_id, workspace_root, &runtime_config).await
}

#[cfg(feature = "memory-sqlite")]
pub fn session_memory_adapter(config: &SessionStoreConfig) -> crate::memory::MvpMemoryAdapter {
    crate::memory::MvpMemoryAdapter::with_config(config.as_memory_runtime_config())
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
    use crate::config::{MemoryProfile, MemorySystemKind};
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::memory::{MemoryContextKind, MemoryRecallMode};
    use crate::session::store::{
        SessionStoreConfig, append_session_turn_direct, ensure_session_store_ready,
        hydrate_session_memory_context_with_workspace_root, load_session_prompt_context,
        window_session_turns,
    };
    use crate::test_support::{ScopedEnv, unique_temp_dir};

    #[test]
    fn session_store_facade_round_trips_transcript_turns() {
        let root = unique_temp_dir("session-store-facade");
        std::fs::create_dir_all(&root).expect("create session store test root");
        let sqlite_path = root.join("memory.sqlite3");
        let config = SessionStoreConfig {
            sqlite_path: Some(sqlite_path.clone()),
            runtime_config: None,
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

    #[test]
    fn session_store_config_from_memory_runtime_config_keeps_runtime_snapshot() {
        let root = unique_temp_dir("session-store-config-from-memory-runtime");
        std::fs::create_dir_all(&root).expect("create session store config test root");
        let sqlite_path = root.join("memory.sqlite3");
        let mut runtime_config = MemoryRuntimeConfig::for_sqlite_path(sqlite_path.clone());
        runtime_config.profile = MemoryProfile::WindowPlusSummary;
        runtime_config.system = MemorySystemKind::RecallFirst;
        runtime_config.resolved_system_id = Some("recall_first".to_owned());
        runtime_config.fail_open = false;
        runtime_config.sliding_window = 9;
        runtime_config.summary_max_chars = 900;

        let session_store_config = SessionStoreConfig::from_memory_runtime_config(&runtime_config);

        assert_eq!(session_store_config.sqlite_path, Some(sqlite_path));
        assert_eq!(session_store_config.runtime_config, Some(runtime_config));
    }

    #[test]
    fn session_store_config_preserves_runtime_memory_semantics_across_boundary() {
        let root = unique_temp_dir("session-store-config-runtime-semantics");
        std::fs::create_dir_all(&root).expect("create session store runtime semantics root");
        let sqlite_path = root.join("memory.sqlite3");
        let override_sqlite_path = root.join("override-memory.sqlite3");

        let mut runtime_config = MemoryRuntimeConfig::for_sqlite_path(sqlite_path);
        runtime_config.profile = MemoryProfile::WindowPlusSummary;
        runtime_config.system = MemorySystemKind::RecallFirst;
        runtime_config.resolved_system_id = Some("recall_first".to_owned());
        runtime_config.fail_open = false;
        runtime_config.sliding_window = 17;
        runtime_config.summary_max_chars = 1234;
        runtime_config.profile_note = Some("preserve profile note".to_owned());

        let mut session_store_config =
            SessionStoreConfig::from_memory_runtime_config(&runtime_config);
        session_store_config.sqlite_path = Some(override_sqlite_path.clone());

        let round_tripped_runtime = session_store_config.as_memory_runtime_config();

        assert_eq!(
            round_tripped_runtime.sqlite_path,
            Some(override_sqlite_path)
        );
        assert_eq!(
            round_tripped_runtime.profile,
            MemoryProfile::WindowPlusSummary
        );
        assert_eq!(round_tripped_runtime.system, MemorySystemKind::RecallFirst);
        assert_eq!(
            round_tripped_runtime.resolved_system_id.as_deref(),
            Some("recall_first")
        );
        assert!(!round_tripped_runtime.fail_open);
        assert_eq!(round_tripped_runtime.sliding_window, 17);
        assert_eq!(round_tripped_runtime.summary_max_chars, 1234);
        assert_eq!(
            round_tripped_runtime.profile_note.as_deref(),
            Some("preserve profile note")
        );
    }

    #[test]
    fn session_store_config_without_env_overrides_ignores_memory_runtime_env() {
        let mut env = ScopedEnv::new();
        env.set("LOONG_MEMORY_PROFILE", "profile_plus_window");
        env.set("LOONG_MEMORY_PROFILE_NOTE", "env note");
        env.set("LOONG_SQLITE_PATH", "/tmp/env-memory.sqlite3");

        let config = crate::config::MemoryConfig {
            profile: MemoryProfile::WindowOnly,
            sqlite_path: "/tmp/config-memory.sqlite3".to_owned(),
            profile_note: Some("config note".to_owned()),
            ..crate::config::MemoryConfig::default()
        };

        let session_store_config =
            SessionStoreConfig::from_memory_config_without_env_overrides(&config);
        let runtime_config = session_store_config.as_memory_runtime_config();

        assert_eq!(runtime_config.profile, MemoryProfile::WindowOnly);
        assert_eq!(
            runtime_config.sqlite_path,
            Some(std::path::PathBuf::from("/tmp/config-memory.sqlite3"))
        );
        assert_eq!(runtime_config.profile_note.as_deref(), Some("config note"));
    }

    #[test]
    fn load_session_prompt_context_reads_session_turns_through_session_store_boundary() {
        let root = unique_temp_dir("session-store-load-prompt-context");
        std::fs::create_dir_all(&root).expect("create session store prompt root");
        let sqlite_path = root.join("memory.sqlite3");
        let config = SessionStoreConfig {
            sqlite_path: Some(sqlite_path.clone()),
            runtime_config: None,
        };

        ensure_session_store_ready(Some(sqlite_path), &config).expect("initialize session store");
        append_session_turn_direct("prompt-context-session", "user", "hello", &config)
            .expect("append turn");

        let entries = load_session_prompt_context("prompt-context-session", &config)
            .expect("load prompt context");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, MemoryContextKind::Turn);
        assert_eq!(entries[0].content, "hello");
    }

    #[test]
    fn hydrate_session_memory_context_with_workspace_root_keeps_workspace_recall() {
        let root = unique_temp_dir("session-store-hydrate-workspace-root");
        std::fs::create_dir_all(&root).expect("create session store hydrate root");
        let sqlite_path = root.join("memory.sqlite3");
        let memory_file_path = root.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember deploy freeze").expect("write memory file");
        let config = SessionStoreConfig {
            sqlite_path: Some(sqlite_path.clone()),
            runtime_config: None,
        };

        ensure_session_store_ready(Some(sqlite_path), &config).expect("initialize session store");
        let hydrated = hydrate_session_memory_context_with_workspace_root(
            "hydrate-session",
            Some(&root),
            &config,
        )
        .expect("hydrate session memory context");

        assert!(hydrated.entries.iter().any(|entry| {
            entry.kind == MemoryContextKind::RetrievedMemory
                && entry.content.contains("remember deploy freeze")
                && entry.provenance.first().is_some_and(|provenance| {
                    provenance.recall_mode == MemoryRecallMode::PromptAssembly
                })
        }));
    }
}
