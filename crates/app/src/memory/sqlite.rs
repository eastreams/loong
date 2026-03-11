use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    MEMORY_OP_APPEND_TURN, MEMORY_OP_CLEAR_SESSION, MEMORY_OP_WINDOW, build_append_turn_request,
    runtime_config::MemoryRuntimeConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub role: String,
    pub content: String,
    pub ts: i64,
}

pub(super) fn append_turn(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.append_turn payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.append_turn requires payload.session_id".to_owned())?;
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.append_turn requires payload.role".to_owned())?;
    let content = payload
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "memory.append_turn requires payload.content".to_owned())?;

    let path = resolve_db_path(config);
    ensure_sqlite_schema(&path)?;
    let conn = rusqlite::Connection::open(&path)
        .map_err(|error| format!("open sqlite memory db failed: {error}"))?;
    let ts = unix_ts_now();
    conn.execute(
        "INSERT INTO turns(session_id, role, content, ts) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![session_id, role, content, ts],
    )
    .map_err(|error| format!("insert memory turn failed: {error}"))?;

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_APPEND_TURN,
            "session_id": session_id,
            "role": role,
            "ts": ts,
            "db_path": path.display().to_string(),
        }),
    })
}

pub(super) fn load_window(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.window payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.window requires payload.session_id".to_owned())?;
    let allow_extended_limit = payload
        .get("allow_extended_limit")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let hard_limit_cap = if allow_extended_limit {
        512_u64
    } else {
        128_u64
    };
    let requested_limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| default_window_size_u64(config))
        .clamp(1, hard_limit_cap) as usize;
    let default_window = default_window_size(config).max(1);
    let window_limit = if allow_extended_limit {
        requested_limit
    } else {
        requested_limit.min(default_window)
    };
    let path = resolve_db_path(config);
    let turns = query_turns(session_id, Some(window_limit), config)?;

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_WINDOW,
            "session_id": session_id,
            "limit": window_limit,
            "allow_extended_limit": allow_extended_limit,
            "turns": turns,
            "db_path": path.display().to_string(),
        }),
    })
}

pub(super) fn clear_session(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.clear_session payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.clear_session requires payload.session_id".to_owned())?;

    let path = resolve_db_path(config);
    ensure_sqlite_schema(&path)?;
    let conn = rusqlite::Connection::open(&path)
        .map_err(|error| format!("open sqlite memory db failed: {error}"))?;
    let affected = conn
        .execute(
            "DELETE FROM turns WHERE session_id = ?1",
            rusqlite::params![session_id],
        )
        .map_err(|error| format!("clear memory session failed: {error}"))?;
    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_CLEAR_SESSION,
            "session_id": session_id,
            "deleted_rows": affected,
        }),
    })
}

pub(super) fn append_turn_direct(
    session_id: &str,
    role: &str,
    content: &str,
    config: &MemoryRuntimeConfig,
) -> Result<(), String> {
    let request = build_append_turn_request(session_id, role, content);
    super::execute_memory_core_with_config(request, config)?;
    Ok(())
}

pub(super) fn window_direct(
    session_id: &str,
    limit: usize,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    window_direct_with_options(session_id, limit, true, config)
}

pub(super) fn window_direct_with_options(
    session_id: &str,
    limit: usize,
    allow_extended_limit: bool,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    let request = MemoryCoreRequest {
        operation: "window".to_owned(),
        payload: json!({
            "session_id": session_id,
            "limit": limit,
            "allow_extended_limit": allow_extended_limit,
        }),
    };
    let outcome = super::execute_memory_core_with_config(request, config)?;
    let turns_raw = outcome.payload.get("turns").cloned().unwrap_or(Value::Null);
    serde_json::from_value(turns_raw)
        .map_err(|error| format!("decode memory turns failed: {error}"))
}

pub(super) fn session_turns_direct(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    query_turns(session_id, None, config)
}

pub(super) fn ensure_memory_db_ready(
    path: Option<PathBuf>,
    config: &MemoryRuntimeConfig,
) -> Result<PathBuf, String> {
    let effective = path.unwrap_or_else(|| resolve_db_path(config));
    ensure_sqlite_schema(&effective)?;
    Ok(effective)
}

fn default_window_size(config: &MemoryRuntimeConfig) -> usize {
    config.sliding_window.max(1)
}

fn default_window_size_u64(config: &MemoryRuntimeConfig) -> u64 {
    default_window_size(config) as u64
}

fn unix_ts_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn resolve_db_path(config: &MemoryRuntimeConfig) -> PathBuf {
    if let Some(path) = &config.sqlite_path {
        return path.clone();
    }
    crate::config::default_loongclaw_home().join("memory.sqlite3")
}

fn query_turns(
    session_id: &str,
    limit: Option<usize>,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    let path = resolve_db_path(config);
    ensure_sqlite_schema(&path)?;
    let conn = rusqlite::Connection::open(&path)
        .map_err(|error| format!("open sqlite memory db failed: {error}"))?;

    let mut turns = if let Some(limit) = limit {
        let mut stmt = conn
            .prepare(
                "SELECT role, content, ts
                 FROM turns
                 WHERE session_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|error| format!("prepare memory window query failed: {error}"))?;
        let rows = stmt
            .query_map(
                rusqlite::params![session_id, limit as i64],
                |row| -> rusqlite::Result<ConversationTurn> {
                    Ok(ConversationTurn {
                        role: row.get(0)?,
                        content: row.get(1)?,
                        ts: row.get(2)?,
                    })
                },
            )
            .map_err(|error| format!("query memory window failed: {error}"))?;

        let mut turns = Vec::new();
        for item in rows {
            turns.push(item.map_err(|error| format!("decode memory window row failed: {error}"))?);
        }
        turns.reverse();
        turns
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT role, content, ts
                 FROM turns
                 WHERE session_id = ?1
                 ORDER BY id ASC",
            )
            .map_err(|error| format!("prepare full memory query failed: {error}"))?;
        let rows = stmt
            .query_map(
                rusqlite::params![session_id],
                |row| -> rusqlite::Result<ConversationTurn> {
                    Ok(ConversationTurn {
                        role: row.get(0)?,
                        content: row.get(1)?,
                        ts: row.get(2)?,
                    })
                },
            )
            .map_err(|error| format!("query full memory session failed: {error}"))?;

        let mut turns = Vec::new();
        for item in rows {
            turns.push(item.map_err(|error| format!("decode full memory row failed: {error}"))?);
        }
        turns
    };

    if turns.is_empty() {
        return Ok(Vec::new());
    }
    Ok(std::mem::take(&mut turns))
}

fn ensure_sqlite_schema(path: &PathBuf) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create sqlite parent directory failed: {error}"))?;
    }

    let conn = rusqlite::Connection::open(path)
        .map_err(|error| format!("open sqlite memory db failed: {error}"))?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS turns(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_turns_session_id ON turns(session_id, id);
        ",
    )
    .map_err(|error| format!("initialize sqlite memory schema failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_window_size_prefers_injected_config() {
        let config = MemoryRuntimeConfig {
            sqlite_path: None,
            sliding_window: Some(24),
        };

        assert_eq!(default_window_size(&config), 24);
    }

    #[test]
    fn default_window_size_falls_back_to_default_without_config() {
        assert_eq!(default_window_size(&MemoryRuntimeConfig::default()), 12);
    }
}
