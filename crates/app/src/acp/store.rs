use std::collections::BTreeMap;
#[cfg(feature = "memory-sqlite")]
use std::path::PathBuf;
use std::sync::RwLock;

#[cfg(feature = "memory-sqlite")]
use rusqlite::OptionalExtension;

use crate::CliResult;

use super::backend::{AcpRoutingOrigin, AcpSessionMetadata, AcpSessionMode, AcpSessionState};
use super::binding::AcpSessionBindingScope;

pub trait AcpSessionStore: Send + Sync {
    fn get(&self, session_key: &str) -> CliResult<Option<AcpSessionMetadata>>;
    fn get_by_conversation_id(
        &self,
        conversation_id: &str,
    ) -> CliResult<Option<AcpSessionMetadata>>;
    fn get_by_binding_route_session_id(
        &self,
        route_session_id: &str,
    ) -> CliResult<Option<AcpSessionMetadata>>;
    fn upsert(&self, metadata: AcpSessionMetadata) -> CliResult<()>;
    fn remove(&self, session_key: &str) -> CliResult<()>;
    fn list(&self) -> CliResult<Vec<AcpSessionMetadata>>;
}

#[derive(Default)]
pub struct InMemoryAcpSessionStore {
    sessions: RwLock<BTreeMap<String, AcpSessionMetadata>>,
}

impl AcpSessionStore for InMemoryAcpSessionStore {
    fn get(&self, session_key: &str) -> CliResult<Option<AcpSessionMetadata>> {
        let guard = self
            .sessions
            .read()
            .map_err(|_error| "ACP session store lock poisoned".to_owned())?;
        Ok(guard.get(session_key).cloned())
    }

    fn get_by_conversation_id(
        &self,
        conversation_id: &str,
    ) -> CliResult<Option<AcpSessionMetadata>> {
        let normalized = conversation_id.trim();
        if normalized.is_empty() {
            return Ok(None);
        }
        let guard = self
            .sessions
            .read()
            .map_err(|_error| "ACP session store lock poisoned".to_owned())?;
        Ok(guard
            .values()
            .find(|metadata| metadata.conversation_id.as_deref() == Some(normalized))
            .cloned())
    }

    fn get_by_binding_route_session_id(
        &self,
        route_session_id: &str,
    ) -> CliResult<Option<AcpSessionMetadata>> {
        let normalized = route_session_id.trim();
        if normalized.is_empty() {
            return Ok(None);
        }
        let guard = self
            .sessions
            .read()
            .map_err(|_error| "ACP session store lock poisoned".to_owned())?;
        Ok(guard
            .values()
            .find(|metadata| {
                metadata
                    .binding
                    .as_ref()
                    .map(|binding| binding.route_session_id.as_str())
                    == Some(normalized)
            })
            .cloned())
    }

    fn upsert(&self, metadata: AcpSessionMetadata) -> CliResult<()> {
        let mut guard = self
            .sessions
            .write()
            .map_err(|_error| "ACP session store lock poisoned".to_owned())?;
        guard.insert(metadata.session_key.clone(), metadata);
        Ok(())
    }

    fn remove(&self, session_key: &str) -> CliResult<()> {
        let mut guard = self
            .sessions
            .write()
            .map_err(|_error| "ACP session store lock poisoned".to_owned())?;
        guard.remove(session_key);
        Ok(())
    }

    fn list(&self) -> CliResult<Vec<AcpSessionMetadata>> {
        let guard = self
            .sessions
            .read()
            .map_err(|_error| "ACP session store lock poisoned".to_owned())?;
        Ok(guard.values().cloned().collect())
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Default)]
pub struct AcpSqliteSessionStore {
    path: Option<PathBuf>,
}

#[cfg(feature = "memory-sqlite")]
impl AcpSqliteSessionStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    fn resolved_path(&self) -> CliResult<PathBuf> {
        let runtime_config = self
            .path
            .clone()
            .map(crate::memory::runtime_config::MemoryRuntimeConfig::for_sqlite_path)
            .unwrap_or_default();
        crate::memory::ensure_memory_db_ready(self.path.clone(), &runtime_config)
            .map_err(|error| format!("resolve ACP sqlite store path failed: {error}"))
    }

    fn connect(&self) -> CliResult<rusqlite::Connection> {
        let path = self.resolved_path()?;
        ensure_sqlite_schema(&path)?;
        rusqlite::Connection::open(&path)
            .map_err(|error| format!("open ACP sqlite session store failed: {error}"))
    }
}

