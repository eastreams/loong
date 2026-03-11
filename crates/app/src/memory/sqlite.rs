use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    build_append_turn_request, build_window_request, runtime_config::MemoryRuntimeConfig,
    MEMORY_OP_APPEND_TURN, MEMORY_OP_CLEAR_SESSION, MEMORY_OP_WINDOW,
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
    let requested_limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or_else(default_window_size_u64)
        .clamp(1, 128) as usize;
    let default_window = default_window_size().max(1);
    let window_limit = requested_limit.min(default_window);

    let path = resolve_db_path(config);
    ensure_sqlite_schema(&path)?;
    let conn = rusqlite::Connection::open(&path)
        .map_err(|error| format!("open sqlite memory db failed: {error}"))?;

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
            rusqlite::params![session_id, window_limit as i64],
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

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_WINDOW,
            "session_id": session_id,
            "limit": window_limit,
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
    let request = build_window_request(session_id, limit);
    let outcome = super::execute_memory_core_with_config(request, config)?;
    let turns_raw = outcome.payload.get("turns").cloned().unwrap_or(Value::Null);
    serde_json::from_value(turns_raw)
        .map_err(|error| format!("decode memory turns failed: {error}"))
}

pub(super) fn ensure_memory_db_ready(
    path: Option<PathBuf>,
    config: &MemoryRuntimeConfig,
) -> Result<PathBuf, String> {
    let effective = path.unwrap_or_else(|| resolve_db_path(config));
    ensure_sqlite_schema(&effective)?;
    Ok(effective)
}

fn default_window_size() -> usize {
    std::env::var("LOONGCLAW_SLIDING_WINDOW")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(12)
}

fn default_window_size_u64() -> u64 {
    default_window_size() as u64
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

fn ensure_sqlite_schema(path: &PathBuf) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create sqlite parent directory failed: {error}"))?;
        }
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