#[cfg(feature = "memory-sqlite")]
impl AcpSessionStore for AcpSqliteSessionStore {
    fn get(&self, session_key: &str) -> CliResult<Option<AcpSessionMetadata>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT session_key, conversation_id, binding_route_session_id, binding_channel_id,
                    binding_account_id, binding_conversation_id, binding_participant_id, binding_thread_id, backend_id,
                    runtime_session_name, working_directory, backend_session_id, agent_session_id,
                    activation_origin, mode, state, last_activity_ms, last_error
             FROM acp_sessions
             WHERE session_key = ?1",
            rusqlite::params![session_key],
            decode_session_row,
        )
        .optional()
        .map_err(|error| format!("load ACP session metadata failed: {error}"))
    }

    fn get_by_conversation_id(
        &self,
        conversation_id: &str,
    ) -> CliResult<Option<AcpSessionMetadata>> {
        let normalized = conversation_id.trim();
        if normalized.is_empty() {
            return Ok(None);
        }
        let conn = self.connect()?;
        conn.query_row(
            "SELECT session_key, conversation_id, binding_route_session_id, binding_channel_id,
                    binding_account_id, binding_conversation_id, binding_participant_id, binding_thread_id, backend_id,
                    runtime_session_name, working_directory, backend_session_id, agent_session_id,
                    activation_origin, mode, state, last_activity_ms, last_error
             FROM acp_sessions
             WHERE conversation_id = ?1
             ORDER BY last_activity_ms DESC, session_key ASC
             LIMIT 1",
            rusqlite::params![normalized],
            decode_session_row,
        )
        .optional()
        .map_err(|error| format!("load ACP session by conversation id failed: {error}"))
    }

    fn get_by_binding_route_session_id(
        &self,
        route_session_id: &str,
    ) -> CliResult<Option<AcpSessionMetadata>> {
        let normalized = route_session_id.trim();
        if normalized.is_empty() {
            return Ok(None);
        }
        let conn = self.connect()?;
        conn.query_row(
            "SELECT session_key, conversation_id, binding_route_session_id, binding_channel_id,
                    binding_account_id, binding_conversation_id, binding_participant_id, binding_thread_id, backend_id,
                    runtime_session_name, working_directory, backend_session_id, agent_session_id,
                    activation_origin, mode, state, last_activity_ms, last_error
             FROM acp_sessions
             WHERE binding_route_session_id = ?1
             ORDER BY last_activity_ms DESC, session_key ASC
             LIMIT 1",
            rusqlite::params![normalized],
            decode_session_row,
        )
        .optional()
        .map_err(|error| format!("load ACP session by binding route failed: {error}"))
    }

    fn upsert(&self, metadata: AcpSessionMetadata) -> CliResult<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO acp_sessions(
                session_key, conversation_id, binding_route_session_id, binding_channel_id,
                binding_account_id, binding_conversation_id, binding_participant_id, binding_thread_id, backend_id,
                runtime_session_name, working_directory, backend_session_id, agent_session_id,
                activation_origin, mode, state, last_activity_ms, last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(session_key) DO UPDATE SET
                conversation_id = excluded.conversation_id,
                binding_route_session_id = excluded.binding_route_session_id,
                binding_channel_id = excluded.binding_channel_id,
                binding_account_id = excluded.binding_account_id,
                binding_conversation_id = excluded.binding_conversation_id,
                binding_participant_id = excluded.binding_participant_id,
                binding_thread_id = excluded.binding_thread_id,
                backend_id = excluded.backend_id,
                runtime_session_name = excluded.runtime_session_name,
                working_directory = excluded.working_directory,
                backend_session_id = excluded.backend_session_id,
                agent_session_id = excluded.agent_session_id,
                activation_origin = excluded.activation_origin,
                mode = excluded.mode,
                state = excluded.state,
                last_activity_ms = excluded.last_activity_ms,
                last_error = excluded.last_error",
            rusqlite::params![
                metadata.session_key,
                metadata.conversation_id,
                metadata
                    .binding
                    .as_ref()
                    .map(|binding| binding.route_session_id.clone()),
                metadata
                    .binding
                    .as_ref()
                    .and_then(|binding| binding.channel_id.clone()),
                metadata
                    .binding
                    .as_ref()
                    .and_then(|binding| binding.account_id.clone()),
                metadata
                    .binding
                    .as_ref()
                    .and_then(|binding| binding.conversation_id.clone()),
                metadata
                    .binding
                    .as_ref()
                    .and_then(|binding| binding.participant_id.clone()),
                metadata
                    .binding
                    .as_ref()
                    .and_then(|binding| binding.thread_id.clone()),
                metadata.backend_id,
                metadata.runtime_session_name,
                metadata
                    .working_directory
                    .as_ref()
                    .map(|path| path.display().to_string()),
                metadata.backend_session_id,
                metadata.agent_session_id,
                metadata.activation_origin.map(AcpRoutingOrigin::as_str),
                metadata.mode.map(encode_mode),
                encode_state(metadata.state),
                i64::try_from(metadata.last_activity_ms).map_err(|error| {
                    format!("ACP last_activity_ms exceeds sqlite INTEGER range: {error}")
                })?,
                metadata.last_error,
            ],
        )
        .map_err(|error| format!("upsert ACP session metadata failed: {error}"))?;
        Ok(())
    }

    fn remove(&self, session_key: &str) -> CliResult<()> {
        let conn = self.connect()?;
        conn.execute(
            "DELETE FROM acp_sessions WHERE session_key = ?1",
            rusqlite::params![session_key],
        )
        .map_err(|error| format!("delete ACP session metadata failed: {error}"))?;
        Ok(())
    }

    fn list(&self) -> CliResult<Vec<AcpSessionMetadata>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT session_key, conversation_id, binding_route_session_id, binding_channel_id,
                        binding_account_id, binding_conversation_id, binding_participant_id, binding_thread_id, backend_id,
                        runtime_session_name, working_directory, backend_session_id, agent_session_id,
                        activation_origin, mode, state, last_activity_ms, last_error
                 FROM acp_sessions
                 ORDER BY session_key ASC",
            )
            .map_err(|error| format!("prepare ACP session list query failed: {error}"))?;
        let rows = stmt
            .query_map([], decode_session_row)
            .map_err(|error| format!("query ACP session list failed: {error}"))?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|error| format!("decode ACP session row failed: {error}"))?);
        }
        Ok(sessions)
    }
}

#[cfg(feature = "memory-sqlite")]
fn ensure_sqlite_schema(path: &PathBuf) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create ACP sqlite parent directory failed: {error}"))?;
    }

    let conn = rusqlite::Connection::open(path)
        .map_err(|error| format!("open ACP sqlite session store failed: {error}"))?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS acp_sessions(
          session_key TEXT PRIMARY KEY,
          conversation_id TEXT,
          binding_route_session_id TEXT,
          binding_channel_id TEXT,
          binding_account_id TEXT,
          binding_conversation_id TEXT,
          binding_participant_id TEXT,
          binding_thread_id TEXT,
          backend_id TEXT NOT NULL,
          runtime_session_name TEXT NOT NULL,
          working_directory TEXT,
          backend_session_id TEXT,
          agent_session_id TEXT,
          activation_origin TEXT,
          mode TEXT,
          state TEXT NOT NULL,
          last_activity_ms INTEGER NOT NULL DEFAULT 0,
          last_error TEXT
        );
        ",
    )
    .map_err(|error| format!("initialize ACP sqlite schema failed: {error}"))?;
    ensure_sqlite_column(&conn, "acp_sessions", "conversation_id", "TEXT")?;
    ensure_sqlite_column(&conn, "acp_sessions", "binding_route_session_id", "TEXT")?;
    ensure_sqlite_column(&conn, "acp_sessions", "binding_channel_id", "TEXT")?;
    ensure_sqlite_column(&conn, "acp_sessions", "binding_account_id", "TEXT")?;
    ensure_sqlite_column(&conn, "acp_sessions", "binding_conversation_id", "TEXT")?;
    ensure_sqlite_column(&conn, "acp_sessions", "binding_participant_id", "TEXT")?;
    ensure_sqlite_column(&conn, "acp_sessions", "binding_thread_id", "TEXT")?;
    ensure_sqlite_column(&conn, "acp_sessions", "activation_origin", "TEXT")?;
    ensure_sqlite_column(
        &conn,
        "acp_sessions",
        "last_activity_ms",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_sqlite_column(&conn, "acp_sessions", "last_error", "TEXT")?;
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_acp_sessions_conversation_id
          ON acp_sessions(conversation_id);
        CREATE INDEX IF NOT EXISTS idx_acp_sessions_binding_route_session_id
          ON acp_sessions(binding_route_session_id);
        ",
    )
    .map_err(|error| format!("initialize ACP sqlite conversation index failed: {error}"))?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn ensure_sqlite_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> CliResult<()> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|error| format!("prepare sqlite schema introspection failed: {error}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("query sqlite schema introspection failed: {error}"))?;
    let mut present = false;
    for row in rows {
        if row.map_err(|error| format!("decode sqlite schema introspection failed: {error}"))?
            == column
        {
            present = true;
            break;
        }
    }

    if present {
        return Ok(());
    }

    let alter = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
    conn.execute(&alter, [])
        .map_err(|error| format!("extend ACP sqlite schema failed: {error}"))?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn decode_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AcpSessionMetadata> {
    let activation_origin_raw: Option<String> = row.get(13)?;
    let mode_raw: Option<String> = row.get(14)?;
    let state_raw: String = row.get(15)?;
    let last_activity_raw: i64 = row.get(16)?;
    let binding_route_session_id: Option<String> = row.get(2)?;
    let binding_channel_id: Option<String> = row.get(3)?;
    let binding_account_id: Option<String> = row.get(4)?;
    let binding_conversation_id: Option<String> = row.get(5)?;
    let binding_participant_id: Option<String> = row.get(6)?;
    let binding_thread_id: Option<String> = row.get(7)?;
    Ok(AcpSessionMetadata {
        session_key: row.get(0)?,
        conversation_id: row.get(1)?,
        binding: binding_route_session_id.map(|route_session_id| AcpSessionBindingScope {
            route_session_id,
            channel_id: binding_channel_id,
            account_id: binding_account_id,
            conversation_id: binding_conversation_id,
            participant_id: binding_participant_id,
            thread_id: binding_thread_id,
        }),
        backend_id: row.get(8)?,
        runtime_session_name: row.get(9)?,
        working_directory: row.get::<_, Option<String>>(10)?.map(PathBuf::from),
        backend_session_id: row.get(11)?,
        agent_session_id: row.get(12)?,
        activation_origin: activation_origin_raw
            .as_deref()
            .map(|raw| {
                AcpRoutingOrigin::parse(raw).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        13,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("unknown ACP routing origin `{raw}`"),
                        )),
                    )
                })
            })
            .transpose()?,
        mode: mode_raw
            .as_deref()
            .map(decode_mode)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    14,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                )
            })?,
        state: decode_state(&state_raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                15,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
            )
        })?,
        last_activity_ms: u64::try_from(last_activity_raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                16,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        last_error: row.get(17)?,
    })
}

#[cfg(feature = "memory-sqlite")]
fn encode_mode(mode: AcpSessionMode) -> &'static str {
    match mode {
        AcpSessionMode::Interactive => "interactive",
        AcpSessionMode::Background => "background",
        AcpSessionMode::Review => "review",
    }
}

#[cfg(feature = "memory-sqlite")]
fn decode_mode(raw: &str) -> Result<AcpSessionMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "interactive" => Ok(AcpSessionMode::Interactive),
        "background" => Ok(AcpSessionMode::Background),
        "review" => Ok(AcpSessionMode::Review),
        other => Err(format!("unknown ACP session mode `{other}`")),
    }
}

#[cfg(feature = "memory-sqlite")]
fn encode_state(state: AcpSessionState) -> &'static str {
    match state {
        AcpSessionState::Initializing => "initializing",
        AcpSessionState::Ready => "ready",
        AcpSessionState::Busy => "busy",
        AcpSessionState::Cancelling => "cancelling",
        AcpSessionState::Error => "error",
        AcpSessionState::Closed => "closed",
    }
}

#[cfg(feature = "memory-sqlite")]
fn decode_state(raw: &str) -> Result<AcpSessionState, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "initializing" => Ok(AcpSessionState::Initializing),
        "ready" => Ok(AcpSessionState::Ready),
        "busy" => Ok(AcpSessionState::Busy),
        "cancelling" => Ok(AcpSessionState::Cancelling),
        "error" => Ok(AcpSessionState::Error),
        "closed" => Ok(AcpSessionState::Closed),
        other => Err(format!("unknown ACP session state `{other}`")),
    }
}
