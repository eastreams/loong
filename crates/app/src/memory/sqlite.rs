#[cfg(test)]
use std::thread::ThreadId;
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH},
};

use loong_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    CanonicalMemoryKind, CanonicalMemoryRecord, DerivedMemoryKind, MEMORY_OP_APPEND_TURN,
    MEMORY_OP_CLEAR_SESSION, MEMORY_OP_REPLACE_TURNS, MEMORY_OP_TRANSCRIPT, MEMORY_OP_WINDOW,
    MemoryAuthority, MemoryRecallMode, MemoryRecordStatus, MemoryScope, MemoryTrustLevel,
    ParsedWorkspaceMemoryDocument, WindowTurn, WorkspaceMemoryDocumentKind,
    WorkspaceMemoryDocumentLocation, canonical_memory_record_from_persisted_turn,
    collect_workspace_memory_document_locations, parse_workspace_memory_document,
    runtime_config::MemoryRuntimeConfig,
};
use crate::search_text::build_search_index_text;
use crate::task_progress::{
    TASK_PROGRESS_EVENT_KIND, TaskProgressRecord, task_progress_from_event_payload,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub role: String,
    pub content: String,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionMetadataHint {
    pub kind: String,
    pub state: String,
    pub parent_session_id: Option<String>,
    pub label: Option<String>,
    pub parent_label: Option<String>,
    pub lineage_root_session_id: Option<String>,
    pub lineage_root_label: Option<String>,
    pub lineage_depth: usize,
    pub workflow_task: Option<String>,
    pub workflow_phase: Option<String>,
    pub workflow_operation_kind: Option<String>,
    pub workflow_operation_scope: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PromptWindowTurn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SqliteBootstrapDiagnostics {
    pub cache_hit: bool,
    pub total_ms: f64,
    pub normalize_path_ms: f64,
    pub registry_lock_ms: f64,
    pub registry_lookup_ms: f64,
    pub runtime_create_ms: f64,
    pub parent_dir_create_ms: f64,
    pub connection_open_ms: f64,
    pub configure_connection_ms: f64,
    pub schema_init_ms: f64,
    pub schema_upgrade_ms: f64,
    pub registry_insert_ms: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SqliteContextLoadDiagnostics {
    pub total_ms: f64,
    pub window_query_ms: f64,
    pub window_turn_count_query_ms: f64,
    pub window_exact_rows_query_ms: f64,
    pub window_known_overflow_rows_query_ms: f64,
    pub window_fallback_rows_query_ms: f64,
    pub summary_checkpoint_meta_query_ms: f64,
    pub summary_checkpoint_body_load_ms: f64,
    pub summary_checkpoint_metadata_update_ms: f64,
    pub summary_checkpoint_metadata_update_returning_body_ms: f64,
    pub summary_rebuild_ms: f64,
    pub summary_rebuild_stream_ms: f64,
    pub summary_rebuild_checkpoint_upsert_ms: f64,
    pub summary_rebuild_checkpoint_metadata_upsert_ms: f64,
    pub summary_rebuild_checkpoint_body_upsert_ms: f64,
    pub summary_rebuild_checkpoint_commit_ms: f64,
    pub summary_catch_up_ms: f64,
}

const SESSION_TERMINAL_OUTCOMES_TABLE_SQL: &str = "
CREATE TABLE IF NOT EXISTS session_terminal_outcomes(
  session_id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  frozen_result_json TEXT NULL,
  recorded_at INTEGER NOT NULL
);
";

#[derive(Debug, Clone, Default)]
struct SqliteConnectionBootstrapDiagnostics {
    parent_dir_create_ms: f64,
    connection_open_ms: f64,
    configure_connection_ms: f64,
    schema_init_ms: f64,
    schema_upgrade_ms: f64,
}

#[derive(Debug, Clone, Default)]
struct SqliteSummaryCheckpointUpsertDiagnostics {
    metadata_upsert_ms: f64,
    body_upsert_ms: f64,
    commit_ms: f64,
}

#[derive(Debug, Clone, Default)]
struct PromptWindowQueryDiagnostics {
    turn_count_query_ms: f64,
    exact_rows_query_ms: f64,
    known_overflow_rows_query_ms: f64,
    fallback_rows_query_ms: f64,
}

impl PromptWindowQueryDiagnostics {
    fn write_into(self, diagnostics: &mut SqliteContextLoadDiagnostics) {
        diagnostics.window_turn_count_query_ms = self.turn_count_query_ms;
        diagnostics.window_exact_rows_query_ms = self.exact_rows_query_ms;
        diagnostics.window_known_overflow_rows_query_ms = self.known_overflow_rows_query_ms;
        diagnostics.window_fallback_rows_query_ms = self.fallback_rows_query_ms;
    }
}

const SUMMARY_FORMAT_VERSION: i64 = 1;
const SQLITE_MEMORY_SCHEMA_VERSION: i64 = 14;
const CANONICAL_REBUILD_BATCH_SIZE: i64 = 256;
const SQLITE_CURRENT_SCHEMA_OBJECT_COUNT: i64 = 29;
const SQLITE_BUSY_TIMEOUT_MS: u64 = 5_000;
const SQLITE_PREPARED_STATEMENT_CACHE_CAPACITY: usize = 16;
const SESSION_TOOL_CONSENT_MODE_CHECK_SQL: &str = "CHECK (mode IN ('prompt', 'auto', 'full'))";
const SESSION_TERMINAL_OUTCOMES_DDL: &str = "
CREATE TABLE IF NOT EXISTS session_terminal_outcomes(
  session_id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  frozen_result_json TEXT NULL,
  recorded_at INTEGER NOT NULL
);
";
const SQL_INSERT_TURN: &str = "INSERT INTO turns(session_id, session_turn_index, role, content, ts) VALUES (?1, ?2, ?3, ?4, ?5)";
const SQL_DELETE_TURNS_FOR_SESSION: &str = "DELETE FROM turns WHERE session_id = ?1";
const SQL_UPSERT_SESSION_TURN_COUNT: &str =
    "INSERT INTO memory_session_state(session_id, turn_count)
             VALUES (?1, 1)
             ON CONFLICT(session_id) DO UPDATE SET
                 turn_count = memory_session_state.turn_count + 1
             RETURNING turn_count";
const SQL_DELETE_SESSION_STATE: &str = "DELETE FROM memory_session_state WHERE session_id = ?1";
const SQL_DELETE_CANONICAL_RECORDS_FOR_SESSION: &str =
    "DELETE FROM memory_canonical_records WHERE session_id = ?1";
const SQL_SET_SESSION_TURN_COUNT: &str = "INSERT INTO memory_session_state(session_id, turn_count)
             VALUES (?1, ?2)
             ON CONFLICT(session_id) DO UPDATE SET
             turn_count = excluded.turn_count";
const SQL_INSERT_CANONICAL_RECORD: &str = "INSERT INTO memory_canonical_records(
             session_id,
             session_turn_index,
             scope,
             kind,
             role,
             content,
             metadata_json,
             search_text,
             ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";
const SQL_SELECT_TURNS_FOR_CANONICAL_REBUILD: &str =
    "SELECT id, session_id, session_turn_index, role, content, ts
             FROM turns
             WHERE id > ?1
             ORDER BY id ASC
             LIMIT ?2";
const SQL_COUNT_TURNS: &str = "SELECT COUNT(*) FROM turns";
const SQL_COUNT_CANONICAL_RECORDS: &str = "SELECT COUNT(*) FROM memory_canonical_records";
const SQL_COUNT_CANONICAL_FTS_ROWS: &str = "SELECT COUNT(*) FROM memory_canonical_records_fts";
const SQL_SEARCH_CANONICAL_RECORDS: &str = "SELECT record.session_id,
             record.session_turn_index,
             record.scope,
             record.kind,
             record.role,
             record.content,
             record.metadata_json,
             record.ts
             FROM memory_canonical_records_fts AS fts
             JOIN memory_canonical_records AS record
               ON record.record_id = fts.rowid
             LEFT JOIN sessions AS session
               ON session.session_id = record.session_id
             LEFT JOIN (
                SELECT session_id, MAX(ts) AS archived_at
                FROM session_events
                WHERE event_kind = 'session_archived'
                GROUP BY session_id
             ) AS archived
               ON archived.session_id = record.session_id
             WHERE memory_canonical_records_fts MATCH ?1
               AND (?2 IS NULL OR record.session_id <> ?2)
               AND record.kind <> 'user_turn'
               AND record.session_id NOT LIKE 'delegate:%'
               AND (
                    session.session_id IS NULL
                    OR (session.kind = 'root' AND archived.archived_at IS NULL)
               )
             ORDER BY bm25(memory_canonical_records_fts), record.ts DESC, record.record_id DESC
             LIMIT ?3";
const SQL_QUERY_RECENT_TURNS_NO_ID: &str = "SELECT role, content, ts, session_turn_index
             FROM turns
             WHERE session_id = ?1
             ORDER BY id DESC
             LIMIT ?2";
#[cfg(test)]
const SQL_QUERY_RECENT_TURNS_WITH_BOUNDARY_ID: &str =
    "SELECT role, content, ts, id, session_turn_index
             FROM turns
             WHERE session_id = ?1
             ORDER BY id DESC
             LIMIT ?2";
const SQL_SELECT_SESSION_TURN_COUNT: &str = "SELECT turn_count
             FROM memory_session_state
             WHERE session_id = ?1";
const SQL_COUNT_CURRENT_SCHEMA_OBJECTS: &str = "SELECT COUNT(*)
             FROM sqlite_master
             WHERE (type = 'table' AND name IN (
                        'turns',
                        'memory_session_state',
                        'memory_summary_checkpoints',
                        'memory_summary_checkpoint_bodies',
                        'memory_canonical_records',
                        'memory_canonical_records_fts',
                        'workspace_memory_documents',
                        'workspace_memory_documents_fts',
                        'session_nodes',
                        'session_heads',
                        'session_artifacts',
                        'session_events_fts',
                        'approval_requests',
                        'approval_grants',
                        'session_tool_consent',
                        'session_tool_policies'
                    ))
                OR (type = 'index' AND name IN (
                        'idx_turns_session_id',
                        'idx_turns_session_turn_index',
                        'idx_session_nodes_session_parent_created',
                        'idx_session_nodes_session_turn_index',
                        'idx_memory_canonical_records_scope_kind_ts',
                        'idx_memory_canonical_records_session_turn',
                        'idx_approval_requests_session_status_requested_at'
                    ))
                OR (type = 'trigger' AND name IN (
                        'memory_canonical_records_ai',
                        'memory_canonical_records_ad',
                        'memory_canonical_records_au',
                        'session_events_ai',
                        'session_events_ad',
                        'session_events_au'
                    ))";
const SQL_QUERY_RECENT_PROMPT_TURNS_WITH_CHECKPOINT_META: &str = "SELECT turns.id,
             turns.role,
             turns.content,
             checkpoint.summarized_through_turn_id,
             checkpoint.summary_before_turn_id,
             checkpoint.summary_body_bytes,
             checkpoint.summary_budget_chars,
             checkpoint.summary_window_size,
             checkpoint.summary_format_version
             FROM turns
             LEFT JOIN memory_summary_checkpoints checkpoint
               ON checkpoint.session_id = ?1
             WHERE turns.session_id = ?1
             ORDER BY turns.id DESC
             LIMIT ?2";
const SQL_QUERY_RECENT_PROMPT_TURNS_WITH_OVERFLOW_PROBE_FALLBACK: &str = "SELECT id, role, content
             FROM turns
             WHERE session_id = ?1
             ORDER BY id DESC
             LIMIT ?2";
const SQL_QUERY_TURNS_AFTER_ID_WITH_LIMIT: &str = "SELECT id, role, content, ts
             FROM turns
             WHERE session_id = ?1
               AND id > ?2
               AND id <= ?3
             ORDER BY id ASC
             LIMIT ?4";
const SQL_QUERY_MAX_TURN_ID_FOR_SESSION: &str = "SELECT COALESCE(MAX(id), 0)
             FROM turns
             WHERE session_id = ?1";
const SQL_QUERY_TURNS_UP_TO_ID: &str = "SELECT id, role, content
             FROM turns
             WHERE session_id = ?1 AND id <= ?2
             ORDER BY id ASC";
const SQL_QUERY_TURNS_BETWEEN_IDS: &str = "SELECT id, role, content
             FROM turns
             WHERE session_id = ?1
               AND id > ?2
               AND id < ?3
             ORDER BY id ASC";
const SQL_QUERY_INITIAL_SUMMARY_ROWS_BY_SESSION_TURN_INDEX: &str = "SELECT id, role, content
             FROM turns
             WHERE session_id = ?1
               AND session_turn_index <= 2
             ORDER BY session_turn_index ASC
             LIMIT 2";
const SQL_QUERY_INITIAL_SUMMARY_ROWS_AFTER_SEED_SESSION_INDEX: &str = "SELECT id, role, content
             FROM turns
             WHERE session_id = ?1
               AND session_turn_index > 1
             ORDER BY session_turn_index ASC";
const SQL_QUERY_SUMMARY_BOUNDARY_TURN_ID_BY_SESSION_TURN_COUNT: &str = "WITH state AS (
             SELECT turn_count
             FROM memory_session_state
             WHERE session_id = ?1
         )
         SELECT turns.id
         FROM state
         JOIN turns
           ON turns.session_id = ?1
          AND turns.session_turn_index = state.turn_count - ?2 + 1
         WHERE state.turn_count >= ?2";
const SQL_QUERY_SUMMARY_FRONTIER_UP_TO_ID: &str = "SELECT id
             FROM turns
             WHERE session_id = ?1
               AND id <= ?2
             ORDER BY id DESC
             LIMIT 1";
const SQL_QUERY_SUMMARY_FRONTIER_BETWEEN_IDS: &str = "SELECT id
             FROM turns
             WHERE session_id = ?1
               AND id > ?2
               AND id < ?3
             ORDER BY id DESC
             LIMIT 1";
const SQL_SELECT_SUMMARY_CHECKPOINT_META: &str = "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version
             FROM memory_summary_checkpoints
             WHERE session_id = ?1";
const SQL_SELECT_SUMMARY_CHECKPOINT_BODY: &str = "SELECT summary_body
             FROM memory_summary_checkpoint_bodies
             WHERE session_id = ?1";
const SQL_QUERY_SUMMARY_APPEND_MAINTENANCE_STATE: &str = "WITH checkpoint AS (
             SELECT summarized_through_turn_id,
                    summary_before_turn_id,
                    summary_body_bytes,
                    summary_budget_chars,
                    summary_window_size,
                    summary_format_version
             FROM memory_summary_checkpoints
             WHERE session_id = ?1
         )
         SELECT
             (SELECT id
              FROM turns
              WHERE session_id = ?1
                AND id > checkpoint.summary_before_turn_id
              ORDER BY id ASC
              LIMIT 1) AS summary_before_turn_id,
             checkpoint.summarized_through_turn_id,
             checkpoint.summary_before_turn_id AS checkpoint_summary_before_turn_id,
             checkpoint.summary_body_bytes,
             checkpoint.summary_budget_chars,
             checkpoint.summary_window_size,
             checkpoint.summary_format_version
         FROM (SELECT 1) AS seed
         LEFT JOIN checkpoint ON 1 = 1";
const SQL_UPSERT_SUMMARY_CHECKPOINT_METADATA: &str = "INSERT INTO memory_summary_checkpoints(
             session_id,
             summarized_through_turn_id,
             summary_before_turn_id,
             summary_body_bytes,
             summary_budget_chars,
             summary_window_size,
             summary_format_version,
             updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(session_id) DO UPDATE SET
             summarized_through_turn_id = excluded.summarized_through_turn_id,
             summary_before_turn_id = excluded.summary_before_turn_id,
             summary_body_bytes = excluded.summary_body_bytes,
             summary_budget_chars = excluded.summary_budget_chars,
             summary_window_size = excluded.summary_window_size,
             summary_format_version = excluded.summary_format_version,
             updated_at_ts = excluded.updated_at_ts";
const SQL_UPSERT_SUMMARY_CHECKPOINT_BODY: &str = "INSERT INTO memory_summary_checkpoint_bodies(
             session_id,
             summary_body
         ) VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET
             summary_body = excluded.summary_body";
const SQL_UPDATE_SUMMARY_CHECKPOINT_METADATA: &str = "UPDATE memory_summary_checkpoints
         SET summarized_through_turn_id = ?2,
             summary_before_turn_id = ?3,
             summary_budget_chars = ?4,
             summary_window_size = ?5,
             summary_format_version = ?6,
             updated_at_ts = ?7
         WHERE session_id = ?1";
const SQL_DELETE_SUMMARY_CHECKPOINT: &str =
    "DELETE FROM memory_summary_checkpoints WHERE session_id = ?1";

#[derive(Debug, Clone, Default)]
pub(super) struct ContextSnapshot {
    pub window_turns: Vec<PromptWindowTurn>,
    pub summary_body: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[cfg(test)]
struct RecentWindowTurns {
    turns: Vec<ConversationTurn>,
    summary_before_turn_id: Option<i64>,
    window_starts_at_session_origin: bool,
}

#[derive(Debug, Clone, Default)]
struct RecentPromptWindowTurns {
    turns: Vec<PromptWindowTurn>,
    summary_before_turn_id: Option<i64>,
    window_starts_at_session_origin: bool,
    checkpoint_meta_lookup: SummaryCheckpointMetaLookup,
}

#[derive(Debug, Clone, Default)]
enum SummaryCheckpointMetaLookup {
    #[default]
    Unknown,
    Known(Option<SummaryCheckpointMeta>),
}

#[derive(Debug, Clone)]
struct SummaryCheckpoint {
    summarized_through_turn_id: i64,
    summary_before_turn_id: Option<i64>,
    summary_body: String,
    summary_budget_chars: usize,
    summary_window_size: usize,
    summary_format_version: i64,
}

#[derive(Debug, Clone)]
struct SummaryCheckpointMeta {
    summarized_through_turn_id: i64,
    summary_before_turn_id: Option<i64>,
    summary_body_len: usize,
    summary_budget_chars: usize,
    summary_window_size: usize,
    summary_format_version: i64,
}

#[derive(Debug, Clone)]
struct SummaryAppendMaintenanceState {
    summary_before_turn_id: Option<i64>,
    checkpoint_meta: Option<SummaryCheckpointMeta>,
}

struct AppendTurnResult {
    db_path: PathBuf,
    ts: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct CanonicalMemorySearchHit {
    pub record: CanonicalMemoryRecord,
    pub session_turn_index: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceMemoryIndexedSearchHit {
    pub label: String,
    pub path: String,
    pub document_kind: WorkspaceMemoryDocumentKind,
    pub body: String,
    pub body_line_offset: usize,
    pub freshness_ts: Option<i64>,
    pub content_hash: String,
    pub record_status: MemoryRecordStatus,
    pub trust_level: MemoryTrustLevel,
    pub authority: MemoryAuthority,
    pub derived_kind: DerivedMemoryKind,
    pub superseded_by: Option<String>,
}

struct WindowLoadResult {
    db_path: PathBuf,
    limit: usize,
    turns: Vec<ConversationTurn>,
    turn_count: Option<usize>,
}

struct TranscriptPageRow {
    id: i64,
    role: String,
    content: String,
    ts: i64,
}

enum ReplaceTurnsFailure {
    Conflict {
        expected_turn_count: usize,
        actual_turn_count: usize,
    },
    Message(String),
}

#[derive(Debug)]
struct SqliteRuntime {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl SqliteRuntime {
    fn new_with_diagnostics(
        path: PathBuf,
    ) -> Result<(Self, SqliteConnectionBootstrapDiagnostics), String> {
        let (connection, diagnostics) = open_sqlite_connection_with_diagnostics(&path)?;
        Ok((
            Self {
                path,
                connection: Mutex::new(connection),
            },
            diagnostics,
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn with_connection<T>(
        &self,
        operation: &'static str,
        callback: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let connection = self.connection.lock().map_err(|poisoned| {
            format!("lock sqlite runtime for {operation} failed: {poisoned}")
        })?;
        callback(&connection)
    }

    fn with_connection_mut<T>(
        &self,
        operation: &'static str,
        callback: impl FnOnce(&mut Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut connection = self.connection.lock().map_err(|poisoned| {
            format!("lock sqlite runtime for {operation} failed: {poisoned}")
        })?;
        callback(&mut connection)
    }
}

fn elapsed_ms(started_at: StdInstant) -> f64 {
    started_at.elapsed().as_secs_f64() * 1000.0
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

    let append = append_turn_internal(session_id, role, content, config)?;

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_APPEND_TURN,
            "session_id": session_id,
            "role": role,
            "ts": append.ts,
            "db_path": append.db_path.display().to_string(),
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
    let window = load_window_internal(session_id, window_limit, allow_extended_limit, config)?;

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_WINDOW,
            "session_id": session_id,
            "limit": window.limit,
            "allow_extended_limit": allow_extended_limit,
            "turns": window.turns,
            "turn_count": window.turn_count,
            "db_path": window.db_path.display().to_string(),
        }),
    })
}

pub(super) fn load_transcript(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.transcript payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.transcript requires payload.session_id".to_owned())?;
    let page_size = payload
        .get("page_size")
        .and_then(Value::as_u64)
        .unwrap_or(256)
        .clamp(1, 512) as usize;
    let turns = transcript_direct_paged(session_id, page_size, config)?;
    let runtime = acquire_memory_runtime(config)?;
    let path = runtime.path().to_path_buf();

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_TRANSCRIPT,
            "session_id": session_id,
            "page_size": page_size,
            "turns": turns,
            "turn_count": turns.len(),
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

    let runtime = acquire_memory_runtime(config)?;
    let affected = runtime.with_connection_mut("memory.clear_session", |conn| {
        let tx = conn
            .transaction()
            .map_err(|error| format!("begin memory clear transaction failed: {error}"))?;
        let affected = {
            let mut delete_turns = prepare_cached_sqlite_statement(
                &tx,
                SQL_DELETE_TURNS_FOR_SESSION,
                "prepare clear-session delete turns statement failed",
            )?;
            delete_turns
                .execute(rusqlite::params![session_id])
                .map_err(|error| format!("clear memory session failed: {error}"))?
        };
        delete_canonical_records_for_session(&tx, session_id)?;
        delete_session_state(&tx, session_id)?;
        delete_summary_checkpoint(&tx, session_id)?;
        tx.commit()
            .map_err(|error| format!("commit memory clear transaction failed: {error}"))?;
        Ok(affected)
    })?;
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

pub(super) fn replace_turns(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.replace_turns payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "memory.replace_turns requires payload.session_id".to_owned())
        .and_then(|value| {
            normalize_required_str(value, "memory.replace_turns requires payload.session_id")
        })?;
    let turns = payload
        .get("turns")
        .cloned()
        .ok_or_else(|| "memory.replace_turns requires payload.turns".to_owned())
        .and_then(|value| {
            serde_json::from_value::<Vec<WindowTurn>>(value)
                .map_err(|error| format!("memory.replace_turns payload.turns invalid: {error}"))
        })?;
    let expected_turn_count = match payload.get("expected_turn_count") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            "memory.replace_turns payload.expected_turn_count must be a non-negative integer"
                .to_owned()
        })? as usize),
    };

    match replace_turns_internal(session_id, &turns, expected_turn_count, config) {
        Ok(replaced) => Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "sqlite-core",
                "operation": MEMORY_OP_REPLACE_TURNS,
                "session_id": session_id,
                "replaced_turns": replaced,
            }),
        }),
        Err(ReplaceTurnsFailure::Conflict {
            expected_turn_count,
            actual_turn_count,
        }) => Ok(MemoryCoreOutcome {
            status: "conflict".to_owned(),
            payload: json!({
                "adapter": "sqlite-core",
                "operation": MEMORY_OP_REPLACE_TURNS,
                "session_id": session_id,
                "expected_turn_count": expected_turn_count,
                "actual_turn_count": actual_turn_count,
            }),
        }),
        Err(ReplaceTurnsFailure::Message(error)) => Err(error),
    }
}

#[cfg(test)]
pub(super) fn replace_session_turns_direct(
    session_id: &str,
    turns: &[WindowTurn],
    config: &MemoryRuntimeConfig,
) -> Result<(), String> {
    let _ = replace_turns_internal(session_id, turns, None, config)
        .map_err(|error| match error {
            ReplaceTurnsFailure::Conflict {
                expected_turn_count,
                actual_turn_count,
            } => format!(
                "memory.replace_turns conflict: expected turn count {expected_turn_count}, found {actual_turn_count}"
            ),
            ReplaceTurnsFailure::Message(message) => message,
        })?;
    Ok(())
}

pub(super) fn append_turn_direct(
    session_id: &str,
    role: &str,
    content: &str,
    config: &MemoryRuntimeConfig,
) -> Result<(), String> {
    let _ = append_turn_internal(session_id, role, content, config)?;
    Ok(())
}

pub(super) fn window_direct(
    session_id: &str,
    limit: usize,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    window_direct_with_options(session_id, limit, true, config)
}

pub(super) fn transcript_direct_paged(
    session_id: &str,
    page_size: usize,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    if page_size == 0 {
        return Err("memory transcript page_size must be >= 1".to_owned());
    }

    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("memory.transcript_direct_paged", |conn| {
        transcript_direct_paged_with_conn(conn, session_id, page_size)
    })
}

pub(crate) fn window_direct_with_conn(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<Vec<ConversationTurn>, String> {
    let (turns, _) = query_recent_turns(conn, session_id, limit)?;
    Ok(turns)
}

pub(crate) fn transcript_direct_paged_with_conn(
    conn: &Connection,
    session_id: &str,
    page_size: usize,
) -> Result<Vec<ConversationTurn>, String> {
    if page_size == 0 {
        return Err("memory transcript page_size must be >= 1".to_owned());
    }

    let upper_bound_turn_id = query_max_turn_id_for_session(conn, session_id)?;
    if upper_bound_turn_id <= 0 {
        return Ok(Vec::new());
    }

    let mut transcript = Vec::new();
    let mut last_seen_turn_id = 0_i64;

    loop {
        if last_seen_turn_id >= upper_bound_turn_id {
            break;
        }

        let page = query_transcript_page_after_id(
            conn,
            session_id,
            last_seen_turn_id,
            upper_bound_turn_id,
            page_size,
        )?;

        if page.is_empty() {
            break;
        }

        let next_last_seen_turn_id = page.last().map(|row| row.id).unwrap_or(last_seen_turn_id);

        for row in page {
            let turn = ConversationTurn {
                role: row.role,
                content: row.content,
                ts: row.ts,
            };
            transcript.push(turn);
        }

        last_seen_turn_id = next_last_seen_turn_id;
    }

    Ok(transcript)
}

pub(super) fn window_direct_with_options(
    session_id: &str,
    limit: usize,
    allow_extended_limit: bool,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    load_window_internal(session_id, limit, allow_extended_limit, config).map(|window| window.turns)
}

pub(super) fn load_context_snapshot(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<ContextSnapshot, String> {
    let (snapshot, _) = load_context_snapshot_with_diagnostics(session_id, config)?;
    Ok(snapshot)
}

pub(super) fn load_context_snapshot_with_diagnostics(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<(ContextSnapshot, SqliteContextLoadDiagnostics), String> {
    let window_limit = default_window_size(config);
    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("memory.context_snapshot", |conn| {
        let mut diagnostics = SqliteContextLoadDiagnostics::default();
        let total_started_at = StdInstant::now();
        let (window_turns, summary_body) =
            if matches!(config.mode, crate::config::MemoryMode::WindowPlusSummary) {
                let window_query_started_at = StdInstant::now();
                let mut window_diagnostics = PromptWindowQueryDiagnostics::default();
                let recent_window = query_recent_prompt_turns_with_overflow_probe(
                    conn,
                    session_id,
                    window_limit,
                    Some(&mut window_diagnostics),
                )?;
                diagnostics.window_query_ms = elapsed_ms(window_query_started_at);
                window_diagnostics.write_into(&mut diagnostics);
                let summary_body = if recent_window.window_starts_at_session_origin {
                    None
                } else {
                    materialize_summary_checkpoint_with_diagnostics(
                        conn,
                        session_id,
                        recent_window.summary_before_turn_id,
                        recent_window.checkpoint_meta_lookup.clone(),
                        config,
                        &mut diagnostics,
                    )?
                    .map(|checkpoint| checkpoint.summary_body)
                };
                (recent_window.turns, summary_body)
            } else {
                let window_query_started_at = StdInstant::now();
                let exact_query_started_at = StdInstant::now();
                let turns = query_recent_prompt_turns(conn, session_id, window_limit)?;
                diagnostics.window_query_ms = elapsed_ms(window_query_started_at);
                diagnostics.window_exact_rows_query_ms = elapsed_ms(exact_query_started_at);
                (turns, None)
            };

        diagnostics.total_ms = elapsed_ms(total_started_at);
        Ok((
            ContextSnapshot {
                window_turns,
                summary_body,
            },
            diagnostics,
        ))
    })
}

pub(super) fn load_summary_body_for_durable_flush(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Option<String>, String> {
    let window_limit = default_window_size(config);
    let runtime = acquire_memory_runtime(config)?;

    runtime.with_connection("memory.durable_flush_summary", |conn| {
        let recent_window =
            query_recent_prompt_turns_with_overflow_probe(conn, session_id, window_limit, None)?;
        if recent_window.window_starts_at_session_origin {
            return Ok(None);
        }

        let checkpoint_meta = match recent_window.checkpoint_meta_lookup {
            SummaryCheckpointMetaLookup::Known(checkpoint_meta) => checkpoint_meta,
            SummaryCheckpointMetaLookup::Unknown => load_summary_checkpoint_meta(conn, session_id)?,
        };
        let checkpoint = materialize_summary_checkpoint(
            conn,
            session_id,
            recent_window.summary_before_turn_id,
            checkpoint_meta,
            config,
        )?;

        Ok(checkpoint.map(|value| value.summary_body))
    })
}

pub(crate) fn load_latest_task_progress_record(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Option<TaskProgressRecord>, String> {
    let runtime = acquire_memory_runtime(config)?;

    runtime.with_connection("memory.latest_task_progress_record", |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload_json
                 FROM session_events
                 WHERE session_id = ?1 AND event_kind = ?2
                 ORDER BY id DESC
                 LIMIT 1",
            )
            .map_err(|error| {
                format!("prepare latest task progress record query failed: {error}")
            })?;
        let payload_json = stmt
            .query_row(
                rusqlite::params![session_id, TASK_PROGRESS_EVENT_KIND],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("query latest task progress record failed: {error}"))?;
        let Some(payload_json) = payload_json else {
            return Ok(None);
        };

        let payload = serde_json::from_str::<Value>(payload_json.as_str()).map_err(|error| {
            format!("decode latest task progress record payload failed: {error}")
        })?;
        Ok(task_progress_from_event_payload(&payload))
    })
}

pub(crate) fn load_session_metadata_hint(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Option<SessionMetadataHint>, String> {
    let runtime = acquire_memory_runtime(config)?;

    runtime.with_connection("memory.session_metadata_hint", |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, kind, state, parent_session_id, label
                 FROM sessions
                 WHERE session_id = ?1
                 LIMIT 1",
            )
            .map_err(|error| format!("prepare session metadata hint query failed: {error}"))?;
        let raw = stmt
            .query_row(rusqlite::params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .optional()
            .map_err(|error| format!("query session metadata hint failed: {error}"))?;
        let Some((resolved_session_id, kind, state, parent_session_id, label)) = raw else {
            return Ok(None);
        };

        let parent_label = match parent_session_id.as_deref() {
            Some(parent_session_id) => load_session_label_with_conn(conn, parent_session_id)?,
            None => None,
        };
        let lineage_root_session_id =
            load_lineage_root_session_id_with_conn(conn, resolved_session_id.as_str())?;
        let lineage_root_label = match lineage_root_session_id.as_deref() {
            Some(lineage_root_session_id) => {
                load_session_label_with_conn(conn, lineage_root_session_id)?
            }
            None => None,
        };
        let lineage_depth =
            load_session_lineage_depth_with_conn(conn, resolved_session_id.as_str())?;
        let workflow_hint = load_session_workflow_hint_with_conn(
            conn,
            kind.as_str(),
            state.as_str(),
            resolved_session_id.as_str(),
        )?;

        Ok(Some(SessionMetadataHint {
            kind,
            state,
            parent_session_id,
            label,
            parent_label,
            lineage_root_session_id,
            lineage_root_label,
            lineage_depth,
            workflow_task: workflow_hint.as_ref().and_then(|hint| hint.task.clone()),
            workflow_phase: workflow_hint.as_ref().and_then(|hint| hint.phase.clone()),
            workflow_operation_kind: workflow_hint
                .as_ref()
                .and_then(|hint| hint.operation_kind.clone()),
            workflow_operation_scope: workflow_hint
                .as_ref()
                .and_then(|hint| hint.operation_scope.clone()),
        }))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionWorkflowHint {
    task: Option<String>,
    phase: Option<String>,
    operation_kind: Option<String>,
    operation_scope: Option<String>,
}

fn load_session_label_with_conn(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT label FROM sessions WHERE session_id = ?1 LIMIT 1",
        rusqlite::params![session_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|value| value.flatten())
    .map_err(|error| format!("query session label failed: {error}"))
}

fn load_session_lineage_depth_with_conn(
    conn: &Connection,
    session_id: &str,
) -> Result<usize, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut next_session_id = Some(session_id.to_owned());
    let mut depth = 0usize;

    while let Some(current_session_id) = next_session_id {
        if !seen.insert(current_session_id.clone()) {
            return Err(format!(
                "session_lineage_cycle_detected: `{current_session_id}` reappeared while computing metadata hint depth"
            ));
        }
        let maybe_parent = conn
            .query_row(
                "SELECT parent_session_id FROM sessions WHERE session_id = ?1 LIMIT 1",
                rusqlite::params![current_session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| format!("query session lineage depth failed: {error}"))?;
        let Some(parent_session_id) = maybe_parent.flatten() else {
            return Ok(depth);
        };
        depth += 1;
        next_session_id = Some(parent_session_id);
    }

    Ok(depth)
}

fn load_lineage_root_session_id_with_conn(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<String>, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut next_session_id = Some(session_id.to_owned());

    while let Some(current_session_id) = next_session_id {
        if !seen.insert(current_session_id.clone()) {
            return Err(format!(
                "session_lineage_cycle_detected: `{current_session_id}` reappeared while computing metadata hint root"
            ));
        }
        let raw = conn
            .query_row(
                "SELECT session_id, parent_session_id FROM sessions WHERE session_id = ?1 LIMIT 1",
                rusqlite::params![current_session_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|error| format!("query session lineage root failed: {error}"))?;
        let Some((resolved_session_id, parent_session_id)) = raw else {
            return Ok(None);
        };
        match parent_session_id {
            Some(parent_session_id) => next_session_id = Some(parent_session_id),
            None => return Ok(Some(resolved_session_id)),
        }
    }

    Ok(None)
}

fn load_session_workflow_hint_with_conn(
    conn: &Connection,
    kind: &str,
    state: &str,
    session_id: &str,
) -> Result<Option<SessionWorkflowHint>, String> {
    let is_delegate_child = kind == "delegate_child";
    if !is_delegate_child {
        return Ok(None);
    }

    let task = load_latest_delegate_task_hint_with_conn(conn, session_id)?;
    let phase = match state {
        "ready" | "running" => Some("execute".to_owned()),
        "completed" => Some("complete".to_owned()),
        "failed" | "timed_out" => Some("failed".to_owned()),
        _ => None,
    };

    Ok(Some(SessionWorkflowHint {
        task,
        phase,
        operation_kind: Some("task".to_owned()),
        operation_scope: Some("task".to_owned()),
    }))
}

fn load_latest_delegate_task_hint_with_conn(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT payload_json
             FROM session_events
             WHERE session_id = ?1
               AND event_kind LIKE 'delegate_%'
             ORDER BY id DESC
             LIMIT 16",
        )
        .map_err(|error| format!("prepare latest delegate task hint query failed: {error}"))?;

    let rows = stmt
        .query_map(rusqlite::params![session_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("query latest delegate task hint failed: {error}"))?;

    for row in rows {
        let payload_json =
            row.map_err(|error| format!("read latest delegate task hint row failed: {error}"))?;
        let payload = serde_json::from_str::<Value>(payload_json.as_str())
            .map_err(|error| format!("decode latest delegate task hint payload failed: {error}"))?;
        let task = payload
            .get("task")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if task.is_some() {
            return Ok(task);
        }
    }

    Ok(None)
}

pub(super) fn ensure_memory_db_ready(
    path: Option<PathBuf>,
    config: &MemoryRuntimeConfig,
) -> Result<PathBuf, String> {
    let (path, _) = ensure_memory_db_ready_with_diagnostics(path, config)?;
    Ok(path)
}

pub(super) fn ensure_memory_db_ready_with_diagnostics(
    path: Option<PathBuf>,
    config: &MemoryRuntimeConfig,
) -> Result<(PathBuf, SqliteBootstrapDiagnostics), String> {
    let effective = path.unwrap_or_else(|| resolve_db_path(config));
    let (runtime, diagnostics) = acquire_sqlite_runtime_with_diagnostics(effective)?;
    runtime.with_connection_mut("memory.ensure_db_ready", |conn| {
        ensure_sqlite_runtime_schema_ready(conn)
    })?;
    Ok((runtime.path().to_path_buf(), diagnostics))
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
    crate::config::default_loong_home().join("memory.sqlite3")
}

fn absolutize_runtime_db_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let cwd = std::env::current_dir()
        .map_err(|error| format!("read current dir for sqlite path failed: {error}"))?;
    Ok(cwd.join(path))
}

fn lexical_normalize_runtime_db_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }

    if normalized.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        normalized
    }
}

fn query_transcript_page_after_id(
    conn: &Connection,
    session_id: &str,
    after_id: i64,
    upper_bound_turn_id: i64,
    page_size: usize,
) -> Result<Vec<TranscriptPageRow>, String> {
    let page_size_i64 = i64::try_from(page_size).map_err(|conversion_error| {
        format!("memory transcript page_size exceeds SQLite LIMIT range: {conversion_error}")
    })?;
    let mut statement = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_TURNS_AFTER_ID_WITH_LIMIT,
        "prepare transcript page query failed",
    )?;
    let rows = statement
        .query_map(
            rusqlite::params![session_id, after_id, upper_bound_turn_id, page_size_i64],
            |row| {
                Ok(TranscriptPageRow {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    ts: row.get(3)?,
                })
            },
        )
        .map_err(|error| format!("query transcript page failed: {error}"))?;

    let mut page = Vec::new();

    for row in rows {
        let transcript_row =
            row.map_err(|error| format!("decode transcript page row failed: {error}"))?;
        page.push(transcript_row);
    }

    Ok(page)
}

fn query_max_turn_id_for_session(conn: &Connection, session_id: &str) -> Result<i64, String> {
    conn.query_row(
        SQL_QUERY_MAX_TURN_ID_FOR_SESSION,
        rusqlite::params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .map_err(|error| format!("query transcript upper bound failed: {error}"))
}

/// Walk up the directory tree to find the deepest existing ancestor, canonicalize it
/// via [`dunce::canonicalize`], and reattach the remaining path components.  This
/// resolves Windows 8.3 short names (e.g. `RUNNER~1` -> `runneradmin`) even when
/// the target file and its immediate parent directory do not yet exist.
fn canonicalize_existing_ancestor(path: &Path) -> PathBuf {
    let mut remaining = Vec::new();
    let mut current = path.to_path_buf();

    while !current.exists() {
        let Some(name) = current.file_name().map(|n| n.to_os_string()) else {
            return path.to_path_buf();
        };
        remaining.push(name);
        let Some(parent) = current.parent().map(|p| p.to_path_buf()) else {
            return path.to_path_buf();
        };
        if parent == current {
            return path.to_path_buf();
        }
        current = parent;
    }

    match dunce::canonicalize(&current) {
        Ok(mut canonical) => {
            for component in remaining.into_iter().rev() {
                canonical.push(component);
            }
            canonical
        }
        Err(_) => path.to_path_buf(),
    }
}

fn normalize_runtime_db_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = lexical_normalize_runtime_db_path(&absolutize_runtime_db_path(path)?);
    if let Some(normalized_path) = sqlite_runtime_path_alias_registry()
        .lock()
        .map_err(|poisoned| format!("lock sqlite runtime path alias registry failed: {poisoned}"))?
        .get(&absolute)
        .cloned()
    {
        #[cfg(test)]
        test_support::record_runtime_path_normalization_alias_hit();
        return Ok(normalized_path);
    }

    #[cfg(test)]
    test_support::record_runtime_path_normalization_full();

    let normalized = if absolute.exists() {
        dunce::canonicalize(&absolute)
            .map_err(|error| format!("canonicalize sqlite db path failed: {error}"))?
    } else {
        let Some(file_name) = absolute.file_name() else {
            return Ok(absolute);
        };
        let Some(parent) = absolute.parent() else {
            return Ok(absolute);
        };

        match dunce::canonicalize(parent) {
            Ok(canonical_parent) => canonical_parent.join(file_name),
            Err(_) => canonicalize_existing_ancestor(&absolute),
        }
    };

    let mut alias_registry = sqlite_runtime_path_alias_registry()
        .lock()
        .map_err(|poisoned| {
            format!("lock sqlite runtime path alias registry failed: {poisoned}")
        })?;
    alias_registry.insert(absolute, normalized.clone());
    alias_registry.insert(normalized.clone(), normalized.clone());
    Ok(normalized)
}

#[cfg(test)]
fn normalize_runtime_db_path_best_effort(path: &Path) -> PathBuf {
    normalize_runtime_db_path(path)
        .or_else(|_| {
            absolutize_runtime_db_path(path)
                .map(|absolute| lexical_normalize_runtime_db_path(&absolute))
        })
        .unwrap_or_else(|_| path.to_path_buf())
}

fn append_turn_internal(
    session_id: &str,
    role: &str,
    content: &str,
    config: &MemoryRuntimeConfig,
) -> Result<AppendTurnResult, String> {
    let session_id =
        normalize_required_str(session_id, "memory.append_turn requires payload.session_id")?;
    let role = normalize_required_str(role, "memory.append_turn requires payload.role")?;
    let ts = unix_ts_now();
    let runtime = acquire_memory_runtime(config)?;
    let path = runtime.path().to_path_buf();
    runtime.with_connection_mut("memory.append_turn", |conn| {
        let tx = conn
            .transaction()
            .map_err(|error| format!("begin memory append transaction failed: {error}"))?;
        let next_session_turn_index = reserve_next_session_turn_index(&tx, session_id)?;
        {
            let mut insert_turn = prepare_cached_sqlite_statement(
                &tx,
                SQL_INSERT_TURN,
                "prepare append-turn insert statement failed",
            )?;
            insert_turn
                .execute(rusqlite::params![
                    session_id,
                    next_session_turn_index,
                    role,
                    content,
                    ts
                ])
                .map_err(|error| format!("insert memory turn failed: {error}"))?;
        }
        insert_canonical_record(
            &tx,
            build_canonical_insert_input(session_id, next_session_turn_index, role, content, ts),
        )?;
        append_turn_session_tree_node(&tx, session_id, role, content, next_session_turn_index, ts)?;

        let summary_window_size = default_window_size(config);
        if matches!(config.mode, crate::config::MemoryMode::WindowPlusSummary)
            && (next_session_turn_index as usize) > summary_window_size
        {
            if (next_session_turn_index as usize) == summary_window_size.saturating_add(1) {
                let summary_budget_chars = config.summary_max_chars.max(256);
                let _ = materialize_initial_summary_checkpoint(
                    &tx,
                    session_id,
                    summary_budget_chars,
                    summary_window_size,
                )?;
            } else {
                let append_maintenance_state =
                    load_summary_append_maintenance_state(&tx, session_id, summary_window_size)?;
                maintain_summary_checkpoint_after_append(
                    &tx,
                    session_id,
                    append_maintenance_state,
                    config,
                )?;
            }
        }

        tx.commit()
            .map_err(|error| format!("commit memory append transaction failed: {error}"))?;
        Ok(())
    })?;

    Ok(AppendTurnResult { db_path: path, ts })
}

struct CanonicalInsertInput {
    session_id: String,
    session_turn_index: i64,
    scope: MemoryScope,
    kind: CanonicalMemoryKind,
    role: Option<String>,
    content: String,
    metadata_json: String,
    search_text: String,
    ts: i64,
}

const ACTIVE_SESSION_HEAD_NAME: &str = "active";

fn session_root_node_id(session_id: &str) -> String {
    format!("session-root:{session_id}")
}

fn session_turn_node_id(session_id: &str, session_turn_index: i64) -> String {
    format!("session-turn:{session_id}:{session_turn_index}")
}

fn append_turn_session_tree_node(
    conn: &Connection,
    session_id: &str,
    role: &str,
    content: &str,
    session_turn_index: i64,
    ts: i64,
) -> Result<(), String> {
    let session_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            format!("probe session existence for session tree append failed: {error}")
        })?;
    if !session_exists {
        return Ok(());
    }

    let root_node_id = session_root_node_id(session_id);
    conn.execute(
        "INSERT OR IGNORE INTO session_nodes(
            node_id,
            session_id,
            parent_node_id,
            node_kind,
            role,
            content,
            session_turn_index,
            metadata_json,
            created_at
         ) VALUES (?1, ?2, NULL, 'root', NULL, NULL, NULL, '{}', ?3)",
        rusqlite::params![root_node_id, session_id, ts],
    )
    .map_err(|error| format!("ensure session root node during append failed: {error}"))?;

    conn.execute(
        "INSERT OR IGNORE INTO session_heads(
            session_id,
            head_name,
            node_id,
            head_mode,
            updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            session_id,
            ACTIVE_SESSION_HEAD_NAME,
            root_node_id,
            "live",
            ts
        ],
    )
    .map_err(|error| format!("ensure active session head during append failed: {error}"))?;

    let parent_node_id = conn
        .query_row(
            "SELECT node_id
             FROM session_heads
             WHERE session_id = ?1 AND head_name = ?2",
            rusqlite::params![session_id, ACTIVE_SESSION_HEAD_NAME],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("load active session head during append failed: {error}"))?;
    let node_id = session_turn_node_id(session_id, session_turn_index);
    let node_kind = if role == "user" {
        "user_turn"
    } else {
        "assistant_turn"
    };

    conn.execute(
        "INSERT OR IGNORE INTO session_nodes(
            node_id,
            session_id,
            parent_node_id,
            node_kind,
            role,
            content,
            session_turn_index,
            metadata_json,
            created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}', ?8)",
        rusqlite::params![
            node_id,
            session_id,
            parent_node_id,
            node_kind,
            role,
            content,
            session_turn_index,
            ts
        ],
    )
    .map_err(|error| format!("insert session turn node during append failed: {error}"))?;

    conn.execute(
        "UPDATE session_heads
         SET node_id = ?3, updated_at = ?4
         WHERE session_id = ?1 AND head_name = ?2",
        rusqlite::params![session_id, ACTIVE_SESSION_HEAD_NAME, node_id, ts],
    )
    .map_err(|error| format!("advance active session head during append failed: {error}"))?;

    Ok(())
}

fn rebuild_linear_session_tree_for_turns(
    conn: &Connection,
    session_id: &str,
    turns: &[WindowTurn],
) -> Result<(), String> {
    let session_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            format!("probe session existence for session tree rebuild failed: {error}")
        })?;
    if !session_exists {
        return Ok(());
    }

    let root_created_at = conn
        .query_row(
            "SELECT created_at FROM sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("load session created_at for tree rebuild failed: {error}"))?;
    let root_node_id = session_root_node_id(session_id);

    conn.execute(
        "DELETE FROM session_nodes WHERE session_id = ?1",
        rusqlite::params![session_id],
    )
    .map_err(|error| format!("clear session nodes during tree rebuild failed: {error}"))?;

    conn.execute(
        "INSERT INTO session_nodes(
            node_id,
            session_id,
            parent_node_id,
            node_kind,
            role,
            content,
            session_turn_index,
            metadata_json,
            created_at
         ) VALUES (?1, ?2, NULL, 'root', NULL, NULL, NULL, '{}', ?3)",
        rusqlite::params![root_node_id, session_id, root_created_at],
    )
    .map_err(|error| format!("insert session root node during tree rebuild failed: {error}"))?;

    let mut parent_node_id = root_node_id.clone();
    let mut active_node_id = root_node_id;
    let mut active_updated_at = root_created_at;

    for (index, turn) in turns.iter().enumerate() {
        let session_turn_index = (index + 1) as i64;
        let role =
            normalize_required_str(&turn.role, "memory.replace_turns requires turns[*].role")?;
        let ts = turn
            .ts
            .ok_or_else(|| "memory.replace_turns requires turns[*].ts".to_owned())?;
        let node_id = session_turn_node_id(session_id, session_turn_index);
        let node_kind = if role == "user" {
            "user_turn"
        } else {
            "assistant_turn"
        };

        conn.execute(
            "INSERT INTO session_nodes(
                node_id,
                session_id,
                parent_node_id,
                node_kind,
                role,
                content,
                session_turn_index,
                metadata_json,
                created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}', ?8)",
            rusqlite::params![
                node_id,
                session_id,
                parent_node_id,
                node_kind,
                role,
                &turn.content,
                session_turn_index,
                ts
            ],
        )
        .map_err(|error| format!("insert session turn node during tree rebuild failed: {error}"))?;

        parent_node_id = node_id.clone();
        active_node_id = node_id;
        active_updated_at = ts;
    }

    conn.execute(
        "INSERT INTO session_heads(
            session_id,
            head_name,
            node_id,
            head_mode,
            updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(session_id, head_name) DO UPDATE SET
            node_id = excluded.node_id,
            head_mode = excluded.head_mode,
            updated_at = excluded.updated_at",
        rusqlite::params![
            session_id,
            ACTIVE_SESSION_HEAD_NAME,
            active_node_id,
            "live",
            active_updated_at
        ],
    )
    .map_err(|error| format!("insert active session head during tree rebuild failed: {error}"))?;

    Ok(())
}

fn build_canonical_insert_input(
    session_id: &str,
    session_turn_index: i64,
    role: &str,
    content: &str,
    ts: i64,
) -> CanonicalInsertInput {
    let record = canonical_memory_record_from_persisted_turn(session_id, role, content);
    let metadata_json = record.metadata.to_string();
    let search_text = canonical_record_search_text(
        session_id,
        record.scope.as_str(),
        record.kind.as_str(),
        record.role.as_deref(),
        record.content.as_str(),
        metadata_json.as_str(),
    );
    CanonicalInsertInput {
        session_id: session_id.to_owned(),
        session_turn_index,
        scope: record.scope,
        kind: record.kind,
        role: record.role,
        content: record.content,
        metadata_json,
        search_text,
        ts,
    }
}

fn insert_canonical_record(conn: &Connection, input: CanonicalInsertInput) -> Result<(), String> {
    let mut insert_record = prepare_cached_sqlite_statement(
        conn,
        SQL_INSERT_CANONICAL_RECORD,
        "prepare canonical memory insert failed",
    )?;
    insert_record
        .execute(rusqlite::params![
            input.session_id,
            input.session_turn_index,
            input.scope.as_str(),
            input.kind.as_str(),
            input.role,
            input.content,
            input.metadata_json,
            input.search_text,
            input.ts,
        ])
        .map(|_| ())
        .map_err(|error| format!("insert canonical memory record failed: {error}"))
}

fn canonical_record_search_text(
    session_id: &str,
    scope: &str,
    kind: &str,
    role: Option<&str>,
    content: &str,
    metadata_json: &str,
) -> String {
    let mut fragments = vec![session_id, scope, kind, content, metadata_json];
    if let Some(role) = role {
        fragments.push(role);
    }

    build_search_index_text(fragments.as_slice())
}

fn replace_turns_internal(
    session_id: &str,
    turns: &[WindowTurn],
    expected_turn_count: Option<usize>,
    config: &MemoryRuntimeConfig,
) -> Result<usize, ReplaceTurnsFailure> {
    let session_id = normalize_required_str(
        session_id,
        "memory.replace_turns requires payload.session_id",
    )
    .map_err(ReplaceTurnsFailure::Message)?;
    let runtime = acquire_memory_runtime(config).map_err(ReplaceTurnsFailure::Message)?;

    runtime
        .with_connection_mut("memory.replace_turns", |conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| format!("begin memory replace transaction failed: {error}"))?;

            if let Some(expected_turn_count) = expected_turn_count {
                let actual_turn_count = resolve_actual_turn_count(&tx, session_id)? as usize;
                if actual_turn_count != expected_turn_count {
                    return Ok(Err(ReplaceTurnsFailure::Conflict {
                        expected_turn_count,
                        actual_turn_count,
                    }));
                }
            }

            {
                let mut delete_turns = prepare_cached_sqlite_statement(
                    &tx,
                    SQL_DELETE_TURNS_FOR_SESSION,
                    "prepare replace-turns delete statement failed",
                )?;
                delete_turns
                    .execute(rusqlite::params![session_id])
                    .map_err(|error| format!("delete memory turns failed: {error}"))?;
            }

            delete_session_state(&tx, session_id)?;
            delete_summary_checkpoint(&tx, session_id)?;
            delete_canonical_records_for_session(&tx, session_id)?;

            if !turns.is_empty() {
                {
                    let mut insert_turn = prepare_cached_sqlite_statement(
                        &tx,
                        SQL_INSERT_TURN,
                        "prepare replace-turns insert statement failed",
                    )?;
                    for (index, turn) in turns.iter().enumerate() {
                        let role = normalize_required_str(
                            &turn.role,
                            "memory.replace_turns requires turns[*].role",
                        )?;
                        let ts = turn.ts.ok_or_else(|| {
                            "memory.replace_turns requires turns[*].ts".to_owned()
                        })?;
                        insert_turn
                            .execute(rusqlite::params![
                                session_id,
                                (index + 1) as i64,
                                role,
                                &turn.content,
                                ts
                            ])
                            .map_err(|error| {
                                format!("insert replaced memory turn failed: {error}")
                            })?;
                        insert_canonical_record(
                            &tx,
                            build_canonical_insert_input(
                                session_id,
                                (index + 1) as i64,
                                role,
                                &turn.content,
                                ts,
                            ),
                        )?;
                    }
                }

                let mut set_turn_count = prepare_cached_sqlite_statement(
                    &tx,
                    SQL_SET_SESSION_TURN_COUNT,
                    "prepare replace-turns session-state statement failed",
                )?;
                set_turn_count
                    .execute(rusqlite::params![session_id, turns.len() as i64])
                    .map_err(|error| {
                        format!("upsert replace-turns session state failed: {error}")
                    })?;
            }

            rebuild_linear_session_tree_for_turns(&tx, session_id, turns)?;

            tx.commit()
                .map_err(|error| format!("commit memory replace transaction failed: {error}"))?;
            Ok(Ok(turns.len()))
        })
        .map_err(ReplaceTurnsFailure::Message)?
}

fn load_window_internal(
    session_id: &str,
    requested_limit: usize,
    allow_extended_limit: bool,
    config: &MemoryRuntimeConfig,
) -> Result<WindowLoadResult, String> {
    let session_id =
        normalize_required_str(session_id, "memory.window requires payload.session_id")?;
    let default_window = default_window_size(config).max(1);
    let hard_limit_cap = if allow_extended_limit { 512 } else { 128 };
    let effective_limit = if allow_extended_limit {
        requested_limit.clamp(1, hard_limit_cap)
    } else {
        requested_limit.clamp(1, hard_limit_cap).min(default_window)
    };
    let runtime = acquire_memory_runtime(config)?;
    let path = runtime.path().to_path_buf();
    let (turns, turn_count) = runtime.with_connection("memory.window", |conn| {
        query_recent_turns(conn, session_id, effective_limit)
    })?;
    Ok(WindowLoadResult {
        db_path: path,
        limit: effective_limit,
        turns,
        turn_count,
    })
}

fn normalize_required_str<'a>(
    value: &'a str,
    error_message: &'static str,
) -> Result<&'a str, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(error_message.to_owned());
    }
    Ok(trimmed)
}

fn prepare_cached_sqlite_statement<'conn>(
    conn: &'conn Connection,
    sql: &'static str,
    error_context: &'static str,
) -> Result<rusqlite::CachedStatement<'conn>, String> {
    #[cfg(test)]
    test_support::record_cached_prepare(sql);

    conn.prepare_cached(sql)
        .map_err(|error| format!("{error_context}: {error}"))
}

fn acquire_memory_runtime(config: &MemoryRuntimeConfig) -> Result<Arc<SqliteRuntime>, String> {
    let path = resolve_db_path(config);
    acquire_sqlite_runtime(path)
}

fn acquire_sqlite_runtime(path: PathBuf) -> Result<Arc<SqliteRuntime>, String> {
    let (runtime, _) = acquire_sqlite_runtime_with_diagnostics(path)?;
    Ok(runtime)
}

fn acquire_sqlite_runtime_with_diagnostics(
    path: PathBuf,
) -> Result<(Arc<SqliteRuntime>, SqliteBootstrapDiagnostics), String> {
    let mut diagnostics = SqliteBootstrapDiagnostics::default();
    let total_started_at = StdInstant::now();

    let normalize_started_at = StdInstant::now();
    let normalized_path = normalize_runtime_db_path(&path)?;
    diagnostics.normalize_path_ms = elapsed_ms(normalize_started_at);

    // Fast path: check cache under a short-lived lock.
    {
        let registry_lock_started_at = StdInstant::now();
        let registry = sqlite_runtime_registry()
            .lock()
            .map_err(|poisoned| format!("lock sqlite runtime registry failed: {poisoned}"))?;
        diagnostics.registry_lock_ms = elapsed_ms(registry_lock_started_at);

        let registry_lookup_started_at = StdInstant::now();
        if let Some(runtime) = registry.get(&normalized_path) {
            diagnostics.registry_lookup_ms = elapsed_ms(registry_lookup_started_at);
            diagnostics.cache_hit = true;
            diagnostics.total_ms = elapsed_ms(total_started_at);
            return Ok((runtime.clone(), diagnostics));
        }
        diagnostics.registry_lookup_ms = elapsed_ms(registry_lookup_started_at);
        // Lock drops here — cold bootstrap runs without blocking other paths.
    }

    #[cfg(test)]
    test_support::wait_for_sqlite_runtime_cache_miss(&normalized_path);

    let bootstrap_lock = {
        let mut bootstrap_registry =
            sqlite_runtime_bootstrap_lock_registry()
                .lock()
                .map_err(|poisoned| {
                    format!("lock sqlite runtime bootstrap registry failed: {poisoned}")
                })?;
        let bootstrap_entry = bootstrap_registry
            .entry(normalized_path.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())));
        bootstrap_entry.clone()
    };
    let _bootstrap_guard = bootstrap_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    {
        let registry_lock_started_at = StdInstant::now();
        let registry = sqlite_runtime_registry()
            .lock()
            .map_err(|poisoned| format!("lock sqlite runtime registry failed: {poisoned}"))?;
        diagnostics.registry_lock_ms += elapsed_ms(registry_lock_started_at);

        let registry_lookup_started_at = StdInstant::now();
        if let Some(runtime) = registry.get(&normalized_path) {
            diagnostics.registry_lookup_ms += elapsed_ms(registry_lookup_started_at);
            diagnostics.cache_hit = true;
            diagnostics.total_ms = elapsed_ms(total_started_at);
            return Ok((runtime.clone(), diagnostics));
        }
        diagnostics.registry_lookup_ms += elapsed_ms(registry_lookup_started_at);
    }

    // Slow path: bootstrap outside the global registry lock, but serialize cold
    // starts for the same normalized path so concurrent callers do not race
    // each other through connection configuration.
    let runtime_create_started_at = StdInstant::now();
    let (runtime, connection_diagnostics) =
        SqliteRuntime::new_with_diagnostics(normalized_path.clone())?;
    diagnostics.runtime_create_ms = elapsed_ms(runtime_create_started_at);
    diagnostics.parent_dir_create_ms = connection_diagnostics.parent_dir_create_ms;
    diagnostics.connection_open_ms = connection_diagnostics.connection_open_ms;
    diagnostics.configure_connection_ms = connection_diagnostics.configure_connection_ms;
    diagnostics.schema_init_ms = connection_diagnostics.schema_init_ms;
    diagnostics.schema_upgrade_ms = connection_diagnostics.schema_upgrade_ms;

    let runtime = Arc::new(runtime);
    let registry_insert_started_at = StdInstant::now();
    let mut registry = sqlite_runtime_registry()
        .lock()
        .map_err(|poisoned| format!("lock sqlite runtime registry failed: {poisoned}"))?;
    // Another thread may have bootstrapped the same path concurrently; use its
    // runtime if so, to avoid duplicate connections.
    if let Some(existing) = registry.get(&normalized_path) {
        diagnostics.registry_insert_ms = elapsed_ms(registry_insert_started_at);
        diagnostics.cache_hit = true;
        diagnostics.total_ms = elapsed_ms(total_started_at);
        return Ok((existing.clone(), diagnostics));
    }
    registry.insert(normalized_path, runtime.clone());
    diagnostics.registry_insert_ms = elapsed_ms(registry_insert_started_at);
    diagnostics.total_ms = elapsed_ms(total_started_at);
    Ok((runtime, diagnostics))
}

fn sqlite_runtime_registry() -> &'static Mutex<HashMap<PathBuf, Arc<SqliteRuntime>>> {
    static SQLITE_RUNTIME_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<SqliteRuntime>>>> =
        OnceLock::new();
    SQLITE_RUNTIME_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sqlite_runtime_bootstrap_lock_registry() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static SQLITE_RUNTIME_BOOTSTRAP_LOCK_REGISTRY: OnceLock<
        Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    > = OnceLock::new();
    SQLITE_RUNTIME_BOOTSTRAP_LOCK_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sqlite_runtime_path_alias_registry() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    static SQLITE_RUNTIME_PATH_ALIAS_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> =
        OnceLock::new();
    SQLITE_RUNTIME_PATH_ALIAS_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn open_sqlite_connection_with_diagnostics(
    path: &Path,
) -> Result<(Connection, SqliteConnectionBootstrapDiagnostics), String> {
    let mut diagnostics = SqliteConnectionBootstrapDiagnostics::default();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let create_dir_started_at = StdInstant::now();
        fs::create_dir_all(parent)
            .map_err(|error| format!("create sqlite parent directory failed: {error}"))?;
        diagnostics.parent_dir_create_ms = elapsed_ms(create_dir_started_at);
    }

    let connection_open_started_at = StdInstant::now();
    let mut conn =
        Connection::open(path).map_err(|error| format!("open sqlite memory db failed: {error}"))?;
    diagnostics.connection_open_ms = elapsed_ms(connection_open_started_at);

    let configure_started_at = StdInstant::now();
    configure_sqlite_connection(&conn)?;
    diagnostics.configure_connection_ms = elapsed_ms(configure_started_at);

    let schema_upgrade_started_at = StdInstant::now();
    let schema_probe = probe_sqlite_schema(&conn)?;
    let requires_current_schema_setup = schema_probe.requires_current_schema_setup();

    if requires_current_schema_setup {
        let schema_init_started_at = StdInstant::now();
        #[cfg(test)]
        test_support::record_sqlite_schema_init(path);
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS turns(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              session_turn_index INTEGER,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              ts INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_turns_session_id ON turns(session_id, id);
            CREATE TABLE IF NOT EXISTS memory_session_state(
              session_id TEXT PRIMARY KEY,
              turn_count INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_summary_checkpoints(
              session_id TEXT PRIMARY KEY,
              summarized_through_turn_id INTEGER NOT NULL,
              summary_before_turn_id INTEGER,
              summary_body_bytes INTEGER NOT NULL DEFAULT 0,
              summary_budget_chars INTEGER NOT NULL,
              summary_window_size INTEGER NOT NULL,
              summary_format_version INTEGER NOT NULL,
              updated_at_ts INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_summary_checkpoint_bodies(
              session_id TEXT PRIMARY KEY
                REFERENCES memory_summary_checkpoints(session_id) ON DELETE CASCADE,
              summary_body TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_canonical_records(
              record_id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              session_turn_index INTEGER NOT NULL,
              scope TEXT NOT NULL,
              kind TEXT NOT NULL,
              role TEXT NULL,
              content TEXT NOT NULL,
              metadata_json TEXT NOT NULL,
              search_text TEXT NOT NULL DEFAULT '',
              ts INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_canonical_records_session_turn
              ON memory_canonical_records(session_id, session_turn_index);
            CREATE INDEX IF NOT EXISTS idx_memory_canonical_records_scope_kind_ts
              ON memory_canonical_records(scope, kind, ts DESC, record_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS memory_canonical_records_fts
              USING fts5(
                content,
                session_id,
                scope,
                kind,
                role,
                metadata_json,
                search_text,
                content='memory_canonical_records',
                content_rowid='record_id'
              );
            CREATE TRIGGER IF NOT EXISTS memory_canonical_records_ai
              AFTER INSERT ON memory_canonical_records
            BEGIN
              INSERT INTO memory_canonical_records_fts(
                rowid,
                content,
                session_id,
                scope,
                kind,
                role,
                metadata_json,
                search_text
              )
              VALUES (
                new.record_id,
                new.content,
                new.session_id,
                new.scope,
                new.kind,
                COALESCE(new.role, ''),
                new.metadata_json,
                new.search_text
              );
            END;
            CREATE TRIGGER IF NOT EXISTS memory_canonical_records_ad
              AFTER DELETE ON memory_canonical_records
            BEGIN
              INSERT INTO memory_canonical_records_fts(
                memory_canonical_records_fts,
                rowid,
                content,
                session_id,
                scope,
                kind,
                role,
                metadata_json,
                search_text
              )
              VALUES (
                'delete',
                old.record_id,
                old.content,
                old.session_id,
                old.scope,
                old.kind,
                COALESCE(old.role, ''),
                old.metadata_json,
                old.search_text
              );
            END;
            CREATE TRIGGER IF NOT EXISTS memory_canonical_records_au
              AFTER UPDATE ON memory_canonical_records
            BEGIN
              INSERT INTO memory_canonical_records_fts(
                memory_canonical_records_fts,
                rowid,
                content,
                session_id,
                scope,
                kind,
                role,
                metadata_json,
                search_text
              )
              VALUES (
                'delete',
                old.record_id,
                old.content,
                old.session_id,
                old.scope,
                old.kind,
                COALESCE(old.role, ''),
                old.metadata_json,
                old.search_text
              );
              INSERT INTO memory_canonical_records_fts(
                rowid,
                content,
                session_id,
                scope,
                kind,
                role,
                metadata_json,
                search_text
              )
              VALUES (
                new.record_id,
                new.content,
                new.session_id,
                new.scope,
                new.kind,
                COALESCE(new.role, ''),
                new.metadata_json,
                new.search_text
              );
            END;
            CREATE TABLE IF NOT EXISTS approval_requests(
              approval_request_id TEXT PRIMARY KEY,
              session_id TEXT NOT NULL,
              turn_id TEXT NOT NULL,
              tool_call_id TEXT NOT NULL,
              tool_name TEXT NOT NULL,
              approval_key TEXT NOT NULL,
              status TEXT NOT NULL,
              decision TEXT NULL,
              request_payload_json TEXT NOT NULL,
              governance_snapshot_json TEXT NOT NULL,
              requested_at INTEGER NOT NULL,
              resolved_at INTEGER NULL,
              resolved_by_session_id TEXT NULL,
              executed_at INTEGER NULL,
              last_error TEXT NULL
            );
            CREATE TABLE IF NOT EXISTS approval_grants(
              scope_session_id TEXT NOT NULL,
              approval_key TEXT NOT NULL,
              created_by_session_id TEXT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY(scope_session_id, approval_key)
            );
            CREATE TABLE IF NOT EXISTS session_tool_consent(
              scope_session_id TEXT PRIMARY KEY,
              mode TEXT NOT NULL CHECK (mode IN ('prompt', 'auto', 'full')),
              updated_by_session_id TEXT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS session_tool_policies(
              session_id TEXT PRIMARY KEY,
              requested_tool_ids_json TEXT NOT NULL,
              runtime_narrowing_json TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_approval_requests_session_status_requested_at
              ON approval_requests(session_id, status, requested_at DESC, approval_request_id);
            ",
        )
        .map_err(|error| format!("initialize sqlite memory schema failed: {error}"))?;
        diagnostics.schema_init_ms = elapsed_ms(schema_init_started_at);
    }

    ensure_sqlite_runtime_schema_ready(&mut conn)?;
    diagnostics.schema_upgrade_ms = elapsed_ms(schema_upgrade_started_at);

    #[cfg(test)]
    test_support::record_sqlite_bootstrap(path);

    Ok((conn, diagnostics))
}

fn configure_sqlite_connection(conn: &Connection) -> Result<(), String> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("set sqlite journal_mode=WAL failed: {error}"))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| format!("set sqlite synchronous=NORMAL failed: {error}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| format!("set sqlite foreign_keys=ON failed: {error}"))?;
    conn.set_prepared_statement_cache_capacity(SQLITE_PREPARED_STATEMENT_CACHE_CAPACITY);
    conn.busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))
        .map_err(|error| format!("set sqlite busy_timeout failed: {error}"))?;
    Ok(())
}

fn read_sqlite_user_version(conn: &Connection) -> Result<i64, String> {
    conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("read sqlite user_version failed: {error}"))
}

#[derive(Debug, Clone, Copy)]
struct SqliteSchemaProbe {
    user_version: i64,
    current_schema_ready: bool,
}

impl SqliteSchemaProbe {
    fn requires_current_schema_setup(self) -> bool {
        if self.user_version > SQLITE_MEMORY_SCHEMA_VERSION {
            return false;
        }
        if self.user_version < SQLITE_MEMORY_SCHEMA_VERSION {
            return true;
        }
        !self.current_schema_ready
    }

    fn requires_repairs(self) -> bool {
        self.requires_current_schema_setup()
    }
}

fn write_sqlite_user_version(conn: &Connection, version: i64) -> Result<(), String> {
    conn.pragma_update(None, "user_version", version)
        .map_err(|error| format!("set sqlite user_version={version} failed: {error}"))
}

fn probe_sqlite_schema(conn: &Connection) -> Result<SqliteSchemaProbe, String> {
    let user_version = read_sqlite_user_version(conn)?;
    let current_schema_ready =
        user_version == SQLITE_MEMORY_SCHEMA_VERSION && sqlite_current_schema_objects_ready(conn)?;

    Ok(SqliteSchemaProbe {
        user_version,
        current_schema_ready,
    })
}

fn ensure_sqlite_schema_repairs_if_needed(conn: &mut Connection) -> Result<(), String> {
    let schema_probe = probe_sqlite_schema(conn)?;
    if !schema_probe.requires_repairs() {
        return Ok(());
    }

    ensure_turn_session_index_and_state_metadata(conn)?;
    ensure_session_terminal_outcome_storage(conn)?;
    ensure_session_event_search_storage(conn)?;
    ensure_approval_lifecycle_tables(conn)?;
    ensure_session_tool_consent_storage(conn)?;
    ensure_session_tool_policy_storage(conn)?;
    ensure_session_tree_storage(conn)?;
    ensure_summary_checkpoint_storage_layout(conn)?;
    ensure_canonical_record_storage(conn)?;
    ensure_workspace_memory_search_storage(conn)?;
    write_sqlite_user_version(conn, SQLITE_MEMORY_SCHEMA_VERSION)?;

    Ok(())
}

fn ensure_sqlite_runtime_schema_ready(conn: &mut Connection) -> Result<(), String> {
    ensure_sqlite_schema_repairs_if_needed(conn)?;
    ensure_control_plane_pairing_tables(conn)?;
    Ok(())
}

fn sqlite_current_schema_objects_ready(conn: &Connection) -> Result<bool, String> {
    let object_count = conn
        .query_row(SQL_COUNT_CURRENT_SCHEMA_OBJECTS, [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| format!("probe sqlite current schema objects failed: {error}"))?;
    let object_count_ready = object_count == SQLITE_CURRENT_SCHEMA_OBJECT_COUNT;
    let canonical_fts_ready = !canonical_record_fts_needs_rebuild(conn)?;
    let session_event_fts_ready = !session_event_fts_needs_rebuild(conn)?;
    let workspace_memory_search_ready = !workspace_memory_search_storage_needs_rebuild(conn)?;
    let terminal_outcome_storage_ready =
        sqlite_table_has_column(conn, "session_terminal_outcomes", "frozen_result_json")?;
    let session_head_mode_ready = sqlite_table_has_column(conn, "session_heads", "head_mode")?;

    Ok(object_count_ready
        && canonical_fts_ready
        && session_event_fts_ready
        && workspace_memory_search_ready
        && terminal_outcome_storage_ready
        && session_head_mode_ready)
}

fn ensure_turn_session_index_and_state_metadata(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("turn_session_index");

    if !sqlite_table_has_column(conn, "turns", "session_turn_index")? {
        conn.execute(
            "ALTER TABLE turns
             ADD COLUMN session_turn_index INTEGER",
            [],
        )
        .map_err(|error| format!("add session turn index column failed: {error}"))?;
    }

    conn.execute_batch(
        "
        WITH ranked AS (
            SELECT id,
                   ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY id ASC) AS session_turn_index
            FROM turns
        )
        UPDATE turns
        SET session_turn_index = (
            SELECT ranked.session_turn_index
            FROM ranked
            WHERE ranked.id = turns.id
        )
        WHERE session_turn_index IS NULL
           OR session_turn_index <= 0;
        CREATE UNIQUE INDEX IF NOT EXISTS idx_turns_session_turn_index
          ON turns(session_id, session_turn_index);
        CREATE TABLE IF NOT EXISTS memory_session_state(
          session_id TEXT PRIMARY KEY,
          turn_count INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_turns_session_id ON turns(session_id, id);
        CREATE TABLE IF NOT EXISTS sessions(
          session_id TEXT PRIMARY KEY,
          kind TEXT NOT NULL,
          parent_session_id TEXT NULL,
          label TEXT NULL,
          state TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          last_error TEXT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_parent_session_id
          ON sessions(parent_session_id, updated_at, session_id);
        CREATE TABLE IF NOT EXISTS session_events(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          event_kind TEXT NOT NULL,
          actor_session_id TEXT NULL,
          payload_json TEXT NOT NULL,
          search_text TEXT NOT NULL DEFAULT '',
          ts INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_events_session_id
          ON session_events(session_id, id);
        ",
    )
    .map_err(|error| format!("backfill session turn index metadata failed: {error}"))?;
    conn.execute_batch(SESSION_TERMINAL_OUTCOMES_DDL)
        .map_err(|error| format!("ensure session terminal outcome storage failed: {error}"))?;

    conn.execute_batch(SESSION_TERMINAL_OUTCOMES_TABLE_SQL)
        .map_err(|error| format!("backfill session terminal outcome storage failed: {error}"))?;

    conn.execute(
        "INSERT INTO memory_session_state(session_id, turn_count)
         SELECT session_id, MAX(session_turn_index)
         FROM turns
         WHERE session_turn_index IS NOT NULL
           AND session_turn_index > 0
         GROUP BY session_id
         ON CONFLICT(session_id) DO UPDATE SET
             turn_count = excluded.turn_count",
        [],
    )
    .map_err(|error| format!("backfill session turn count metadata failed: {error}"))?;

    conn.execute(
        "DELETE FROM memory_session_state
         WHERE session_id NOT IN (
             SELECT DISTINCT session_id
             FROM turns
         )",
        [],
    )
    .map_err(|error| format!("remove stale session turn count metadata failed: {error}"))?;

    Ok(())
}

fn ensure_session_tree_storage(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("session_tree");

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS session_nodes(
          node_id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          parent_node_id TEXT NULL,
          node_kind TEXT NOT NULL,
          role TEXT NULL,
          content TEXT NULL,
          session_turn_index INTEGER NULL,
          metadata_json TEXT NOT NULL DEFAULT '{}',
          created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_nodes_session_parent_created
          ON session_nodes(session_id, parent_node_id, created_at, node_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_session_nodes_session_turn_index
          ON session_nodes(session_id, session_turn_index)
          WHERE session_turn_index IS NOT NULL;
        CREATE TABLE IF NOT EXISTS session_heads(
          session_id TEXT NOT NULL,
          head_name TEXT NOT NULL,
          node_id TEXT NOT NULL,
          head_mode TEXT NOT NULL DEFAULT 'live',
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(session_id, head_name)
        );
        CREATE TABLE IF NOT EXISTS session_artifacts(
          artifact_id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          artifact_type TEXT NOT NULL,
          head_name TEXT NULL,
          anchor_node_id TEXT NULL,
          source_start_node_id TEXT NULL,
          source_end_node_id TEXT NULL,
          payload_json TEXT NOT NULL DEFAULT '{}',
          summary_text TEXT NULL,
          created_at INTEGER NOT NULL
        );
        ",
    )
    .map_err(|error| format!("ensure session tree storage failed: {error}"))?;

    ensure_session_head_mode_storage(conn)?;

    conn.execute(
        "INSERT OR IGNORE INTO session_nodes(
            node_id,
            session_id,
            parent_node_id,
            node_kind,
            role,
            content,
            session_turn_index,
            metadata_json,
            created_at
         )
         SELECT
            'session-root:' || session_id,
            session_id,
            NULL,
            'root',
            NULL,
            NULL,
            NULL,
            '{}',
            created_at
         FROM sessions",
        [],
    )
    .map_err(|error| format!("backfill session root nodes failed: {error}"))?;

    conn.execute(
        "INSERT OR IGNORE INTO session_nodes(
            node_id,
            session_id,
            parent_node_id,
            node_kind,
            role,
            content,
            session_turn_index,
            metadata_json,
            created_at
         )
         SELECT
            'session-turn:' || turns.session_id || ':' || turns.session_turn_index,
            turns.session_id,
            CASE
              WHEN turns.session_turn_index = 1 THEN 'session-root:' || turns.session_id
              ELSE 'session-turn:' || turns.session_id || ':' || (turns.session_turn_index - 1)
            END,
            CASE
              WHEN turns.role = 'user' THEN 'user_turn'
              ELSE 'assistant_turn'
            END,
            turns.role,
            turns.content,
            turns.session_turn_index,
            '{}',
            turns.ts
         FROM turns
         JOIN sessions ON sessions.session_id = turns.session_id
         WHERE turns.session_turn_index IS NOT NULL
           AND turns.session_turn_index > 0",
        [],
    )
    .map_err(|error| format!("backfill linear session turn nodes failed: {error}"))?;

    conn.execute(
        "INSERT OR IGNORE INTO session_heads(
            session_id,
            head_name,
            node_id,
            head_mode,
            updated_at
         )
         SELECT
            sessions.session_id,
            'active',
            COALESCE(
              (
                SELECT 'session-turn:' || turns.session_id || ':' || turns.session_turn_index
                FROM turns
                WHERE turns.session_id = sessions.session_id
                  AND turns.session_turn_index IS NOT NULL
                  AND turns.session_turn_index > 0
                ORDER BY turns.session_turn_index DESC
                LIMIT 1
              ),
              'session-root:' || sessions.session_id
            ),
            'live',
            sessions.updated_at
         FROM sessions",
        [],
    )
    .map_err(|error| format!("backfill active session heads failed: {error}"))?;

    Ok(())
}

fn ensure_session_head_mode_storage(conn: &Connection) -> Result<(), String> {
    if !sqlite_table_has_column(conn, "session_heads", "head_mode")? {
        conn.execute(
            "ALTER TABLE session_heads
             ADD COLUMN head_mode TEXT NOT NULL DEFAULT 'live'",
            [],
        )
        .map_err(|error| format!("add session head mode column failed: {error}"))?;
    }

    conn.execute(
        "UPDATE session_heads
         SET head_mode = 'live'
         WHERE head_mode IS NULL OR head_mode = ''",
        [],
    )
    .map_err(|error| format!("backfill default session head mode failed: {error}"))?;

    conn.execute(
        "UPDATE session_heads
         SET head_mode = 'pinned'
         WHERE head_name LIKE 'checkpoint/%'",
        [],
    )
    .map_err(|error| format!("backfill checkpoint session head mode failed: {error}"))?;

    conn.execute(
        "UPDATE session_heads
         SET head_mode = 'live'
         WHERE head_name = 'active'",
        [],
    )
    .map_err(|error| format!("normalize active session head mode failed: {error}"))?;

    Ok(())
}

fn ensure_session_terminal_outcome_storage(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("session_terminal_outcomes");

    conn.execute_batch(SESSION_TERMINAL_OUTCOMES_TABLE_SQL)
        .map_err(|error| format!("ensure session terminal outcome storage failed: {error}"))?;

    let has_frozen_result_column =
        sqlite_table_has_column(conn, "session_terminal_outcomes", "frozen_result_json")?;
    if !has_frozen_result_column {
        conn.execute(
            "ALTER TABLE session_terminal_outcomes
             ADD COLUMN frozen_result_json TEXT NULL",
            [],
        )
        .map_err(|error| {
            format!("add session terminal outcome frozen result column failed: {error}")
        })?;
    }

    Ok(())
}

fn ensure_session_event_search_storage(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("session_event_search");

    if !sqlite_table_has_column(conn, "session_events", "search_text")? {
        conn.execute(
            "ALTER TABLE session_events
             ADD COLUMN search_text TEXT NOT NULL DEFAULT ''",
            [],
        )
        .map_err(|error| format!("add session event search_text column failed: {error}"))?;
    }

    backfill_session_event_search_text(conn)?;
    create_session_event_fts_index(conn)?;

    if session_event_fts_needs_rebuild(conn)? {
        rebuild_session_event_search_storage(conn)?;
        return Ok(());
    }

    rebuild_session_event_search_storage_if_needed(conn)
}

fn backfill_session_event_search_text(conn: &Connection) -> Result<(), String> {
    let mut select_stmt = conn
        .prepare(
            "SELECT id, event_kind, payload_json
             FROM session_events
             WHERE search_text = '' OR search_text IS NULL",
        )
        .map_err(|error| format!("prepare session event search_text backfill failed: {error}"))?;
    let rows = select_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("query session event search_text backfill failed: {error}"))?;

    let mut pending_updates = Vec::new();
    for row in rows {
        let (event_id, event_kind, payload_json) = row.map_err(|error| {
            format!("decode session event search_text backfill row failed: {error}")
        })?;
        let search_text = session_event_search_text(event_kind.as_str(), payload_json.as_str());
        pending_updates.push((event_id, search_text));
    }
    drop(select_stmt);

    let mut update_stmt = conn
        .prepare(
            "UPDATE session_events
             SET search_text = ?2
             WHERE id = ?1",
        )
        .map_err(|error| format!("prepare session event search_text update failed: {error}"))?;
    for (event_id, search_text) in pending_updates {
        update_stmt
            .execute(rusqlite::params![event_id, search_text])
            .map_err(|error| format!("update session event search_text failed: {error}"))?;
    }

    Ok(())
}

fn session_event_fts_needs_rebuild(conn: &Connection) -> Result<bool, String> {
    let columns = sqlite_table_columns(conn, "session_events_fts")?;
    if columns.is_empty() {
        return Ok(false);
    }

    let required_columns = ["event_kind", "payload_json", "search_text"];
    let has_all_required_columns = required_columns.iter().all(|required_column| {
        columns
            .iter()
            .any(|current_column| current_column == required_column)
    });

    Ok(!has_all_required_columns)
}

fn drop_session_event_fts_index(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        DROP TRIGGER IF EXISTS session_events_ai;
        DROP TRIGGER IF EXISTS session_events_ad;
        DROP TRIGGER IF EXISTS session_events_au;
        DROP TABLE IF EXISTS session_events_fts;
        ",
    )
    .map_err(|error| format!("drop session event FTS index failed: {error}"))?;

    Ok(())
}

fn create_session_event_fts_index(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS session_events_fts
          USING fts5(
            event_kind,
            payload_json,
            search_text
          );
        CREATE TRIGGER IF NOT EXISTS session_events_ai
          AFTER INSERT ON session_events
        BEGIN
          INSERT INTO session_events_fts(
            rowid,
            event_kind,
            payload_json,
            search_text
          )
          VALUES (
            new.id,
            new.event_kind,
            new.payload_json,
            new.search_text
          );
        END;
        CREATE TRIGGER IF NOT EXISTS session_events_ad
          AFTER DELETE ON session_events
        BEGIN
          DELETE FROM session_events_fts
          WHERE rowid = old.id;
        END;
        CREATE TRIGGER IF NOT EXISTS session_events_au
          AFTER UPDATE ON session_events
        BEGIN
          DELETE FROM session_events_fts
          WHERE rowid = old.id;
          INSERT INTO session_events_fts(
            rowid,
            event_kind,
            payload_json,
            search_text
          )
          VALUES (
            new.id,
            new.event_kind,
            new.payload_json,
            new.search_text
          );
        END;
        ",
    )
    .map_err(|error| format!("create session event FTS index failed: {error}"))?;

    Ok(())
}

fn rebuild_session_event_fts_index_contents(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "
        INSERT INTO session_events_fts(
          rowid,
          event_kind,
          payload_json,
          search_text
        )
        SELECT id, event_kind, payload_json, search_text
        FROM session_events
        ",
        [],
    )
    .map(|_| ())
    .map_err(|error| format!("rebuild session event FTS contents failed: {error}"))?;

    Ok(())
}

fn rebuild_session_event_search_storage(conn: &Connection) -> Result<(), String> {
    drop_session_event_fts_index(conn)?;
    create_session_event_fts_index(conn)?;
    rebuild_session_event_fts_index_contents(conn)
}

fn rebuild_session_event_search_storage_if_needed(conn: &Connection) -> Result<(), String> {
    let session_event_count = conn
        .query_row("SELECT COUNT(*) FROM session_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| format!("count session events failed: {error}"))?;
    let session_event_fts_count = conn
        .query_row("SELECT COUNT(*) FROM session_events_fts", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| format!("count session event FTS rows failed: {error}"))?;

    if session_event_count == session_event_fts_count {
        return Ok(());
    }

    rebuild_session_event_search_storage(conn)
}

fn session_event_search_text(event_kind: &str, payload_json: &str) -> String {
    build_search_index_text(&[event_kind, payload_json])
}

#[derive(Debug, Clone)]
struct WorkspaceMemoryDocumentIndexRow {
    document_id: i64,
    label: String,
    modified_at_ms: i64,
}

#[derive(Debug, Clone)]
struct WorkspaceMemoryDocumentIndexEntry {
    workspace_root: String,
    path: String,
    label: String,
    document_kind: WorkspaceMemoryDocumentKind,
    modified_at_ms: i64,
    freshness_ts: Option<i64>,
    content_hash: String,
    record_status: MemoryRecordStatus,
    trust_level: MemoryTrustLevel,
    authority: MemoryAuthority,
    derived_kind: DerivedMemoryKind,
    superseded_by: Option<String>,
    body_line_offset: i64,
    body: String,
    search_text: String,
}

fn ensure_workspace_memory_search_storage(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("workspace_memory_search");

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS workspace_memory_documents(
          document_id INTEGER PRIMARY KEY AUTOINCREMENT,
          workspace_root TEXT NOT NULL,
          path TEXT NOT NULL,
          label TEXT NOT NULL,
          document_kind TEXT NOT NULL,
          modified_at_ms INTEGER NOT NULL,
          freshness_ts INTEGER NULL,
          content_hash TEXT NOT NULL,
          record_status TEXT NOT NULL,
          trust_level TEXT NOT NULL,
          authority TEXT NOT NULL,
          derived_kind TEXT NOT NULL,
          superseded_by TEXT NULL,
          body_line_offset INTEGER NOT NULL,
          body TEXT NOT NULL,
          search_text TEXT NOT NULL,
          UNIQUE(workspace_root, path)
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS workspace_memory_documents_fts
          USING fts5(
            label,
            body,
            search_text
          );
        ",
    )
    .map_err(|error| format!("ensure workspace memory search storage failed: {error}"))?;

    if workspace_memory_search_storage_needs_rebuild(conn)? {
        rebuild_workspace_memory_search_storage(conn)?;
    }

    Ok(())
}

fn workspace_memory_search_storage_needs_rebuild(conn: &Connection) -> Result<bool, String> {
    let document_columns = sqlite_table_columns(conn, "workspace_memory_documents")?;
    let fts_columns = sqlite_table_columns(conn, "workspace_memory_documents_fts")?;
    if document_columns.is_empty() || fts_columns.is_empty() {
        return Ok(false);
    }

    let required_document_columns = [
        "workspace_root",
        "path",
        "label",
        "document_kind",
        "modified_at_ms",
        "freshness_ts",
        "content_hash",
        "record_status",
        "trust_level",
        "authority",
        "derived_kind",
        "superseded_by",
        "body_line_offset",
        "body",
        "search_text",
    ];
    let documents_ready = required_document_columns.iter().all(|required_column| {
        document_columns
            .iter()
            .any(|current_column| current_column == required_column)
    });
    let required_fts_columns = ["label", "body", "search_text"];
    let fts_ready = required_fts_columns.iter().all(|required_column| {
        fts_columns
            .iter()
            .any(|current_column| current_column == required_column)
    });

    Ok(!(documents_ready && fts_ready))
}

fn rebuild_workspace_memory_search_storage(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        DROP TABLE IF EXISTS workspace_memory_documents_fts;
        DROP TABLE IF EXISTS workspace_memory_documents;
        ",
    )
    .map_err(|error| format!("drop workspace memory search storage failed: {error}"))?;

    conn.execute_batch(
        "
        CREATE TABLE workspace_memory_documents(
          document_id INTEGER PRIMARY KEY AUTOINCREMENT,
          workspace_root TEXT NOT NULL,
          path TEXT NOT NULL,
          label TEXT NOT NULL,
          document_kind TEXT NOT NULL,
          modified_at_ms INTEGER NOT NULL,
          freshness_ts INTEGER NULL,
          content_hash TEXT NOT NULL,
          record_status TEXT NOT NULL,
          trust_level TEXT NOT NULL,
          authority TEXT NOT NULL,
          derived_kind TEXT NOT NULL,
          superseded_by TEXT NULL,
          body_line_offset INTEGER NOT NULL,
          body TEXT NOT NULL,
          search_text TEXT NOT NULL,
          UNIQUE(workspace_root, path)
        );
        CREATE VIRTUAL TABLE workspace_memory_documents_fts
          USING fts5(
            label,
            body,
            search_text
          );
        ",
    )
    .map_err(|error| format!("recreate workspace memory search storage failed: {error}"))?;

    Ok(())
}

fn workspace_memory_root_key(workspace_root: &Path) -> Result<String, String> {
    let canonical_root = workspace_root.canonicalize().map_err(|error| {
        format!(
            "canonicalize workspace memory root {} failed: {error}",
            workspace_root.display()
        )
    })?;
    Ok(canonical_root.display().to_string())
}

fn workspace_document_modified_at_ms(path: &Path) -> Result<i64, String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "read workspace memory metadata {} failed: {error}",
            path.display()
        )
    })?;
    let modified = metadata.modified().map_err(|error| {
        format!(
            "read workspace memory modified time {} failed: {error}",
            path.display()
        )
    })?;
    let duration_since_epoch = modified.duration_since(UNIX_EPOCH).map_err(|error| {
        format!(
            "read workspace memory modified time {} failed: {error}",
            path.display()
        )
    })?;
    let modified_ms = duration_since_epoch.as_millis();
    i64::try_from(modified_ms).map_err(|error| {
        format!(
            "workspace memory modified time {} exceeds i64 milliseconds: {error}",
            path.display()
        )
    })
}

fn read_workspace_memory_text_lossy(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read memory file {}: {error}", path.display()))?;
    Ok(String::from_utf8_lossy(bytes.as_slice()).into_owned())
}

fn workspace_memory_search_text(
    label: &str,
    body: &str,
    derived_kind: DerivedMemoryKind,
    trust_level: MemoryTrustLevel,
    superseded_by: Option<&str>,
) -> String {
    let mut fragments = vec![label, body, derived_kind.as_str(), trust_level.as_str()];
    if let Some(superseded_by) = superseded_by {
        fragments.push(superseded_by);
    }

    build_search_index_text(fragments.as_slice())
}

fn build_workspace_memory_document_index_entry(
    workspace_root_key: &str,
    location: &WorkspaceMemoryDocumentLocation,
    modified_at_ms: i64,
    parsed_document: ParsedWorkspaceMemoryDocument,
) -> Result<Option<WorkspaceMemoryDocumentIndexEntry>, String> {
    let record_status = parsed_document
        .provenance
        .record_status
        .unwrap_or(MemoryRecordStatus::Active);
    if !record_status.is_active() {
        return Ok(None);
    }

    let derived_kind = parsed_document
        .provenance
        .derived_kind
        .unwrap_or(DerivedMemoryKind::Overview);
    let trust_level = parsed_document
        .provenance
        .trust_level
        .unwrap_or(MemoryTrustLevel::WorkspaceCurated);
    let authority = parsed_document
        .provenance
        .authority
        .unwrap_or(MemoryAuthority::Advisory);
    let content_hash = parsed_document
        .provenance
        .content_hash
        .clone()
        .unwrap_or_default();
    let superseded_by = parsed_document.provenance.superseded_by.clone();
    let search_text = workspace_memory_search_text(
        location.label.as_str(),
        parsed_document.body.as_str(),
        derived_kind,
        trust_level,
        superseded_by.as_deref(),
    );
    let body_line_offset = i64::try_from(parsed_document.body_line_offset).map_err(|error| {
        format!(
            "workspace memory body_line_offset for {} exceeds i64: {error}",
            location.label
        )
    })?;

    Ok(Some(WorkspaceMemoryDocumentIndexEntry {
        workspace_root: workspace_root_key.to_owned(),
        path: location.path.display().to_string(),
        label: location.label.clone(),
        document_kind: location.kind,
        modified_at_ms,
        freshness_ts: parsed_document.provenance.freshness_ts,
        content_hash,
        record_status,
        trust_level,
        authority,
        derived_kind,
        superseded_by,
        body_line_offset,
        body: parsed_document.body,
        search_text,
    }))
}

fn load_workspace_memory_document_index_rows(
    conn: &Connection,
    workspace_root_key: &str,
) -> Result<HashMap<String, WorkspaceMemoryDocumentIndexRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT document_id, path, label, modified_at_ms
             FROM workspace_memory_documents
             WHERE workspace_root = ?1",
        )
        .map_err(|error| format!("prepare workspace memory index row query failed: {error}"))?;
    let rows = stmt
        .query_map(rusqlite::params![workspace_root_key], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| format!("query workspace memory index rows failed: {error}"))?;

    let mut index_rows = HashMap::new();
    for row in rows {
        let (document_id, path, label, modified_at_ms) =
            row.map_err(|error| format!("decode workspace memory index row failed: {error}"))?;
        index_rows.insert(
            path,
            WorkspaceMemoryDocumentIndexRow {
                document_id,
                label,
                modified_at_ms,
            },
        );
    }

    Ok(index_rows)
}

fn upsert_workspace_memory_document_index_entry(
    conn: &Connection,
    existing_document_id: Option<i64>,
    entry: WorkspaceMemoryDocumentIndexEntry,
) -> Result<(), String> {
    let WorkspaceMemoryDocumentIndexEntry {
        workspace_root,
        path,
        label,
        document_kind,
        modified_at_ms,
        freshness_ts,
        content_hash,
        record_status,
        trust_level,
        authority,
        derived_kind,
        superseded_by,
        body_line_offset,
        body,
        search_text,
    } = entry;

    let document_id = if let Some(existing_document_id) = existing_document_id {
        conn.execute(
            "UPDATE workspace_memory_documents
             SET label = ?2,
                 document_kind = ?3,
                 modified_at_ms = ?4,
                 freshness_ts = ?5,
                 content_hash = ?6,
                 record_status = ?7,
                 trust_level = ?8,
                 authority = ?9,
                 derived_kind = ?10,
                 superseded_by = ?11,
                 body_line_offset = ?12,
                 body = ?13,
                 search_text = ?14
             WHERE document_id = ?1",
            rusqlite::params![
                existing_document_id,
                label,
                document_kind.as_str(),
                modified_at_ms,
                freshness_ts,
                content_hash,
                record_status.as_str(),
                trust_level.as_str(),
                authority.as_str(),
                derived_kind.as_str(),
                superseded_by,
                body_line_offset,
                body,
                search_text,
            ],
        )
        .map_err(|error| format!("update workspace memory index row failed: {error}"))?;
        existing_document_id
    } else {
        conn.execute(
            "INSERT INTO workspace_memory_documents(
                workspace_root,
                path,
                label,
                document_kind,
                modified_at_ms,
                freshness_ts,
                content_hash,
                record_status,
                trust_level,
                authority,
                derived_kind,
                superseded_by,
                body_line_offset,
                body,
                search_text
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                workspace_root,
                path,
                label,
                document_kind.as_str(),
                modified_at_ms,
                freshness_ts,
                content_hash,
                record_status.as_str(),
                trust_level.as_str(),
                authority.as_str(),
                derived_kind.as_str(),
                superseded_by,
                body_line_offset,
                body,
                search_text,
            ],
        )
        .map_err(|error| format!("insert workspace memory index row failed: {error}"))?;
        conn.last_insert_rowid()
    };

    conn.execute(
        "DELETE FROM workspace_memory_documents_fts WHERE rowid = ?1",
        rusqlite::params![document_id],
    )
    .map_err(|error| format!("delete stale workspace memory FTS row failed: {error}"))?;
    let (label, body, search_text) = conn
        .query_row(
            "SELECT label, body, search_text
             FROM workspace_memory_documents
             WHERE document_id = ?1",
            rusqlite::params![document_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|error| {
            format!("reload workspace memory index row for FTS sync failed: {error}")
        })?;
    conn.execute(
        "INSERT INTO workspace_memory_documents_fts(rowid, label, body, search_text)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![document_id, label, body, search_text],
    )
    .map_err(|error| format!("insert workspace memory FTS row failed: {error}"))?;

    Ok(())
}

fn delete_workspace_memory_document_index_row(
    conn: &Connection,
    document_id: i64,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM workspace_memory_documents_fts WHERE rowid = ?1",
        rusqlite::params![document_id],
    )
    .map_err(|error| format!("delete workspace memory FTS row failed: {error}"))?;
    conn.execute(
        "DELETE FROM workspace_memory_documents WHERE document_id = ?1",
        rusqlite::params![document_id],
    )
    .map_err(|error| format!("delete workspace memory index row failed: {error}"))?;
    Ok(())
}

fn sync_workspace_memory_search_index(
    workspace_root: &Path,
    memory_system_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<(), String> {
    let workspace_root_key = workspace_memory_root_key(workspace_root)?;
    let locations = collect_workspace_memory_document_locations(workspace_root)?;
    let runtime = acquire_memory_runtime(config)?;

    runtime.with_connection_mut("memory.sync_workspace_memory_search_index", |conn| {
        ensure_workspace_memory_search_storage(conn)?;
        let mut existing_rows =
            load_workspace_memory_document_index_rows(conn, workspace_root_key.as_str())?;

        for location in locations {
            let path_key = location.path.display().to_string();
            let modified_at_ms = workspace_document_modified_at_ms(location.path.as_path())?;
            let existing_row = existing_rows.remove(path_key.as_str());
            let can_reuse_existing = existing_row.as_ref().is_some_and(|row| {
                row.modified_at_ms == modified_at_ms && row.label == location.label
            });
            if can_reuse_existing {
                continue;
            }

            let raw_content = read_workspace_memory_text_lossy(location.path.as_path())?;
            let maybe_parsed_document = parse_workspace_memory_document(
                raw_content.as_str(),
                &location,
                memory_system_id,
                MemoryRecallMode::OperatorInspection,
            )?;
            let Some(parsed_document) = maybe_parsed_document else {
                if let Some(existing_row) = existing_row {
                    delete_workspace_memory_document_index_row(conn, existing_row.document_id)?;
                }
                continue;
            };

            let maybe_entry = build_workspace_memory_document_index_entry(
                workspace_root_key.as_str(),
                &location,
                modified_at_ms,
                parsed_document,
            )?;
            let Some(entry) = maybe_entry else {
                if let Some(existing_row) = existing_row {
                    delete_workspace_memory_document_index_row(conn, existing_row.document_id)?;
                }
                continue;
            };

            upsert_workspace_memory_document_index_entry(
                conn,
                existing_row.as_ref().map(|row| row.document_id),
                entry,
            )?;
        }

        for stale_row in existing_rows.into_values() {
            delete_workspace_memory_document_index_row(conn, stale_row.document_id)?;
        }

        Ok(())
    })
}

pub(crate) fn search_workspace_memory_documents(
    query: &str,
    limit: usize,
    workspace_root: &Path,
    memory_system_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<WorkspaceMemoryIndexedSearchHit>, String> {
    let Some(match_query) = crate::search_text::build_search_fts_query(query, 6) else {
        return Ok(Vec::new());
    };

    sync_workspace_memory_search_index(workspace_root, memory_system_id, config)?;
    let workspace_root_key = workspace_memory_root_key(workspace_root)?;
    let runtime = acquire_memory_runtime(config)?;

    runtime.with_connection("memory.search_workspace_memory_documents", |conn| {
        ensure_workspace_memory_search_storage(conn)?;
        let mut stmt = prepare_cached_sqlite_statement(
            conn,
            "SELECT doc.label,
                    doc.path,
                    doc.document_kind,
                    doc.body,
                    doc.body_line_offset,
                    doc.freshness_ts,
                    doc.content_hash,
                    doc.record_status,
                    doc.trust_level,
                    doc.authority,
                    doc.derived_kind,
                    doc.superseded_by
             FROM workspace_memory_documents_fts AS fts
             JOIN workspace_memory_documents AS doc
               ON doc.document_id = fts.rowid
             WHERE workspace_memory_documents_fts MATCH ?1
               AND doc.workspace_root = ?2
             ORDER BY bm25(workspace_memory_documents_fts),
                      COALESCE(doc.freshness_ts, 0) DESC,
                      doc.label ASC
             LIMIT ?3",
            "prepare workspace memory search statement failed",
        )?;
        let mut rows = stmt
            .query(rusqlite::params![
                match_query,
                workspace_root_key,
                limit.clamp(1, 16) as i64
            ])
            .map_err(|error| format!("query workspace memory search failed: {error}"))?;
        let mut hits = Vec::new();

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("read workspace memory search row failed: {error}"))?
        {
            let label = row
                .get::<_, String>(0)
                .map_err(|error| format!("decode workspace memory label failed: {error}"))?;
            let path = row
                .get::<_, String>(1)
                .map_err(|error| format!("decode workspace memory path failed: {error}"))?;
            let document_kind_text = row.get::<_, String>(2).map_err(|error| {
                format!("decode workspace memory document kind failed: {error}")
            })?;
            let Some(document_kind) =
                WorkspaceMemoryDocumentKind::parse_id(document_kind_text.as_str())
            else {
                continue;
            };
            let body = row
                .get::<_, String>(3)
                .map_err(|error| format!("decode workspace memory body failed: {error}"))?;
            let body_line_offset = row.get::<_, i64>(4).map_err(|error| {
                format!("decode workspace memory body line offset failed: {error}")
            })?;
            let freshness_ts = row
                .get::<_, Option<i64>>(5)
                .map_err(|error| format!("decode workspace memory freshness failed: {error}"))?;
            let content_hash = row
                .get::<_, String>(6)
                .map_err(|error| format!("decode workspace memory content hash failed: {error}"))?;
            let record_status_text = row.get::<_, String>(7).map_err(|error| {
                format!("decode workspace memory record status failed: {error}")
            })?;
            let Some(record_status) = MemoryRecordStatus::parse_id(record_status_text.as_str())
            else {
                continue;
            };
            let trust_level_text = row
                .get::<_, String>(8)
                .map_err(|error| format!("decode workspace memory trust level failed: {error}"))?;
            let Some(trust_level) = MemoryTrustLevel::parse_id(trust_level_text.as_str()) else {
                continue;
            };
            let authority_text = row
                .get::<_, String>(9)
                .map_err(|error| format!("decode workspace memory authority failed: {error}"))?;
            let Some(authority) = MemoryAuthority::parse_id(authority_text.as_str()) else {
                continue;
            };
            let derived_kind_text = row
                .get::<_, String>(10)
                .map_err(|error| format!("decode workspace memory derived kind failed: {error}"))?;
            let Some(derived_kind) = DerivedMemoryKind::parse_id(derived_kind_text.as_str()) else {
                continue;
            };
            let superseded_by = row.get::<_, Option<String>>(11).map_err(|error| {
                format!("decode workspace memory superseded_by failed: {error}")
            })?;
            let body_line_offset = usize::try_from(body_line_offset).map_err(|error| {
                format!("decode workspace memory body line offset failed: {error}")
            })?;

            hits.push(WorkspaceMemoryIndexedSearchHit {
                label,
                path,
                document_kind,
                body,
                body_line_offset,
                freshness_ts,
                content_hash,
                record_status,
                trust_level,
                authority,
                derived_kind,
                superseded_by,
            });
        }

        Ok(hits)
    })
}

fn ensure_approval_lifecycle_tables(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("approval_lifecycle");

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS approval_requests(
          approval_request_id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          turn_id TEXT NOT NULL,
          tool_call_id TEXT NOT NULL,
          tool_name TEXT NOT NULL,
          approval_key TEXT NOT NULL,
          status TEXT NOT NULL,
          decision TEXT NULL,
          request_payload_json TEXT NOT NULL,
          governance_snapshot_json TEXT NOT NULL,
          requested_at INTEGER NOT NULL,
          resolved_at INTEGER NULL,
          resolved_by_session_id TEXT NULL,
          executed_at INTEGER NULL,
          last_error TEXT NULL
        );
        CREATE TABLE IF NOT EXISTS approval_grants(
          scope_session_id TEXT NOT NULL,
          approval_key TEXT NOT NULL,
          created_by_session_id TEXT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(scope_session_id, approval_key)
        );
        CREATE INDEX IF NOT EXISTS idx_approval_requests_session_status_requested_at
          ON approval_requests(session_id, status, requested_at DESC, approval_request_id);
        ",
    )
    .map_err(|error| format!("ensure approval lifecycle storage failed: {error}"))?;

    Ok(())
}

fn ensure_control_plane_pairing_tables(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("control_plane_pairing");

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS control_plane_pairing_requests(
          pairing_request_id TEXT PRIMARY KEY,
          device_id TEXT NOT NULL,
          client_id TEXT NOT NULL,
          public_key TEXT NOT NULL,
          role TEXT NOT NULL,
          requested_scopes_json TEXT NOT NULL,
          status TEXT NOT NULL,
          requested_at_ms INTEGER NOT NULL,
          resolved_at_ms INTEGER NULL,
          issued_token_id TEXT NULL,
          last_error TEXT NULL
        );
        CREATE TABLE IF NOT EXISTS control_plane_device_tokens(
          token_id TEXT PRIMARY KEY,
          device_id TEXT NOT NULL UNIQUE,
          public_key TEXT NOT NULL,
          role TEXT NOT NULL,
          approved_scopes_json TEXT NOT NULL,
          token_hash TEXT NOT NULL,
          issued_at_ms INTEGER NOT NULL,
          expires_at_ms INTEGER NULL,
          revoked_at_ms INTEGER NULL,
          last_used_at_ms INTEGER NULL,
          pairing_request_id TEXT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_control_plane_pairing_requests_status_requested_at
          ON control_plane_pairing_requests(status, requested_at_ms DESC, pairing_request_id);
        CREATE INDEX IF NOT EXISTS idx_control_plane_device_tokens_device_id
          ON control_plane_device_tokens(device_id);
        ",
    )
    .map_err(|error| format!("ensure control-plane pairing storage failed: {error}"))?;

    Ok(())
}

fn ensure_session_tool_consent_storage(conn: &mut Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("session_tool_consent");

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS session_tool_consent(
          scope_session_id TEXT PRIMARY KEY,
          mode TEXT NOT NULL CHECK (mode IN ('prompt', 'auto', 'full')),
          updated_by_session_id TEXT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        ",
    )
    .map_err(|error| format!("ensure session tool consent storage failed: {error}"))?;

    let has_mode_check = sqlite_table_sql_contains(
        conn,
        "session_tool_consent",
        SESSION_TOOL_CONSENT_MODE_CHECK_SQL,
    )?;
    if !has_mode_check {
        rebuild_session_tool_consent_storage_with_mode_check(conn)?;
    }

    Ok(())
}

fn ensure_session_tool_policy_storage(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("session_tool_policy");

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS session_tool_policies(
          session_id TEXT PRIMARY KEY,
          requested_tool_ids_json TEXT NOT NULL,
          runtime_narrowing_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        ",
    )
    .map_err(|error| format!("ensure session tool policy storage failed: {error}"))?;

    Ok(())
}

fn ensure_canonical_record_storage(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("canonical_records");

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS memory_canonical_records(
          record_id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          session_turn_index INTEGER NOT NULL,
          scope TEXT NOT NULL,
          kind TEXT NOT NULL,
          role TEXT NULL,
          content TEXT NOT NULL,
          metadata_json TEXT NOT NULL,
          search_text TEXT NOT NULL DEFAULT '',
          ts INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_canonical_records_session_turn
          ON memory_canonical_records(session_id, session_turn_index);
        CREATE INDEX IF NOT EXISTS idx_memory_canonical_records_scope_kind_ts
          ON memory_canonical_records(scope, kind, ts DESC, record_id);
        CREATE VIRTUAL TABLE IF NOT EXISTS memory_canonical_records_fts
          USING fts5(
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text,
            content='memory_canonical_records',
            content_rowid='record_id'
          );
        CREATE TRIGGER IF NOT EXISTS memory_canonical_records_ai
          AFTER INSERT ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            new.record_id,
            new.content,
            new.session_id,
            new.scope,
            new.kind,
            COALESCE(new.role, ''),
            new.metadata_json,
            new.search_text
          );
        END;
        CREATE TRIGGER IF NOT EXISTS memory_canonical_records_ad
          AFTER DELETE ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(
            memory_canonical_records_fts,
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            'delete',
            old.record_id,
            old.content,
            old.session_id,
            old.scope,
            old.kind,
            COALESCE(old.role, ''),
            old.metadata_json,
            old.search_text
          );
        END;
        CREATE TRIGGER IF NOT EXISTS memory_canonical_records_au
          AFTER UPDATE ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(
            memory_canonical_records_fts,
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            'delete',
            old.record_id,
            old.content,
            old.session_id,
            old.scope,
            old.kind,
            COALESCE(old.role, ''),
            old.metadata_json,
            old.search_text
          );
          INSERT INTO memory_canonical_records_fts(
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            new.record_id,
            new.content,
            new.session_id,
            new.scope,
            new.kind,
            COALESCE(new.role, ''),
            new.metadata_json,
            new.search_text
          );
        END;
        ",
    )
    .map_err(|error| format!("ensure canonical memory storage failed: {error}"))?;

    if !sqlite_table_has_column(conn, "memory_canonical_records", "search_text")? {
        conn.execute(
            "ALTER TABLE memory_canonical_records
             ADD COLUMN search_text TEXT NOT NULL DEFAULT ''",
            [],
        )
        .map_err(|error| format!("add canonical memory search_text column failed: {error}"))?;
    }

    backfill_canonical_record_search_text(conn)?;

    let needs_canonical_fts_rebuild = canonical_record_fts_needs_rebuild(conn)?;
    if needs_canonical_fts_rebuild {
        rebuild_canonical_record_storage(conn)?;
        return Ok(());
    }

    rebuild_canonical_record_storage_if_needed(conn)?;

    Ok(())
}

fn backfill_canonical_record_search_text(conn: &Connection) -> Result<(), String> {
    let mut select_stmt = conn
        .prepare(
            "SELECT record_id, session_id, scope, kind, role, content, metadata_json
             FROM memory_canonical_records
             WHERE search_text = '' OR search_text IS NULL",
        )
        .map_err(|error| format!("prepare canonical search_text backfill failed: {error}"))?;
    let rows = select_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| format!("query canonical search_text backfill failed: {error}"))?;

    let mut pending_updates = Vec::new();
    for row in rows {
        let (record_id, session_id, scope, kind, role, content, metadata_json) =
            row.map_err(|error| {
                format!("decode canonical search_text backfill row failed: {error}")
            })?;
        let search_text = canonical_record_search_text(
            session_id.as_str(),
            scope.as_str(),
            kind.as_str(),
            role.as_deref(),
            content.as_str(),
            metadata_json.as_str(),
        );
        pending_updates.push((record_id, search_text));
    }
    drop(select_stmt);

    let mut update_stmt = conn
        .prepare(
            "UPDATE memory_canonical_records
             SET search_text = ?2
             WHERE record_id = ?1",
        )
        .map_err(|error| format!("prepare canonical search_text update failed: {error}"))?;
    for (record_id, search_text) in pending_updates {
        update_stmt
            .execute(rusqlite::params![record_id, search_text])
            .map_err(|error| format!("update canonical search_text failed: {error}"))?;
    }

    Ok(())
}

fn canonical_record_fts_needs_rebuild(conn: &Connection) -> Result<bool, String> {
    let columns = sqlite_table_columns(conn, "memory_canonical_records_fts")?;
    if columns.is_empty() {
        return Ok(false);
    }

    let required_columns = [
        "content",
        "session_id",
        "scope",
        "kind",
        "role",
        "metadata_json",
        "search_text",
    ];
    let has_all_required_columns = required_columns.iter().all(|required_column| {
        columns
            .iter()
            .any(|current_column| current_column == required_column)
    });

    Ok(!has_all_required_columns)
}

fn drop_canonical_record_fts_index(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        DROP TRIGGER IF EXISTS memory_canonical_records_ai;
        DROP TRIGGER IF EXISTS memory_canonical_records_ad;
        DROP TRIGGER IF EXISTS memory_canonical_records_au;
        DROP TABLE IF EXISTS memory_canonical_records_fts;
        ",
    )
    .map_err(|error| format!("drop canonical memory FTS index failed: {error}"))?;

    Ok(())
}

fn create_canonical_record_fts_index(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE memory_canonical_records_fts
          USING fts5(
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text,
            content='memory_canonical_records',
            content_rowid='record_id'
          );
        CREATE TRIGGER memory_canonical_records_ai
          AFTER INSERT ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            new.record_id,
            new.content,
            new.session_id,
            new.scope,
            new.kind,
            COALESCE(new.role, ''),
            new.metadata_json,
            new.search_text
          );
        END;
        CREATE TRIGGER memory_canonical_records_ad
          AFTER DELETE ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(
            memory_canonical_records_fts,
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            'delete',
            old.record_id,
            old.content,
            old.session_id,
            old.scope,
            old.kind,
            COALESCE(old.role, ''),
            old.metadata_json,
            old.search_text
          );
        END;
        CREATE TRIGGER memory_canonical_records_au
          AFTER UPDATE ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(
            memory_canonical_records_fts,
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            'delete',
            old.record_id,
            old.content,
            old.session_id,
            old.scope,
            old.kind,
            COALESCE(old.role, ''),
            old.metadata_json,
            old.search_text
          );
          INSERT INTO memory_canonical_records_fts(
            rowid,
            content,
            session_id,
            scope,
            kind,
            role,
            metadata_json,
            search_text
          )
          VALUES (
            new.record_id,
            new.content,
            new.session_id,
            new.scope,
            new.kind,
            COALESCE(new.role, ''),
            new.metadata_json,
            new.search_text
          );
        END;
        ",
    )
    .map_err(|error| format!("recreate canonical memory FTS index failed: {error}"))?;

    Ok(())
}

fn rebuild_canonical_record_fts_index_contents(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "
        INSERT INTO memory_canonical_records_fts(
          rowid,
          content,
          session_id,
          scope,
          kind,
          role,
          metadata_json,
          search_text
        )
        SELECT
          record_id,
          content,
          session_id,
          scope,
          kind,
          COALESCE(role, ''),
          metadata_json,
          search_text
        FROM memory_canonical_records
        ",
        [],
    )
    .map(|_| ())
    .map_err(|error| format!("rebuild canonical memory FTS index contents failed: {error}"))?;

    Ok(())
}

fn ensure_summary_checkpoint_storage_layout(conn: &Connection) -> Result<(), String> {
    #[cfg(test)]
    test_support::record_sqlite_schema_repair("summary_checkpoint_metadata");

    if sqlite_table_columns(conn, "memory_summary_checkpoints")?.is_empty() {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memory_summary_checkpoints(
              session_id TEXT PRIMARY KEY,
              summarized_through_turn_id INTEGER NOT NULL,
              summary_before_turn_id INTEGER,
              summary_body_bytes INTEGER NOT NULL DEFAULT 0,
              summary_budget_chars INTEGER NOT NULL,
              summary_window_size INTEGER NOT NULL,
              summary_format_version INTEGER NOT NULL,
              updated_at_ts INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_summary_checkpoint_bodies(
              session_id TEXT PRIMARY KEY
                REFERENCES memory_summary_checkpoints(session_id) ON DELETE CASCADE,
              summary_body TEXT NOT NULL
            );
            ",
        )
        .map_err(|error| format!("create summary checkpoint storage tables failed: {error}"))?;
        return Ok(());
    }

    if sqlite_table_has_column(conn, "memory_summary_checkpoints", "summary_body")? {
        ensure_legacy_summary_checkpoint_metadata_columns(conn)?;
        split_summary_checkpoint_body_storage(conn)?;
        return Ok(());
    }

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS memory_summary_checkpoint_bodies(
          session_id TEXT PRIMARY KEY
            REFERENCES memory_summary_checkpoints(session_id) ON DELETE CASCADE,
          summary_body TEXT NOT NULL
        );
        ",
    )
    .map_err(|error| format!("ensure summary checkpoint body table failed: {error}"))?;

    Ok(())
}

fn ensure_legacy_summary_checkpoint_metadata_columns(conn: &Connection) -> Result<(), String> {
    if !sqlite_table_has_column(conn, "memory_summary_checkpoints", "summary_body_bytes")? {
        conn.execute(
            "ALTER TABLE memory_summary_checkpoints
             ADD COLUMN summary_body_bytes INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|error| format!("add summary checkpoint body bytes column failed: {error}"))?;
    }

    conn.execute(
        "UPDATE memory_summary_checkpoints
         SET summary_body_bytes = LENGTH(CAST(summary_body AS BLOB))
         WHERE summary_body_bytes <= 0
           AND summary_body <> ''",
        [],
    )
    .map_err(|error| format!("backfill summary checkpoint body bytes failed: {error}"))?;

    if !sqlite_table_has_column(conn, "memory_summary_checkpoints", "summary_before_turn_id")? {
        conn.execute(
            "ALTER TABLE memory_summary_checkpoints
             ADD COLUMN summary_before_turn_id INTEGER",
            [],
        )
        .map_err(|error| {
            format!("add summary checkpoint boundary turn id column failed: {error}")
        })?;
    }

    conn.execute(
        "UPDATE memory_summary_checkpoints
         SET summary_before_turn_id = (
             SELECT id
             FROM turns
             WHERE session_id = memory_summary_checkpoints.session_id
               AND id > memory_summary_checkpoints.summarized_through_turn_id
             ORDER BY id ASC
             LIMIT 1
         )
         WHERE summary_before_turn_id IS NULL
            OR summary_before_turn_id <= 0",
        [],
    )
    .map_err(|error| format!("backfill summary checkpoint boundary turn id failed: {error}"))?;

    Ok(())
}

fn split_summary_checkpoint_body_storage(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        BEGIN IMMEDIATE;
        DROP TABLE IF EXISTS memory_summary_checkpoint_bodies;
        ALTER TABLE memory_summary_checkpoints
          RENAME TO memory_summary_checkpoints_legacy;
        CREATE TABLE memory_summary_checkpoints(
          session_id TEXT PRIMARY KEY,
          summarized_through_turn_id INTEGER NOT NULL,
          summary_before_turn_id INTEGER,
          summary_body_bytes INTEGER NOT NULL DEFAULT 0,
          summary_budget_chars INTEGER NOT NULL,
          summary_window_size INTEGER NOT NULL,
          summary_format_version INTEGER NOT NULL,
          updated_at_ts INTEGER NOT NULL
        );
        CREATE TABLE memory_summary_checkpoint_bodies(
          session_id TEXT PRIMARY KEY
            REFERENCES memory_summary_checkpoints(session_id) ON DELETE CASCADE,
          summary_body TEXT NOT NULL
        );
        INSERT INTO memory_summary_checkpoints(
          session_id,
          summarized_through_turn_id,
          summary_before_turn_id,
          summary_body_bytes,
          summary_budget_chars,
          summary_window_size,
          summary_format_version,
          updated_at_ts
        )
        SELECT session_id,
               summarized_through_turn_id,
               summary_before_turn_id,
               summary_body_bytes,
               summary_budget_chars,
               summary_window_size,
               summary_format_version,
               updated_at_ts
        FROM memory_summary_checkpoints_legacy;
        INSERT INTO memory_summary_checkpoint_bodies(
          session_id,
          summary_body
        )
        SELECT session_id,
               summary_body
        FROM memory_summary_checkpoints_legacy;
        DROP TABLE memory_summary_checkpoints_legacy;
        COMMIT;
        ",
    )
    .map_err(|error| {
        let _ = conn.execute_batch("ROLLBACK;");
        format!("split summary checkpoint body storage failed: {error}")
    })
}

fn sqlite_table_has_column(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, String> {
    Ok(sqlite_table_columns(conn, table_name)?
        .iter()
        .any(|current_name| current_name == column_name))
}

fn sqlite_table_sql_contains(
    conn: &Connection,
    table_name: &str,
    needle: &str,
) -> Result<bool, String> {
    let sql = conn
        .query_row(
            "SELECT sql
             FROM sqlite_master
             WHERE type = 'table' AND name = ?1",
            [table_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("query sqlite table sql failed: {error}"))?;

    Ok(sql.is_some_and(|value| value.contains(needle)))
}

fn rebuild_session_tool_consent_storage_with_mode_check(
    conn: &mut Connection,
) -> Result<(), String> {
    let invalid_mode_rows = conn
        .query_row(
            "SELECT COUNT(*)
             FROM session_tool_consent
             WHERE mode NOT IN ('prompt', 'auto', 'full')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("validate session tool consent modes failed: {error}"))?;
    if invalid_mode_rows > 0 {
        return Err(format!(
            "session_tool_consent contains {invalid_mode_rows} invalid mode rows"
        ));
    }

    let tx = conn.transaction().map_err(|error| {
        format!("open session tool consent rebuild transaction failed: {error}")
    })?;
    tx.execute_batch(
        "
        ALTER TABLE session_tool_consent RENAME TO session_tool_consent_legacy;
        CREATE TABLE session_tool_consent(
          scope_session_id TEXT PRIMARY KEY,
          mode TEXT NOT NULL CHECK (mode IN ('prompt', 'auto', 'full')),
          updated_by_session_id TEXT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        INSERT INTO session_tool_consent(
          scope_session_id,
          mode,
          updated_by_session_id,
          created_at,
          updated_at
        )
        SELECT
          scope_session_id,
          mode,
          updated_by_session_id,
          created_at,
          updated_at
        FROM session_tool_consent_legacy;
        DROP TABLE session_tool_consent_legacy;
        ",
    )
    .map_err(|error| format!("rebuild session tool consent storage failed: {error}"))?;
    tx.commit()
        .map_err(|error| format!("commit session tool consent rebuild failed: {error}"))?;

    Ok(())
}

fn sqlite_table_columns(conn: &Connection, table_name: &str) -> Result<Vec<String>, String> {
    let pragma = format!("PRAGMA table_info({table_name})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|error| format!("prepare sqlite table info query failed: {error}"))?;
    let mut rows = stmt
        .query([])
        .map_err(|error| format!("query sqlite table info failed: {error}"))?;
    let mut columns = Vec::new();

    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read sqlite table info row failed: {error}"))?
    {
        columns.push(
            row.get::<_, String>(1)
                .map_err(|error| format!("decode sqlite table info column failed: {error}"))?,
        );
    }

    Ok(columns)
}

#[cfg(test)]
fn sqlite_bootstrap_count_for_tests(path: &Path) -> usize {
    test_support::sqlite_bootstrap_count(path)
}

#[cfg(test)]
fn sqlite_bootstrap_count_under_prefix_for_tests(path: &Path) -> usize {
    test_support::sqlite_bootstrap_count_under_prefix(path)
}

#[cfg(test)]
fn sqlite_schema_repair_count_for_tests(kind: &'static str) -> usize {
    test_support::sqlite_schema_repair_count(kind)
}

#[cfg(test)]
fn sqlite_schema_init_count_for_tests(path: &Path) -> usize {
    test_support::sqlite_schema_init_count(path)
}

#[cfg(test)]
fn runtime_path_normalization_full_count_for_tests() -> usize {
    test_support::runtime_path_normalization_full_count()
}

#[cfg(test)]
fn runtime_path_normalization_alias_hit_count_for_tests() -> usize {
    test_support::runtime_path_normalization_alias_hit_count()
}

#[cfg(test)]
fn reset_cached_prepare_metrics_for_tests() {
    test_support::reset_cached_prepare_metrics();
}

#[cfg(test)]
fn reset_sqlite_schema_repair_metrics_for_tests() {
    test_support::reset_sqlite_schema_repair_metrics();
}

#[cfg(test)]
struct SqliteMetricCaptureGuard;

#[cfg(test)]
impl Drop for SqliteMetricCaptureGuard {
    fn drop(&mut self) {
        test_support::end_sqlite_metric_capture();
    }
}

#[cfg(test)]
fn begin_sqlite_metric_capture_for_tests() -> SqliteMetricCaptureGuard {
    test_support::begin_sqlite_metric_capture();
    SqliteMetricCaptureGuard
}

#[cfg(test)]
fn cached_prepare_count_for_sql_fragment_for_tests(fragment: &str) -> usize {
    test_support::cached_prepare_count_for_sql_fragment(fragment)
}

#[cfg(test)]
fn reset_summary_materialization_metrics_for_tests() {
    test_support::reset_summary_materialization_metrics();
}

#[cfg(test)]
fn summary_buffered_query_count_for_tests(kind: &'static str) -> usize {
    test_support::summary_buffered_query_count(kind)
}

#[cfg(test)]
fn summary_streaming_query_count_for_tests(kind: &'static str) -> usize {
    test_support::summary_streaming_query_count(kind)
}

#[cfg(test)]
fn summary_payload_decode_count_for_tests() -> usize {
    test_support::summary_payload_decode_count()
}

#[cfg(test)]
fn summary_row_observed_count_for_tests() -> usize {
    test_support::summary_row_observed_count()
}

#[cfg(test)]
fn summary_frontier_probe_count_for_tests(kind: &'static str) -> usize {
    test_support::summary_frontier_probe_count(kind)
}

#[cfg(test)]
fn summary_normalization_count_for_tests() -> usize {
    test_support::summary_normalization_count()
}

#[cfg(test)]
fn configure_sqlite_runtime_cache_miss_for_tests(path: &Path, target_waiters: usize) {
    test_support::configure_sqlite_runtime_cache_miss(path, target_waiters);
}

#[cfg(test)]
fn clear_sqlite_runtime_cache_miss_for_tests() {
    test_support::clear_sqlite_runtime_cache_miss();
}

#[cfg(test)]
fn reset_sqlite_runtime_test_state() {
    if let Ok(mut registry) = sqlite_runtime_registry().lock() {
        registry.clear();
    }
    if let Ok(mut bootstrap_registry) = sqlite_runtime_bootstrap_lock_registry().lock() {
        bootstrap_registry.clear();
    }
    if let Ok(mut alias_registry) = sqlite_runtime_path_alias_registry().lock() {
        alias_registry.clear();
    }
    test_support::reset_test_state();
}

#[cfg(test)]
fn sqlite_runtime_test_lock() -> &'static Mutex<()> {
    static SQLITE_RUNTIME_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    SQLITE_RUNTIME_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) fn drop_cached_sqlite_runtime(path: &Path) -> Result<bool, String> {
    let normalized_path = normalize_runtime_db_path(path)?;
    let mut registry = sqlite_runtime_registry()
        .lock()
        .map_err(|poisoned| format!("lock sqlite runtime registry failed: {poisoned}"))?;
    let removed = registry.remove(&normalized_path).is_some();
    if removed && let Ok(mut alias_registry) = sqlite_runtime_path_alias_registry().lock() {
        alias_registry.retain(|_key, value| *value != normalized_path);
    }
    if removed && let Ok(mut bootstrap_registry) = sqlite_runtime_bootstrap_lock_registry().lock() {
        bootstrap_registry.remove(&normalized_path);
    }
    Ok(removed)
}

#[cfg(test)]
fn drop_cached_sqlite_runtime_for_tests(path: &Path) {
    let _ = drop_cached_sqlite_runtime(path);
}

fn query_recent_turns(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<(Vec<ConversationTurn>, Option<usize>), String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_RECENT_TURNS_NO_ID,
        "prepare memory window query failed",
    )?;
    let mut rows = stmt
        .query(rusqlite::params![session_id, limit as i64])
        .map_err(|error| format!("query memory window failed: {error}"))?;
    let mut turns = Vec::with_capacity(limit);
    let mut turn_count = None;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read memory window row failed: {error}"))?
    {
        if turn_count.is_none() {
            turn_count = row
                .get::<_, Option<i64>>(3)
                .map_err(|error| format!("decode memory window turn count failed: {error}"))?
                .map(|value| value.max(0) as usize);
        }
        turns.push(ConversationTurn {
            role: row
                .get(0)
                .map_err(|error| format!("decode memory window role failed: {error}"))?,
            content: row
                .get(1)
                .map_err(|error| format!("decode memory window content failed: {error}"))?,
            ts: row
                .get(2)
                .map_err(|error| format!("decode memory window timestamp failed: {error}"))?,
        });
    }
    turns.reverse();
    Ok((turns, turn_count))
}

#[cfg(test)]
fn query_recent_turns_with_boundary_id(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<RecentWindowTurns, String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_RECENT_TURNS_WITH_BOUNDARY_ID,
        "prepare memory window query failed",
    )?;
    let mut rows = stmt
        .query(rusqlite::params![session_id, limit as i64])
        .map_err(|error| format!("query memory window failed: {error}"))?;
    let mut turns = Vec::with_capacity(limit);
    let mut summary_before_turn_id = None;
    let mut oldest_session_turn_index = None;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read memory window row failed: {error}"))?
    {
        summary_before_turn_id = Some(
            row.get::<_, i64>(3)
                .map_err(|error| format!("decode memory window boundary id failed: {error}"))?,
        );
        oldest_session_turn_index = row
            .get::<_, Option<i64>>(4)
            .map_err(|error| format!("decode memory window session turn index failed: {error}"))?;
        turns.push(ConversationTurn {
            role: row
                .get(0)
                .map_err(|error| format!("decode memory window role failed: {error}"))?,
            content: row
                .get(1)
                .map_err(|error| format!("decode memory window content failed: {error}"))?,
            ts: row
                .get(2)
                .map_err(|error| format!("decode memory window timestamp failed: {error}"))?,
        });
    }
    turns.reverse();
    Ok(RecentWindowTurns {
        turns,
        summary_before_turn_id,
        window_starts_at_session_origin: summary_before_turn_id.is_none()
            || oldest_session_turn_index == Some(1),
    })
}

fn query_recent_prompt_turns(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<Vec<PromptWindowTurn>, String> {
    let mut recent_rows = query_recent_prompt_turn_rows_with_ids(conn, session_id, limit)?;

    recent_rows.reverse();
    let turns = recent_rows
        .into_iter()
        .map(|(_, turn)| turn)
        .collect::<Vec<_>>();

    Ok(turns)
}

fn query_recent_prompt_turns_with_overflow_probe(
    conn: &Connection,
    session_id: &str,
    limit: usize,
    mut diagnostics: Option<&mut PromptWindowQueryDiagnostics>,
) -> Result<RecentPromptWindowTurns, String> {
    let turn_count_started_at = StdInstant::now();
    let session_turn_count = query_session_turn_count(conn, session_id)?;
    if let Some(diagnostics) = diagnostics.as_deref_mut() {
        diagnostics.turn_count_query_ms = elapsed_ms(turn_count_started_at);
    }

    match session_turn_count {
        Some(session_turn_count) if session_turn_count <= limit as i64 => {
            let exact_query_started_at = StdInstant::now();
            let turns = query_recent_prompt_turns(conn, session_id, limit)?;
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.exact_rows_query_ms = elapsed_ms(exact_query_started_at);
            }
            if !prompt_window_rows_match_session_turn_count(
                Some(session_turn_count),
                turns.len(),
                limit,
                false,
            ) {
                return query_recent_prompt_turns_with_payload_overflow_probe_fallback(
                    conn,
                    session_id,
                    limit,
                    diagnostics,
                );
            }

            Ok(RecentPromptWindowTurns {
                turns,
                summary_before_turn_id: None,
                window_starts_at_session_origin: true,
                checkpoint_meta_lookup: SummaryCheckpointMetaLookup::Known(None),
            })
        }
        Some(session_turn_count) => {
            let known_overflow_rows_started_at = StdInstant::now();
            let recent_window =
                query_recent_prompt_turns_with_known_overflow(conn, session_id, limit)?;
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.known_overflow_rows_query_ms =
                    elapsed_ms(known_overflow_rows_started_at);
            }
            if !prompt_window_rows_match_session_turn_count(
                Some(session_turn_count),
                recent_window.turns.len(),
                limit,
                false,
            ) {
                return query_recent_prompt_turns_with_payload_overflow_probe_fallback(
                    conn,
                    session_id,
                    limit,
                    diagnostics,
                );
            }
            Ok(recent_window)
        }
        None => query_recent_prompt_turns_with_payload_overflow_probe_fallback(
            conn,
            session_id,
            limit,
            diagnostics,
        ),
    }
}

fn prompt_window_rows_match_session_turn_count(
    session_turn_count: Option<i64>,
    row_count: usize,
    limit: usize,
    inconsistent_session_turn_count: bool,
) -> bool {
    if inconsistent_session_turn_count {
        return false;
    }

    let Some(session_turn_count) = session_turn_count else {
        return row_count < limit;
    };
    if session_turn_count < 0 {
        return false;
    }

    let session_turn_count = session_turn_count as usize;
    (session_turn_count <= limit && row_count == session_turn_count)
        || (session_turn_count > limit && row_count == limit)
}

fn query_session_turn_count(conn: &Connection, session_id: &str) -> Result<Option<i64>, String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_SELECT_SESSION_TURN_COUNT,
        "prepare session turn-count query failed",
    )?;
    stmt.query_row(rusqlite::params![session_id], |row| row.get::<_, i64>(0))
        .map(Some)
        .or_else(|error| {
            if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                Ok(None)
            } else {
                Err(error)
            }
        })
        .map_err(|error| format!("query session turn count failed: {error}"))
}

fn resolve_actual_turn_count(conn: &Connection, session_id: &str) -> Result<i64, String> {
    if let Some(turn_count) = query_session_turn_count(conn, session_id)? {
        return Ok(turn_count.max(0));
    }

    conn.query_row(
        "SELECT COALESCE(MAX(session_turn_index), 0)
         FROM turns
         WHERE session_id = ?1",
        rusqlite::params![session_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|turn_count| turn_count.max(0))
    .map_err(|error| format!("query fallback session turn count failed: {error}"))
}

fn query_recent_prompt_turns_with_known_overflow(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<RecentPromptWindowTurns, String> {
    let fetch_limit = limit.saturating_add(1);
    let (mut recent_rows, checkpoint_meta) =
        query_recent_prompt_turn_rows_with_ids_and_checkpoint_meta(conn, session_id, fetch_limit)?;
    recent_rows.reverse();
    let has_visible_overflow = recent_rows.len() > limit;
    let summary_before_turn_id = if has_visible_overflow {
        recent_rows.get(1).map(|(turn_id, _)| *turn_id)
    } else {
        None
    };
    let turns = recent_rows
        .into_iter()
        .skip(usize::from(has_visible_overflow))
        .map(|(_, turn)| turn)
        .collect();
    Ok(RecentPromptWindowTurns {
        turns,
        summary_before_turn_id,
        window_starts_at_session_origin: !has_visible_overflow,
        checkpoint_meta_lookup: SummaryCheckpointMetaLookup::Known(checkpoint_meta),
    })
}

fn query_recent_prompt_turns_with_payload_overflow_probe_fallback(
    conn: &Connection,
    session_id: &str,
    limit: usize,
    diagnostics: Option<&mut PromptWindowQueryDiagnostics>,
) -> Result<RecentPromptWindowTurns, String> {
    let fetch_limit = limit.saturating_add(1);
    let fallback_query_started_at = StdInstant::now();
    let mut recent_rows = query_recent_prompt_turn_rows_with_ids(conn, session_id, fetch_limit)?;
    if let Some(diagnostics) = diagnostics {
        diagnostics.fallback_rows_query_ms = elapsed_ms(fallback_query_started_at);
    }
    recent_rows.reverse();
    let has_overflow = recent_rows.len() > limit;
    let summary_before_turn_id = if has_overflow {
        recent_rows.get(1).map(|(turn_id, _)| *turn_id)
    } else {
        None
    };
    let turns = recent_rows
        .into_iter()
        .skip(usize::from(has_overflow))
        .map(|(_, turn)| turn)
        .collect();
    Ok(RecentPromptWindowTurns {
        turns,
        summary_before_turn_id,
        window_starts_at_session_origin: !has_overflow,
        checkpoint_meta_lookup: SummaryCheckpointMetaLookup::Unknown,
    })
}

fn query_recent_prompt_turn_rows_with_ids(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<Vec<(i64, PromptWindowTurn)>, String> {
    let mut request_limit = limit.max(1);

    loop {
        let mut stmt = prepare_cached_sqlite_statement(
            conn,
            SQL_QUERY_RECENT_PROMPT_TURNS_WITH_OVERFLOW_PROBE_FALLBACK,
            "prepare prompt window id query failed",
        )?;
        let mut rows = stmt
            .query(rusqlite::params![session_id, request_limit as i64])
            .map_err(|error| format!("query prompt window id rows failed: {error}"))?;
        let mut recent_rows = Vec::with_capacity(limit);
        let mut raw_row_count = 0usize;

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("read prompt window id row failed: {error}"))?
        {
            raw_row_count = raw_row_count.saturating_add(1);
            let turn_id = row
                .get::<_, i64>(0)
                .map_err(|error| format!("decode prompt window turn id failed: {error}"))?;
            let role = row
                .get::<_, String>(1)
                .map_err(|error| format!("decode prompt window role failed: {error}"))?;
            let content = row
                .get::<_, String>(2)
                .map_err(|error| format!("decode prompt window content failed: {error}"))?;
            let include_turn =
                prompt_window_turn_is_visible(session_id, role.as_str(), content.as_str());

            if !include_turn {
                continue;
            }

            recent_rows.push((turn_id, PromptWindowTurn { role, content }));
            if recent_rows.len() >= limit {
                break;
            }
        }

        let reached_visible_limit = recent_rows.len() >= limit;
        let exhausted_rows = raw_row_count < request_limit;
        if reached_visible_limit || exhausted_rows {
            return Ok(recent_rows);
        }

        request_limit = request_limit.saturating_mul(2);
    }
}

fn query_recent_prompt_turn_rows_with_ids_and_checkpoint_meta(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<(Vec<(i64, PromptWindowTurn)>, Option<SummaryCheckpointMeta>), String> {
    let mut request_limit = limit.max(1);

    loop {
        let mut stmt = prepare_cached_sqlite_statement(
            conn,
            SQL_QUERY_RECENT_PROMPT_TURNS_WITH_CHECKPOINT_META,
            "prepare prompt window checkpoint query failed",
        )?;
        let mut rows = stmt
            .query(rusqlite::params![session_id, request_limit as i64])
            .map_err(|error| format!("query prompt window checkpoint rows failed: {error}"))?;
        let mut recent_rows = Vec::with_capacity(limit);
        let mut checkpoint_meta = None;
        let mut raw_row_count = 0usize;

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("read prompt window checkpoint row failed: {error}"))?
        {
            raw_row_count = raw_row_count.saturating_add(1);
            if checkpoint_meta.is_none() {
                let summarized_through_turn_id = row.get::<_, Option<i64>>(3).map_err(|error| {
                    format!(
                        "decode summary checkpoint frontier from prompt window row failed: {error}"
                    )
                })?;
                if let Some(summarized_through_turn_id) = summarized_through_turn_id {
                    checkpoint_meta = Some(SummaryCheckpointMeta {
                        summarized_through_turn_id,
                        summary_before_turn_id: row.get::<_, Option<i64>>(4).map_err(|error| {
                            format!(
                                "decode summary checkpoint boundary from prompt window row failed: {error}"
                            )
                        })?,
                        summary_body_len: row
                            .get::<_, Option<i64>>(5)
                            .map_err(|error| {
                                format!(
                                    "decode summary checkpoint body length from prompt window row failed: {error}"
                                )
                            })?
                            .unwrap_or_default()
                            .max(0) as usize,
                        summary_budget_chars: row
                            .get::<_, Option<i64>>(6)
                            .map_err(|error| {
                                format!(
                                    "decode summary checkpoint budget from prompt window row failed: {error}"
                                )
                            })?
                            .unwrap_or_default()
                            .max(0) as usize,
                        summary_window_size: row
                            .get::<_, Option<i64>>(7)
                            .map_err(|error| {
                                format!(
                                    "decode summary checkpoint window from prompt window row failed: {error}"
                                )
                            })?
                            .unwrap_or_default()
                            .max(0) as usize,
                        summary_format_version: row
                            .get::<_, Option<i64>>(8)
                            .map_err(|error| {
                                format!(
                                    "decode summary checkpoint version from prompt window row failed: {error}"
                                )
                            })?
                            .unwrap_or_default(),
                    });
                }
            }

            let turn_id = row
                .get::<_, i64>(0)
                .map_err(|error| format!("decode prompt window turn id failed: {error}"))?;
            let role = row
                .get::<_, String>(1)
                .map_err(|error| format!("decode prompt window role failed: {error}"))?;
            let content = row
                .get::<_, String>(2)
                .map_err(|error| format!("decode prompt window content failed: {error}"))?;
            let include_turn =
                prompt_window_turn_is_visible(session_id, role.as_str(), content.as_str());

            if !include_turn {
                continue;
            }

            recent_rows.push((turn_id, PromptWindowTurn { role, content }));
            if recent_rows.len() >= limit {
                break;
            }
        }

        let reached_visible_limit = recent_rows.len() >= limit;
        let exhausted_rows = raw_row_count < request_limit;
        if reached_visible_limit || exhausted_rows {
            return Ok((recent_rows, checkpoint_meta));
        }

        request_limit = request_limit.saturating_mul(2);
    }
}

fn prompt_window_turn_is_visible(session_id: &str, role: &str, content: &str) -> bool {
    let canonical_record = canonical_memory_record_from_persisted_turn(session_id, role, content);
    let canonical_kind = canonical_record.kind;

    matches!(
        canonical_kind,
        CanonicalMemoryKind::UserTurn | CanonicalMemoryKind::AssistantTurn
    )
}

struct SummaryStreamProgress {
    latest_turn_id: Option<i64>,
    saturated: bool,
}

fn stream_summary_rows_until_saturation(
    rows: &mut rusqlite::Rows<'_>,
    row_error_context: &'static str,
    summary_body: &mut String,
    summary_budget_chars: usize,
) -> Result<SummaryStreamProgress, String> {
    reserve_summary_body_capacity(summary_body, summary_budget_chars);
    let mut latest_turn_id = None;
    let mut saturated = false;

    while let Some(row) = rows
        .next()
        .map_err(|error| format!("{row_error_context}: {error}"))?
    {
        #[cfg(test)]
        test_support::record_summary_row_observed();
        let turn_id = row
            .get_ref(0)
            .map_err(|error| format!("decode summary turn id failed: {error}"))?
            .as_i64()
            .map_err(|error| format!("decode summary turn id failed: {error}"))?;
        latest_turn_id = Some(turn_id);
        if summary_body.len() >= summary_budget_chars {
            saturated = true;
            break;
        }

        #[cfg(test)]
        test_support::record_summary_payload_decode();
        let role = row
            .get_ref(1)
            .map_err(|error| format!("decode summary turn role failed: {error}"))?
            .as_str()
            .map_err(|error| format!("decode summary turn role failed: {error}"))?;
        let content = row
            .get_ref(2)
            .map_err(|error| format!("decode summary turn content failed: {error}"))?
            .as_str()
            .map_err(|error| format!("decode summary turn content failed: {error}"))?;

        append_summary_line(summary_body, role, content, summary_budget_chars);
        if summary_body.len() >= summary_budget_chars {
            saturated = true;
            break;
        }
    }

    Ok(SummaryStreamProgress {
        latest_turn_id,
        saturated,
    })
}

fn query_summary_frontier_turn_id_up_to_id(
    conn: &Connection,
    session_id: &str,
    through_turn_id: i64,
) -> Result<Option<i64>, String> {
    #[cfg(test)]
    test_support::record_summary_frontier_probe("rebuild");

    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_SUMMARY_FRONTIER_UP_TO_ID,
        "prepare summary rebuild frontier query failed",
    )?;
    stmt.query_row(rusqlite::params![session_id, through_turn_id], |row| {
        row.get::<_, i64>(0)
    })
    .map(Some)
    .or_else(|error| {
        if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
            Ok(None)
        } else {
            Err(error)
        }
    })
    .map_err(|error| format!("query summary rebuild frontier failed: {error}"))
}

fn query_summary_frontier_turn_id_between_ids(
    conn: &Connection,
    session_id: &str,
    after_turn_id: i64,
    before_turn_id: i64,
) -> Result<Option<i64>, String> {
    #[cfg(test)]
    test_support::record_summary_frontier_probe("catch_up");

    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_SUMMARY_FRONTIER_BETWEEN_IDS,
        "prepare summary catch-up frontier query failed",
    )?;
    stmt.query_row(
        rusqlite::params![session_id, after_turn_id, before_turn_id],
        |row| row.get::<_, i64>(0),
    )
    .map(Some)
    .or_else(|error| {
        if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
            Ok(None)
        } else {
            Err(error)
        }
    })
    .map_err(|error| format!("query summary catch-up frontier failed: {error}"))
}

fn stream_summary_turns_up_to_id(
    conn: &Connection,
    session_id: &str,
    through_turn_id: i64,
    summary_body: &mut String,
    summary_budget_chars: usize,
) -> Result<Option<i64>, String> {
    #[cfg(test)]
    test_support::record_summary_streaming_query("rebuild");

    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_TURNS_UP_TO_ID,
        "prepare summary rebuild query failed",
    )?;
    let progress = {
        let mut rows = stmt
            .query(rusqlite::params![session_id, through_turn_id])
            .map_err(|error| format!("query summary rebuild turns failed: {error}"))?;
        stream_summary_rows_until_saturation(
            &mut rows,
            "read summary rebuild row failed",
            summary_body,
            summary_budget_chars,
        )?
    };
    drop(stmt);

    if progress.saturated {
        return query_summary_frontier_turn_id_up_to_id(conn, session_id, through_turn_id);
    }

    Ok(progress.latest_turn_id)
}

fn stream_summary_turns_between_ids(
    conn: &Connection,
    session_id: &str,
    after_turn_id: i64,
    before_turn_id: i64,
    summary_body: &mut String,
    summary_budget_chars: usize,
) -> Result<Option<i64>, String> {
    #[cfg(test)]
    test_support::record_summary_streaming_query("catch_up");

    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_TURNS_BETWEEN_IDS,
        "prepare summary catch-up query failed",
    )?;
    let progress = {
        let mut rows = stmt
            .query(rusqlite::params![session_id, after_turn_id, before_turn_id])
            .map_err(|error| format!("query summary catch-up turns failed: {error}"))?;
        stream_summary_rows_until_saturation(
            &mut rows,
            "read summary catch-up row failed",
            summary_body,
            summary_budget_chars,
        )?
    };
    drop(stmt);

    if progress.saturated {
        return query_summary_frontier_turn_id_between_ids(
            conn,
            session_id,
            after_turn_id,
            before_turn_id,
        );
    }

    Ok(progress.latest_turn_id)
}

fn load_summary_checkpoint_meta(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<SummaryCheckpointMeta>, String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_SELECT_SUMMARY_CHECKPOINT_META,
        "prepare summary checkpoint metadata query failed",
    )?;
    let mut rows = stmt
        .query(rusqlite::params![session_id])
        .map_err(|error| format!("query summary checkpoint metadata failed: {error}"))?;

    let Some(row) = rows
        .next()
        .map_err(|error| format!("read summary checkpoint metadata row failed: {error}"))?
    else {
        return Ok(None);
    };

    let summary_body_len = row
        .get::<_, i64>(2)
        .map_err(|error| format!("decode summary checkpoint body length failed: {error}"))?
        .max(0) as usize;
    let summary_budget_chars = row
        .get::<_, i64>(3)
        .map_err(|error| format!("decode summary checkpoint metadata budget failed: {error}"))?
        .max(0) as usize;
    let summary_window_size = row
        .get::<_, i64>(4)
        .map_err(|error| format!("decode summary checkpoint metadata window failed: {error}"))?
        .max(0) as usize;

    let meta = SummaryCheckpointMeta {
        summarized_through_turn_id: row.get(0).map_err(|error| {
            format!("decode summary checkpoint metadata frontier failed: {error}")
        })?,
        summary_before_turn_id: row.get::<_, Option<i64>>(1).map_err(|error| {
            format!("decode summary checkpoint metadata boundary failed: {error}")
        })?,
        summary_body_len,
        summary_budget_chars,
        summary_window_size,
        summary_format_version: row.get(5).map_err(|error| {
            format!("decode summary checkpoint metadata version failed: {error}")
        })?,
    };

    Ok(Some(meta))
}

fn load_summary_checkpoint_body(
    conn: &Connection,
    session_id: &str,
    checkpoint_meta: SummaryCheckpointMeta,
) -> Result<Option<SummaryCheckpoint>, String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_SELECT_SUMMARY_CHECKPOINT_BODY,
        "prepare summary checkpoint body query failed",
    )?;
    let mut rows = stmt
        .query(rusqlite::params![session_id])
        .map_err(|error| format!("query summary checkpoint body failed: {error}"))?;

    let Some(row) = rows
        .next()
        .map_err(|error| format!("read summary checkpoint body row failed: {error}"))?
    else {
        return Ok(None);
    };

    Ok(Some(SummaryCheckpoint {
        summarized_through_turn_id: checkpoint_meta.summarized_through_turn_id,
        summary_before_turn_id: checkpoint_meta.summary_before_turn_id,
        summary_body: row
            .get(0)
            .map_err(|error| format!("decode summary checkpoint body failed: {error}"))?,
        summary_budget_chars: checkpoint_meta.summary_budget_chars,
        summary_window_size: checkpoint_meta.summary_window_size,
        summary_format_version: checkpoint_meta.summary_format_version,
    }))
}

fn load_summary_append_maintenance_state(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<SummaryAppendMaintenanceState, String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_SUMMARY_APPEND_MAINTENANCE_STATE,
        "prepare summary append maintenance state query failed",
    )?;
    let mut rows = stmt
        .query(rusqlite::params![session_id])
        .map_err(|error| format!("query summary append maintenance state failed: {error}"))?;

    let Some(row) = rows
        .next()
        .map_err(|error| format!("read summary append maintenance state row failed: {error}"))?
    else {
        return Ok(SummaryAppendMaintenanceState {
            summary_before_turn_id: None,
            checkpoint_meta: None,
        });
    };

    let next_summary_before_turn_id = row
        .get::<_, Option<i64>>(0)
        .map_err(|error| format!("decode summary boundary turn id failed: {error}"))?;
    let summarized_through_turn_id = row
        .get::<_, Option<i64>>(1)
        .map_err(|error| format!("decode summary checkpoint frontier failed: {error}"))?;
    let checkpoint_summary_before_turn_id = row
        .get::<_, Option<i64>>(2)
        .map_err(|error| format!("decode checkpoint boundary turn id failed: {error}"))?;
    let summary_body_len = row
        .get::<_, Option<i64>>(3)
        .map_err(|error| format!("decode summary checkpoint body length failed: {error}"))?
        .unwrap_or_default()
        .max(0) as usize;
    let summary_budget_chars = row
        .get::<_, Option<i64>>(4)
        .map_err(|error| format!("decode summary checkpoint budget failed: {error}"))?
        .unwrap_or_default()
        .max(0) as usize;
    let summary_window_size = row
        .get::<_, Option<i64>>(5)
        .map_err(|error| format!("decode summary checkpoint window failed: {error}"))?
        .unwrap_or_default()
        .max(0) as usize;
    let summary_format_version = row
        .get::<_, Option<i64>>(6)
        .map_err(|error| format!("decode summary checkpoint version failed: {error}"))?;

    let checkpoint_meta =
        summarized_through_turn_id.map(|summarized_through_turn_id| SummaryCheckpointMeta {
            summarized_through_turn_id,
            summary_before_turn_id: checkpoint_summary_before_turn_id,
            summary_body_len,
            summary_budget_chars,
            summary_window_size,
            summary_format_version: summary_format_version.unwrap_or_default(),
        });
    let can_incrementally_advance_boundary = checkpoint_meta.as_ref().is_some_and(|checkpoint| {
        checkpoint.summary_before_turn_id.is_some() && checkpoint.summary_window_size == limit
    });
    let summary_before_turn_id = if can_incrementally_advance_boundary {
        if let Some(summary_before_turn_id) = next_summary_before_turn_id {
            Some(summary_before_turn_id)
        } else {
            query_summary_boundary_turn_id_by_session_turn_count(conn, session_id, limit)?
        }
    } else {
        query_summary_boundary_turn_id_by_session_turn_count(conn, session_id, limit)?
    };

    Ok(SummaryAppendMaintenanceState {
        summary_before_turn_id,
        checkpoint_meta,
    })
}

fn reserve_next_session_turn_index(conn: &Connection, session_id: &str) -> Result<i64, String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_UPSERT_SESSION_TURN_COUNT,
        "prepare session turn count upsert failed",
    )?;
    stmt.query_row(rusqlite::params![session_id], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("reserve next session turn index failed: {error}"))
}

fn query_summary_boundary_turn_id_by_session_turn_count(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<Option<i64>, String> {
    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_SUMMARY_BOUNDARY_TURN_ID_BY_SESSION_TURN_COUNT,
        "prepare summary boundary turn id by session turn count query failed",
    )?;
    let mut rows = stmt
        .query(rusqlite::params![session_id, limit as i64])
        .map_err(|error| {
            format!("query summary boundary turn id by session turn count failed: {error}")
        })?;
    let direct_boundary_turn_id = rows
        .next()
        .map_err(|error| {
            format!("read summary boundary turn id by session turn count row failed: {error}")
        })?
        .map(|row| {
            row.get::<_, i64>(0).map_err(|error| {
                format!("decode summary boundary turn id by session turn count failed: {error}")
            })
        })
        .transpose()?;
    let fetch_limit = limit.saturating_add(1);
    let mut visible_rows = query_recent_prompt_turn_rows_with_ids(conn, session_id, fetch_limit)?;

    visible_rows.reverse();

    let has_visible_overflow = visible_rows.len() > limit;
    if !has_visible_overflow {
        return Ok(None);
    }

    let boundary_turn_id = visible_rows.get(1).map(|(turn_id, _)| *turn_id);

    Ok(boundary_turn_id.or(direct_boundary_turn_id))
}

fn upsert_summary_checkpoint(
    conn: &Connection,
    session_id: &str,
    checkpoint: &SummaryCheckpoint,
) -> Result<(), String> {
    upsert_summary_checkpoint_with_diagnostics(conn, session_id, checkpoint).map(|_| ())
}

fn upsert_summary_checkpoint_with_diagnostics(
    conn: &Connection,
    session_id: &str,
    checkpoint: &SummaryCheckpoint,
) -> Result<SqliteSummaryCheckpointUpsertDiagnostics, String> {
    conn.execute_batch("SAVEPOINT summary_checkpoint_upsert")
        .map_err(|error| format!("begin summary checkpoint upsert savepoint failed: {error}"))?;

    let mut diagnostics = SqliteSummaryCheckpointUpsertDiagnostics::default();
    let result = (|| {
        let metadata_upsert_started_at = StdInstant::now();
        let mut upsert_summary_metadata = prepare_cached_sqlite_statement(
            conn,
            SQL_UPSERT_SUMMARY_CHECKPOINT_METADATA,
            "prepare summary checkpoint metadata upsert failed",
        )?;
        upsert_summary_metadata
            .execute(rusqlite::params![
                session_id,
                checkpoint.summarized_through_turn_id,
                checkpoint.summary_before_turn_id,
                checkpoint.summary_body.len() as i64,
                checkpoint.summary_budget_chars as i64,
                checkpoint.summary_window_size as i64,
                checkpoint.summary_format_version,
                unix_ts_now(),
            ])
            .map_err(|error| format!("upsert summary checkpoint metadata failed: {error}"))?;
        diagnostics.metadata_upsert_ms += elapsed_ms(metadata_upsert_started_at);

        let body_upsert_started_at = StdInstant::now();
        let mut upsert_summary_body = prepare_cached_sqlite_statement(
            conn,
            SQL_UPSERT_SUMMARY_CHECKPOINT_BODY,
            "prepare summary checkpoint body upsert failed",
        )?;
        upsert_summary_body
            .execute(rusqlite::params![session_id, checkpoint.summary_body])
            .map_err(|error| format!("upsert summary checkpoint body failed: {error}"))?;
        diagnostics.body_upsert_ms += elapsed_ms(body_upsert_started_at);
        Ok(())
    })();

    match result {
        Ok(()) => {
            let commit_started_at = StdInstant::now();
            conn.execute_batch("RELEASE summary_checkpoint_upsert")
                .map_err(|error| {
                    format!("commit summary checkpoint upsert savepoint failed: {error}")
                })?;
            diagnostics.commit_ms += elapsed_ms(commit_started_at);
            Ok(diagnostics)
        }
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO summary_checkpoint_upsert;
                 RELEASE summary_checkpoint_upsert;",
            );
            Err(error)
        }
    }
}

fn update_summary_checkpoint_metadata(
    conn: &Connection,
    session_id: &str,
    summarized_through_turn_id: i64,
    summary_before_turn_id: i64,
    summary_budget_chars: usize,
    summary_window_size: usize,
    summary_format_version: i64,
) -> Result<(), String> {
    let mut update_summary = prepare_cached_sqlite_statement(
        conn,
        SQL_UPDATE_SUMMARY_CHECKPOINT_METADATA,
        "prepare summary checkpoint metadata update failed",
    )?;
    update_summary
        .execute(rusqlite::params![
            session_id,
            summarized_through_turn_id,
            summary_before_turn_id,
            summary_budget_chars as i64,
            summary_window_size as i64,
            summary_format_version,
            unix_ts_now(),
        ])
        .map(|_| ())
        .map_err(|error| format!("update summary checkpoint metadata failed: {error}"))
}

fn delete_summary_checkpoint(conn: &Connection, session_id: &str) -> Result<(), String> {
    let mut delete_summary = prepare_cached_sqlite_statement(
        conn,
        SQL_DELETE_SUMMARY_CHECKPOINT,
        "prepare summary checkpoint delete failed",
    )?;
    delete_summary
        .execute(rusqlite::params![session_id])
        .map(|_| ())
        .map_err(|error| format!("delete summary checkpoint failed: {error}"))
}

fn delete_session_state(conn: &Connection, session_id: &str) -> Result<(), String> {
    let mut delete_session_state = prepare_cached_sqlite_statement(
        conn,
        SQL_DELETE_SESSION_STATE,
        "prepare session-state delete failed",
    )?;
    delete_session_state
        .execute(rusqlite::params![session_id])
        .map(|_| ())
        .map_err(|error| format!("delete session-state failed: {error}"))
}

fn delete_canonical_records_for_session(conn: &Connection, session_id: &str) -> Result<(), String> {
    let mut delete_records = prepare_cached_sqlite_statement(
        conn,
        SQL_DELETE_CANONICAL_RECORDS_FOR_SESSION,
        "prepare canonical-record delete failed",
    )?;
    delete_records
        .execute(rusqlite::params![session_id])
        .map(|_| ())
        .map_err(|error| format!("delete canonical records failed: {error}"))
}

fn rebuild_canonical_record_storage(conn: &Connection) -> Result<(), String> {
    #[derive(Debug)]
    struct PersistedTurnRow {
        turn_id: i64,
        session_id: String,
        session_turn_index: i64,
        role: String,
        content: String,
        ts: i64,
    }

    conn.execute_batch("SAVEPOINT canonical_rebuild")
        .map_err(|error| format!("begin canonical rebuild savepoint failed: {error}"))?;

    let rebuild_result = (|| {
        drop_canonical_record_fts_index(conn)?;
        conn.execute("DELETE FROM memory_canonical_records", [])
            .map_err(|error| format!("clear canonical records before rebuild failed: {error}"))?;
        let mut last_turn_id = 0_i64;

        loop {
            let mut select_turns = prepare_cached_sqlite_statement(
                conn,
                SQL_SELECT_TURNS_FOR_CANONICAL_REBUILD,
                "prepare canonical rebuild turn query failed",
            )?;
            let rows = select_turns
                .query_map(
                    rusqlite::params![last_turn_id, CANONICAL_REBUILD_BATCH_SIZE],
                    |row| {
                        Ok(PersistedTurnRow {
                            turn_id: row.get(0)?,
                            session_id: row.get(1)?,
                            session_turn_index: row.get(2)?,
                            role: row.get(3)?,
                            content: row.get(4)?,
                            ts: row.get(5)?,
                        })
                    },
                )
                .map_err(|error| format!("query canonical rebuild turns failed: {error}"))?;
            let turns = rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("read canonical rebuild turns failed: {error}"))?;
            drop(select_turns);

            if turns.is_empty() {
                break;
            }

            for turn in &turns {
                last_turn_id = turn.turn_id;
                insert_canonical_record(
                    conn,
                    build_canonical_insert_input(
                        turn.session_id.as_str(),
                        turn.session_turn_index,
                        turn.role.as_str(),
                        turn.content.as_str(),
                        turn.ts,
                    ),
                )?;
            }
        }

        create_canonical_record_fts_index(conn)?;
        rebuild_canonical_record_fts_index_contents(conn)?;

        Ok(())
    })();

    match rebuild_result {
        Ok(()) => conn
            .execute_batch("RELEASE canonical_rebuild")
            .map_err(|error| format!("commit canonical rebuild savepoint failed: {error}")),
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO canonical_rebuild;
                 RELEASE canonical_rebuild;",
            );
            Err(error)
        }
    }
}

fn rebuild_canonical_record_storage_if_needed(conn: &Connection) -> Result<(), String> {
    let turn_count = conn
        .query_row(SQL_COUNT_TURNS, [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("count persisted turns for canonical rebuild failed: {error}"))?;
    let canonical_count = conn
        .query_row(SQL_COUNT_CANONICAL_RECORDS, [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("count canonical records failed: {error}"))?;
    let canonical_fts_count = conn
        .query_row(SQL_COUNT_CANONICAL_FTS_ROWS, [], |row| row.get::<_, i64>(0))
        .map_err(|error| format!("count canonical FTS rows failed: {error}"))?;

    if canonical_count == turn_count && canonical_fts_count == canonical_count {
        return Ok(());
    }

    rebuild_canonical_record_storage(conn)
}

fn build_canonical_fts_query(query: &str) -> Option<String> {
    crate::search_text::build_search_fts_query(query, 6)
}

pub(super) fn search_canonical_records_for_recall(
    query: &str,
    limit: usize,
    exclude_session_id: Option<&str>,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<CanonicalMemorySearchHit>, String> {
    let Some(match_query) = build_canonical_fts_query(query) else {
        return Ok(Vec::new());
    };

    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("memory.search_canonical_records", |conn| {
        let mut stmt = prepare_cached_sqlite_statement(
            conn,
            SQL_SEARCH_CANONICAL_RECORDS,
            "prepare canonical memory search statement failed",
        )?;
        let mut rows = stmt
            .query(rusqlite::params![
                match_query,
                exclude_session_id,
                limit.clamp(1, 16) as i64
            ])
            .map_err(|error| format!("query canonical memory search failed: {error}"))?;
        let mut hits = Vec::new();

        while let Some(row) = rows
            .next()
            .map_err(|error| format!("read canonical memory search row failed: {error}"))?
        {
            let session_id = row.get::<_, String>(0).map_err(|error| {
                format!("decode canonical memory search session id failed: {error}")
            })?;
            let session_turn_index = row.get::<_, i64>(1).map_err(|error| {
                format!("decode canonical memory search turn index failed: {error}")
            })?;
            let scope_text = row
                .get::<_, String>(2)
                .map_err(|error| format!("decode canonical memory search scope failed: {error}"))?;
            let kind_text = row
                .get::<_, String>(3)
                .map_err(|error| format!("decode canonical memory search kind failed: {error}"))?;
            let role = row
                .get::<_, Option<String>>(4)
                .map_err(|error| format!("decode canonical memory search role failed: {error}"))?;
            let content = row.get::<_, String>(5).map_err(|error| {
                format!("decode canonical memory search content failed: {error}")
            })?;
            let metadata_json = row.get::<_, String>(6).map_err(|error| {
                format!("decode canonical memory search metadata failed: {error}")
            })?;
            let _ts = row.get::<_, i64>(7).map_err(|error| {
                format!("decode canonical memory search timestamp failed: {error}")
            })?;

            let Some(scope) = MemoryScope::parse_id(scope_text.as_str()) else {
                continue;
            };
            let Some(kind) = CanonicalMemoryKind::parse_id(kind_text.as_str()) else {
                continue;
            };
            let metadata =
                serde_json::from_str::<Value>(metadata_json.as_str()).unwrap_or_else(|_| json!({}));

            hits.push(CanonicalMemorySearchHit {
                record: CanonicalMemoryRecord {
                    session_id,
                    scope,
                    kind,
                    role,
                    content,
                    metadata,
                },
                session_turn_index: Some(session_turn_index),
            });
        }

        Ok(hits)
    })
}

fn maintain_summary_checkpoint_after_append(
    conn: &Connection,
    session_id: &str,
    append_maintenance_state: SummaryAppendMaintenanceState,
    config: &MemoryRuntimeConfig,
) -> Result<(), String> {
    let summary_budget_chars = config.summary_max_chars.max(256);
    let summary_window_size = default_window_size(config);
    let checkpoint_meta = append_maintenance_state.checkpoint_meta.clone();
    let summary_target_through_turn_id = append_maintenance_state
        .summary_before_turn_id
        .map(|turn_id| turn_id.saturating_sub(1))
        .unwrap_or_default();

    if summary_target_through_turn_id <= 0 {
        if checkpoint_meta.is_some() {
            delete_summary_checkpoint(conn, session_id)?;
        }
        return Ok(());
    }

    if let Some(ref checkpoint_meta) = checkpoint_meta {
        let summary_is_saturated = checkpoint_meta.summary_body_len >= summary_budget_chars;
        let compatible_checkpoint = checkpoint_meta.summary_budget_chars == summary_budget_chars
            && checkpoint_meta.summary_format_version == SUMMARY_FORMAT_VERSION
            && checkpoint_meta.summarized_through_turn_id <= summary_target_through_turn_id;

        if compatible_checkpoint && summary_is_saturated {
            if checkpoint_meta.summarized_through_turn_id != summary_target_through_turn_id
                || checkpoint_meta.summary_before_turn_id
                    != append_maintenance_state.summary_before_turn_id
                || checkpoint_meta.summary_window_size != summary_window_size
            {
                update_summary_checkpoint_metadata(
                    conn,
                    session_id,
                    summary_target_through_turn_id,
                    append_maintenance_state
                        .summary_before_turn_id
                        .unwrap_or_default(),
                    summary_budget_chars,
                    summary_window_size,
                    SUMMARY_FORMAT_VERSION,
                )?;
            }
            return Ok(());
        }
    } else {
        let _ = rebuild_summary_checkpoint(
            conn,
            session_id,
            append_maintenance_state
                .summary_before_turn_id
                .unwrap_or_default(),
            summary_target_through_turn_id,
            summary_budget_chars,
            summary_window_size,
        )?;
        return Ok(());
    }

    let _ = materialize_summary_checkpoint(
        conn,
        session_id,
        append_maintenance_state.summary_before_turn_id,
        checkpoint_meta,
        config,
    )?;
    Ok(())
}

fn materialize_summary_checkpoint(
    conn: &Connection,
    session_id: &str,
    summary_before_turn_id: Option<i64>,
    existing_meta: Option<SummaryCheckpointMeta>,
    config: &MemoryRuntimeConfig,
) -> Result<Option<SummaryCheckpoint>, String> {
    let mut diagnostics = SqliteContextLoadDiagnostics::default();
    materialize_summary_checkpoint_with_diagnostics(
        conn,
        session_id,
        summary_before_turn_id,
        SummaryCheckpointMetaLookup::Known(existing_meta),
        config,
        &mut diagnostics,
    )
}

fn materialize_summary_checkpoint_with_diagnostics(
    conn: &Connection,
    session_id: &str,
    summary_before_turn_id: Option<i64>,
    existing_meta_lookup: SummaryCheckpointMetaLookup,
    config: &MemoryRuntimeConfig,
    diagnostics: &mut SqliteContextLoadDiagnostics,
) -> Result<Option<SummaryCheckpoint>, String> {
    let summary_budget_chars = config.summary_max_chars.max(256);
    let summary_window_size = default_window_size(config);
    let summary_target_through_turn_id = summary_before_turn_id
        .map(|turn_id| turn_id.saturating_sub(1))
        .unwrap_or_default();

    // Wrap all checkpoint writes in a savepoint so a mid-materialization
    // failure (e.g. disk full) cannot leave the checkpoint deleted without
    // a replacement — the entire write set rolls back atomically.
    conn.execute_batch("SAVEPOINT materialize_summary")
        .map_err(|error| format!("begin materialize summary savepoint failed: {error}"))?;

    let result = materialize_summary_checkpoint_inner(
        conn,
        session_id,
        summary_before_turn_id,
        existing_meta_lookup,
        summary_target_through_turn_id,
        summary_budget_chars,
        summary_window_size,
        diagnostics,
    );

    match &result {
        Ok(_) => {
            conn.execute_batch("RELEASE materialize_summary")
                .map_err(|error| format!("commit materialize summary savepoint failed: {error}"))?;
        }
        Err(_) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO materialize_summary;
                 RELEASE materialize_summary;",
            );
        }
    }

    result
}

fn materialize_summary_checkpoint_inner(
    conn: &Connection,
    session_id: &str,
    summary_before_turn_id: Option<i64>,
    existing_meta_lookup: SummaryCheckpointMetaLookup,
    summary_target_through_turn_id: i64,
    summary_budget_chars: usize,
    summary_window_size: usize,
    diagnostics: &mut SqliteContextLoadDiagnostics,
) -> Result<Option<SummaryCheckpoint>, String> {
    if summary_target_through_turn_id <= 0 {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    }

    let existing_meta = match existing_meta_lookup {
        SummaryCheckpointMetaLookup::Known(checkpoint_meta) => checkpoint_meta,
        SummaryCheckpointMetaLookup::Unknown => {
            let meta_query_started_at = StdInstant::now();
            let loaded_meta = load_summary_checkpoint_meta(conn, session_id)?;
            diagnostics.summary_checkpoint_meta_query_ms += elapsed_ms(meta_query_started_at);
            loaded_meta
        }
    };
    let needs_rebuild = existing_meta.as_ref().is_none_or(|checkpoint| {
        checkpoint.summary_format_version != SUMMARY_FORMAT_VERSION
            || checkpoint.summarized_through_turn_id > summary_target_through_turn_id
            || checkpoint_budget_change_requires_rebuild(checkpoint, summary_budget_chars)
    });

    let mut checkpoint = if needs_rebuild {
        let rebuild_started_at = StdInstant::now();
        let checkpoint = rebuild_summary_checkpoint_with_diagnostics(
            conn,
            session_id,
            summary_before_turn_id.unwrap_or_default(),
            summary_target_through_turn_id,
            summary_budget_chars,
            summary_window_size,
            Some(diagnostics),
        )?;
        diagnostics.summary_rebuild_ms += elapsed_ms(rebuild_started_at);
        checkpoint
    } else {
        match existing_meta {
            Some(checkpoint_meta) => {
                let body_load_started_at = StdInstant::now();
                let checkpoint = load_summary_checkpoint_body(conn, session_id, checkpoint_meta)?;
                diagnostics.summary_checkpoint_body_load_ms += elapsed_ms(body_load_started_at);
                checkpoint
            }
            None => None,
        }
    };

    if let Some(checkpoint_state) = checkpoint.as_mut()
        && let Some(summary_boundary_id) = summary_before_turn_id
        && checkpoint_state.summarized_through_turn_id < summary_target_through_turn_id
    {
        let catch_up_started_at = StdInstant::now();
        let latest_turn_id = stream_summary_turns_between_ids(
            conn,
            session_id,
            checkpoint_state.summarized_through_turn_id,
            summary_boundary_id,
            &mut checkpoint_state.summary_body,
            summary_budget_chars,
        )?;
        diagnostics.summary_catch_up_ms += elapsed_ms(catch_up_started_at);

        if let Some(last_turn_id) = latest_turn_id {
            checkpoint_state.summarized_through_turn_id = last_turn_id;
            checkpoint_state.summary_before_turn_id = Some(summary_boundary_id);
            checkpoint_state.summary_budget_chars = summary_budget_chars;
            checkpoint_state.summary_window_size = summary_window_size;
            checkpoint_state.summary_format_version = SUMMARY_FORMAT_VERSION;
            upsert_summary_checkpoint(conn, session_id, checkpoint_state)?;
        }
    }

    if let Some(checkpoint_state) = checkpoint.as_mut()
        && checkpoint_state.summarized_through_turn_id == summary_target_through_turn_id
        && (checkpoint_state.summary_budget_chars != summary_budget_chars
            || checkpoint_state.summary_window_size != summary_window_size
            || checkpoint_state.summary_before_turn_id != summary_before_turn_id)
    {
        checkpoint_state.summary_before_turn_id = summary_before_turn_id;
        checkpoint_state.summary_budget_chars = summary_budget_chars;
        checkpoint_state.summary_window_size = summary_window_size;
        checkpoint_state.summary_format_version = SUMMARY_FORMAT_VERSION;
        let metadata_update_started_at = StdInstant::now();
        update_summary_checkpoint_metadata(
            conn,
            session_id,
            checkpoint_state.summarized_through_turn_id,
            checkpoint_state.summary_before_turn_id.unwrap_or_default(),
            checkpoint_state.summary_budget_chars,
            checkpoint_state.summary_window_size,
            checkpoint_state.summary_format_version,
        )?;
        diagnostics.summary_checkpoint_metadata_update_ms += elapsed_ms(metadata_update_started_at);
    }

    if checkpoint
        .as_ref()
        .is_some_and(|checkpoint| checkpoint.summary_body.is_empty())
    {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    }

    Ok(checkpoint)
}

fn checkpoint_budget_change_requires_rebuild(
    checkpoint: &SummaryCheckpointMeta,
    target_summary_budget_chars: usize,
) -> bool {
    checkpoint.summary_budget_chars != target_summary_budget_chars
        && (checkpoint.summary_body_len >= checkpoint.summary_budget_chars
            || checkpoint.summary_body_len > target_summary_budget_chars)
}

fn rebuild_summary_checkpoint(
    conn: &Connection,
    session_id: &str,
    summary_before_turn_id: i64,
    summary_target_through_turn_id: i64,
    summary_budget_chars: usize,
    summary_window_size: usize,
) -> Result<Option<SummaryCheckpoint>, String> {
    rebuild_summary_checkpoint_with_diagnostics(
        conn,
        session_id,
        summary_before_turn_id,
        summary_target_through_turn_id,
        summary_budget_chars,
        summary_window_size,
        None,
    )
}

fn rebuild_summary_checkpoint_with_diagnostics(
    conn: &Connection,
    session_id: &str,
    summary_before_turn_id: i64,
    summary_target_through_turn_id: i64,
    summary_budget_chars: usize,
    summary_window_size: usize,
    mut diagnostics: Option<&mut SqliteContextLoadDiagnostics>,
) -> Result<Option<SummaryCheckpoint>, String> {
    let mut summary_body = String::with_capacity(summary_budget_chars);
    let stream_started_at = StdInstant::now();
    let summarized_through_turn_id = stream_summary_turns_up_to_id(
        conn,
        session_id,
        summary_target_through_turn_id,
        &mut summary_body,
        summary_budget_chars,
    )?;
    let stream_elapsed_ms = elapsed_ms(stream_started_at);
    if let Some(diagnostics) = diagnostics.as_deref_mut() {
        diagnostics.summary_rebuild_stream_ms += stream_elapsed_ms;
    }
    if summary_body.is_empty() {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    }

    let checkpoint = SummaryCheckpoint {
        summarized_through_turn_id: summarized_through_turn_id
            .unwrap_or(summary_target_through_turn_id),
        summary_before_turn_id: Some(summary_before_turn_id),
        summary_body,
        summary_budget_chars,
        summary_window_size,
        summary_format_version: SUMMARY_FORMAT_VERSION,
    };
    let checkpoint_upsert_started_at = StdInstant::now();
    let checkpoint_upsert_diagnostics =
        upsert_summary_checkpoint_with_diagnostics(conn, session_id, &checkpoint)?;
    let checkpoint_upsert_elapsed_ms = elapsed_ms(checkpoint_upsert_started_at);
    if let Some(diagnostics) = diagnostics {
        diagnostics.summary_rebuild_checkpoint_upsert_ms += checkpoint_upsert_elapsed_ms;
        diagnostics.summary_rebuild_checkpoint_metadata_upsert_ms +=
            checkpoint_upsert_diagnostics.metadata_upsert_ms;
        diagnostics.summary_rebuild_checkpoint_body_upsert_ms +=
            checkpoint_upsert_diagnostics.body_upsert_ms;
        diagnostics.summary_rebuild_checkpoint_commit_ms += checkpoint_upsert_diagnostics.commit_ms;
    }
    Ok(Some(checkpoint))
}

fn materialize_initial_summary_checkpoint(
    conn: &Connection,
    session_id: &str,
    summary_budget_chars: usize,
    summary_window_size: usize,
) -> Result<Option<SummaryCheckpoint>, String> {
    let visible_limit = summary_window_size.saturating_add(1);
    let mut visible_rows = query_recent_prompt_turn_rows_with_ids(conn, session_id, visible_limit)?;

    visible_rows.reverse();

    if visible_rows.len() <= summary_window_size {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    }

    let summary_before_turn_id = visible_rows.get(1).map(|(turn_id, _turn)| *turn_id);
    let Some(summary_before_turn_id) = summary_before_turn_id else {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    };

    let mut stmt = prepare_cached_sqlite_statement(
        conn,
        SQL_QUERY_INITIAL_SUMMARY_ROWS_BY_SESSION_TURN_INDEX,
        "prepare initial summary checkpoint query failed",
    )?;
    let mut rows = stmt
        .query(rusqlite::params![session_id])
        .map_err(|error| format!("query initial summary checkpoint rows failed: {error}"))?;

    let mut first_visible_turn_id: Option<i64> = None;
    let mut first_visible_role: Option<String> = None;
    let mut first_visible_content: Option<String> = None;

    collect_initial_summary_first_visible_turn(
        session_id,
        &mut rows,
        &mut first_visible_turn_id,
        &mut first_visible_role,
        &mut first_visible_content,
    )?;

    if first_visible_turn_id.is_none() {
        let mut stmt = prepare_cached_sqlite_statement(
            conn,
            SQL_QUERY_INITIAL_SUMMARY_ROWS_AFTER_SEED_SESSION_INDEX,
            "prepare initial summary checkpoint fallback query failed",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id]).map_err(|error| {
            format!("query initial summary checkpoint fallback rows failed: {error}")
        })?;

        collect_initial_summary_first_visible_turn(
            session_id,
            &mut rows,
            &mut first_visible_turn_id,
            &mut first_visible_role,
            &mut first_visible_content,
        )?;
    }

    let Some(first_visible_turn_id) = first_visible_turn_id else {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    };
    let Some(first_visible_role) = first_visible_role else {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    };
    let Some(first_visible_content) = first_visible_content else {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    };
    let mut summary_body = String::with_capacity(summary_budget_chars);

    append_summary_line(
        &mut summary_body,
        first_visible_role.as_str(),
        first_visible_content.as_str(),
        summary_budget_chars,
    );

    if summary_body.is_empty() {
        delete_summary_checkpoint(conn, session_id)?;
        return Ok(None);
    }

    let checkpoint = SummaryCheckpoint {
        summarized_through_turn_id: first_visible_turn_id,
        summary_before_turn_id: Some(summary_before_turn_id),
        summary_body,
        summary_budget_chars,
        summary_window_size,
        summary_format_version: SUMMARY_FORMAT_VERSION,
    };
    upsert_summary_checkpoint(conn, session_id, &checkpoint)?;
    Ok(Some(checkpoint))
}

fn collect_initial_summary_first_visible_turn(
    session_id: &str,
    rows: &mut rusqlite::Rows<'_>,
    first_visible_turn_id: &mut Option<i64>,
    first_visible_role: &mut Option<String>,
    first_visible_content: &mut Option<String>,
) -> Result<(), String> {
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("read initial summary checkpoint row failed: {error}"))?
    {
        let turn_id = row
            .get::<_, i64>(0)
            .map_err(|error| format!("decode initial summary turn id failed: {error}"))?;
        let role = row
            .get_ref(1)
            .map_err(|error| format!("decode initial summary turn role failed: {error}"))?
            .as_str()
            .map_err(|error| format!("decode initial summary turn role failed: {error}"))?;
        let content = row
            .get_ref(2)
            .map_err(|error| format!("decode initial summary turn content failed: {error}"))?
            .as_str()
            .map_err(|error| format!("decode initial summary turn content failed: {error}"))?;
        let include_turn = prompt_window_turn_is_visible(session_id, role, content);

        if !include_turn {
            continue;
        }

        if first_visible_turn_id.is_none() {
            #[cfg(test)]
            test_support::record_summary_row_observed();
            #[cfg(test)]
            test_support::record_summary_payload_decode();
            *first_visible_turn_id = Some(turn_id);
            *first_visible_role = Some(role.to_owned());
            *first_visible_content = Some(content.to_owned());
            return Ok(());
        }
    }

    Ok(())
}

fn append_summary_line(
    summary_body: &mut String,
    role: &str,
    content: &str,
    summary_budget_chars: usize,
) {
    let mut remaining_bytes = summary_budget_chars.saturating_sub(summary_body.len());
    if remaining_bytes == 0 {
        return;
    }

    let mut tokens = content.split_whitespace();
    let Some(first_token) = tokens.next() else {
        return;
    };

    if !summary_body.is_empty() {
        append_truncated_summary_fragment(summary_body, "\n", &mut remaining_bytes);
    }
    append_truncated_summary_fragment(summary_body, "- ", &mut remaining_bytes);
    append_truncated_summary_fragment(summary_body, role, &mut remaining_bytes);
    append_truncated_summary_fragment(summary_body, ": ", &mut remaining_bytes);
    if !append_truncated_summary_fragment(summary_body, first_token, &mut remaining_bytes) {
        return;
    }
    for token in tokens {
        if remaining_bytes == 0 {
            return;
        }
        append_truncated_summary_fragment(summary_body, " ", &mut remaining_bytes);
        if !append_truncated_summary_fragment(summary_body, token, &mut remaining_bytes) {
            return;
        }
    }
}

fn reserve_summary_body_capacity(summary_body: &mut String, summary_budget_chars: usize) {
    if summary_body.capacity() < summary_budget_chars {
        summary_body.reserve(summary_budget_chars - summary_body.capacity());
    }
}

fn append_truncated_summary_fragment(
    summary_body: &mut String,
    fragment: &str,
    remaining_bytes: &mut usize,
) -> bool {
    if *remaining_bytes == 0 || fragment.is_empty() {
        return fragment.is_empty();
    }

    if fragment.len() <= *remaining_bytes {
        summary_body.push_str(fragment);
        *remaining_bytes -= fragment.len();
        return true;
    }

    let mut end = 0;
    for (idx, ch) in fragment.char_indices() {
        let next = idx + ch.len_utf8();
        if next > *remaining_bytes {
            break;
        }
        end = next;
    }

    if end > 0 {
        summary_body.push_str(&fragment[..end]);
        *remaining_bytes -= end;
        false
    } else {
        *remaining_bytes = 0;
        false
    }
}

pub(super) fn format_summary_block(summary_body: &str) -> Option<String> {
    let trimmed = summary_body.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(format!(
        "## Memory Summary\nEarlier session context condensed from turns outside the active window:\n{trimmed}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MemoryProfile;
    use crate::test_support::ScopedCurrentDir;
    use serde_json::json;

    fn sqlite_test_config(db_path: impl Into<PathBuf>) -> MemoryRuntimeConfig {
        MemoryRuntimeConfig::for_sqlite_path(db_path)
    }

    fn sqlite_test_config_with_profile(
        db_path: impl Into<PathBuf>,
        profile: MemoryProfile,
        sliding_window: usize,
    ) -> MemoryRuntimeConfig {
        let mut config = sqlite_test_config(db_path);
        config.profile = profile;
        config.mode = profile.mode();
        config.sliding_window = sliding_window;
        config
    }

    fn sqlite_test_summary_config(
        db_path: impl Into<PathBuf>,
        sliding_window: usize,
        summary_max_chars: usize,
    ) -> MemoryRuntimeConfig {
        let mut config = sqlite_test_config_with_profile(
            db_path,
            MemoryProfile::WindowPlusSummary,
            sliding_window,
        );
        config.summary_max_chars = summary_max_chars;
        config
    }

    fn read_summary_checkpoint(
        config: &MemoryRuntimeConfig,
        session_id: &str,
    ) -> Result<(i64, String, i64, i64), String> {
        let runtime = acquire_memory_runtime(config)?;
        runtime.with_connection("test.read_summary_checkpoint", |conn| {
            conn.query_row(
                "SELECT checkpoint.summarized_through_turn_id,
                        body.summary_body,
                        checkpoint.summary_budget_chars,
                        checkpoint.summary_window_size
                 FROM memory_summary_checkpoints AS checkpoint
                 JOIN memory_summary_checkpoint_bodies AS body
                   ON body.session_id = checkpoint.session_id
                 WHERE checkpoint.session_id = ?1",
                rusqlite::params![session_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(|error| format!("read summary checkpoint failed: {error}"))
        })
    }

    fn count_summary_checkpoints(
        config: &MemoryRuntimeConfig,
        session_id: &str,
    ) -> Result<i64, String> {
        let runtime = acquire_memory_runtime(config)?;
        runtime.with_connection("test.count_summary_checkpoints", |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM memory_summary_checkpoints WHERE session_id = ?1",
                rusqlite::params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("count summary checkpoints failed: {error}"))
        })
    }

    fn read_summary_checkpoint_boundary_turn_id(
        config: &MemoryRuntimeConfig,
        session_id: &str,
    ) -> Result<Option<i64>, String> {
        let runtime = acquire_memory_runtime(config)?;
        runtime.with_connection("test.read_summary_checkpoint_boundary_turn_id", |conn| {
            conn.query_row(
                "SELECT summary_before_turn_id
                 FROM memory_summary_checkpoints
                 WHERE session_id = ?1",
                rusqlite::params![session_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|error| format!("read summary checkpoint boundary turn id failed: {error}"))
        })
    }

    fn read_session_turn_indices(
        config: &MemoryRuntimeConfig,
        session_id: &str,
    ) -> Result<Vec<i64>, String> {
        let runtime = acquire_memory_runtime(config)?;
        runtime.with_connection("test.read_session_turn_indices", |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT session_turn_index
                     FROM turns
                     WHERE session_id = ?1
                     ORDER BY id ASC",
                )
                .map_err(|error| format!("prepare session turn index query failed: {error}"))?;
            let rows = stmt
                .query_map(rusqlite::params![session_id], |row| row.get::<_, i64>(0))
                .map_err(|error| format!("query session turn indices failed: {error}"))?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("decode session turn indices failed: {error}"))
        })
    }

    fn read_session_turn_count(
        config: &MemoryRuntimeConfig,
        session_id: &str,
    ) -> Result<Option<i64>, String> {
        let runtime = acquire_memory_runtime(config)?;
        runtime.with_connection("test.read_session_turn_count", |conn| {
            conn.query_row(
                "SELECT turn_count
                 FROM memory_session_state
                 WHERE session_id = ?1",
                rusqlite::params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .map(Some)
            .or_else(|error| {
                if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(None)
                } else {
                    Err(error)
                }
            })
            .map_err(|error| format!("read session turn count failed: {error}"))
        })
    }

    #[test]
    fn load_window_includes_turn_count_in_payload() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-window-turn-count-payload-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("window-turn-count.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        append_turn_direct("window-turn-count-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("window-turn-count-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("window-turn-count-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let outcome = load_window(
            crate::memory::build_window_request("window-turn-count-session", 2),
            &config,
        )
        .expect("window load should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["turn_count"], 3);
        assert_eq!(
            outcome.payload["turns"]
                .as_array()
                .expect("window payload turns")
                .len(),
            2
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn replace_turns_requires_object_payload() {
        let error = replace_turns(
            MemoryCoreRequest {
                operation: MEMORY_OP_REPLACE_TURNS.to_owned(),
                payload: json!("not-an-object"),
            },
            &MemoryRuntimeConfig::default(),
        )
        .expect_err("replace_turns should reject non-object payloads");

        assert_eq!(error, "memory.replace_turns payload must be an object");
    }

    #[test]
    fn replace_turns_rejects_malformed_expected_turn_count() {
        let error = replace_turns(
            MemoryCoreRequest {
                operation: MEMORY_OP_REPLACE_TURNS.to_owned(),
                payload: json!({
                    "session_id": "replace-turns-invalid-expected-count",
                    "turns": [],
                    "expected_turn_count": "invalid",
                }),
            },
            &MemoryRuntimeConfig::default(),
        )
        .expect_err("replace_turns should reject malformed expected_turn_count");

        assert_eq!(
            error,
            "memory.replace_turns payload.expected_turn_count must be a non-negative integer"
        );
    }

    #[test]
    fn replace_turns_uses_turn_rows_when_session_state_metadata_is_missing() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-replace-turns-fallback-count-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("replace-turns-fallback-count.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);
        let session_id = "replace-turns-fallback-count-session";

        append_turn_direct(session_id, "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(session_id, "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct(session_id, "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let runtime = acquire_memory_runtime(&config).expect("acquire runtime");
        runtime
            .with_connection_mut("test.delete_turn_count_before_replace", |conn| {
                conn.execute(
                    "DELETE FROM memory_session_state WHERE session_id = ?1",
                    rusqlite::params![session_id],
                )
                .map_err(|error| format!("delete session turn count metadata failed: {error}"))
            })
            .expect("delete turn count metadata");

        let outcome = replace_turns(
            MemoryCoreRequest {
                operation: MEMORY_OP_REPLACE_TURNS.to_owned(),
                payload: json!({
                    "session_id": session_id,
                    "turns": [
                        {"role": "user", "content": "replacement 1", "ts": 11},
                        {"role": "assistant", "content": "replacement 2", "ts": 12},
                    ],
                    "expected_turn_count": 3,
                }),
            },
            &config,
        )
        .expect("replace_turns should fall back to turn rows when session metadata is missing");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["replaced_turns"], 2);
        assert_eq!(
            read_session_turn_count(&config, session_id).expect("read session turn count"),
            Some(2)
        );
        assert_eq!(
            window_direct(session_id, 4, &config)
                .expect("load replacement turns")
                .into_iter()
                .map(|turn| turn.content)
                .collect::<Vec<_>>(),
            vec!["replacement 1".to_owned(), "replacement 2".to_owned()]
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn memory_operations_reuse_cached_sqlite_runtime_for_same_path() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-reuse-same-path-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("runtime-reuse.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config).expect("ensure memory db ready");
        let turns = window_direct_with_options("runtime-reuse-session", 2, true, &config)
            .expect("window query should succeed");

        assert!(turns.is_empty(), "expected no turns for a fresh session");
        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path),
            1,
            "expected same-path operations to reuse one SQLite runtime bootstrap"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn concurrent_same_path_bootstrap_reuses_one_cold_runtime() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-concurrent-bootstrap-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("runtime-concurrent.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        configure_sqlite_runtime_cache_miss_for_tests(&db_path, 2);

        let start_barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let thread_a_barrier = start_barrier.clone();
        let thread_a_path = db_path.clone();
        let thread_a_config = config.clone();
        let thread_a = std::thread::spawn(move || {
            thread_a_barrier.wait();
            ensure_memory_db_ready(Some(thread_a_path), &thread_a_config)
        });

        let thread_b_barrier = start_barrier.clone();
        let thread_b_path = db_path.clone();
        let thread_b_config = config;
        let thread_b = std::thread::spawn(move || {
            thread_b_barrier.wait();
            ensure_memory_db_ready(Some(thread_b_path), &thread_b_config)
        });

        start_barrier.wait();

        let thread_a_result = thread_a.join().expect("join bootstrap thread a");
        let thread_b_result = thread_b.join().expect("join bootstrap thread b");

        clear_sqlite_runtime_cache_miss_for_tests();

        thread_a_result.expect("bootstrap thread a should succeed");
        thread_b_result.expect("bootstrap thread b should succeed");

        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path),
            1,
            "expected concurrent cold access to serialize same-path bootstrap work"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn distinct_sqlite_paths_get_distinct_runtime_bootstraps() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-reuse-distinct-paths-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path_a = tmp.join("runtime-a.sqlite3");
        let db_path_b = tmp.join("runtime-b.sqlite3");
        let _ = fs::remove_file(&db_path_a);
        let _ = fs::remove_file(&db_path_b);

        let config_a =
            sqlite_test_config_with_profile(db_path_a.clone(), MemoryProfile::WindowOnly, 2);
        let config_b =
            sqlite_test_config_with_profile(db_path_b.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path_a.clone()), &config_a).expect("ensure db a ready");
        window_direct_with_options("runtime-a-session", 2, true, &config_a)
            .expect("window query for db a should succeed");
        ensure_memory_db_ready(Some(db_path_b.clone()), &config_b).expect("ensure db b ready");
        window_direct_with_options("runtime-b-session", 2, true, &config_b)
            .expect("window query for db b should succeed");

        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path_a),
            1,
            "expected db path a to bootstrap once"
        );
        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path_b),
            1,
            "expected db path b to bootstrap once"
        );

        let _ = fs::remove_file(&db_path_a);
        let _ = fs::remove_file(&db_path_b);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn resetting_cached_runtime_forces_runtime_recreation_on_next_access() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-reuse-reset-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("runtime-reset.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config).expect("ensure memory db ready");
        window_direct_with_options("runtime-reset-session", 2, true, &config)
            .expect("initial window query should succeed");
        drop_cached_sqlite_runtime_for_tests(&db_path);
        window_direct_with_options("runtime-reset-session", 2, true, &config)
            .expect("window query after reset should succeed");

        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path),
            2,
            "expected cached runtime reset to force exactly one additional bootstrap"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn dropping_one_cached_runtime_preserves_other_cached_runtimes() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-drop-one-preserve-others-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path_a = tmp.join("runtime-a.sqlite3");
        let db_path_b = tmp.join("runtime-b.sqlite3");
        let _ = fs::remove_file(&db_path_a);
        let _ = fs::remove_file(&db_path_b);

        let config_a =
            sqlite_test_config_with_profile(db_path_a.clone(), MemoryProfile::WindowOnly, 2);
        let config_b =
            sqlite_test_config_with_profile(db_path_b.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path_a.clone()), &config_a).expect("ensure db a ready");
        window_direct_with_options("runtime-drop-a", 2, true, &config_a)
            .expect("window query for db a should succeed");
        ensure_memory_db_ready(Some(db_path_b.clone()), &config_b).expect("ensure db b ready");
        window_direct_with_options("runtime-drop-b", 2, true, &config_b)
            .expect("window query for db b should succeed");

        drop_cached_sqlite_runtime(&db_path_a).expect("drop cached runtime a");

        window_direct_with_options("runtime-drop-a", 2, true, &config_a)
            .expect("window query for db a after drop should succeed");
        window_direct_with_options("runtime-drop-b", 2, true, &config_b)
            .expect("window query for db b after dropping db a should still succeed");

        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path_a),
            2,
            "expected dropping db a to force one additional bootstrap for db a"
        );
        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path_b),
            1,
            "expected dropping db a to preserve the cached runtime for db b"
        );

        let _ = fs::remove_file(&db_path_a);
        let _ = fs::remove_file(&db_path_b);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn equivalent_relative_and_absolute_paths_share_one_runtime() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-alias-relative-absolute-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("data").join("alias.sqlite3");
        let _cwd_guard = ScopedCurrentDir::new(&tmp);

        let relative_config =
            sqlite_test_config_with_profile("data/alias.sqlite3", MemoryProfile::WindowOnly, 2);
        let absolute_config =
            sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(None, &relative_config).expect("ensure relative db ready");
        window_direct_with_options("relative-alias-session", 2, true, &relative_config)
            .expect("relative alias window query should succeed");
        ensure_memory_db_ready(None, &absolute_config).expect("ensure absolute db ready");
        window_direct_with_options("absolute-alias-session", 2, true, &absolute_config)
            .expect("absolute alias window query should succeed");

        assert_eq!(
            sqlite_bootstrap_count_under_prefix_for_tests(&tmp),
            1,
            "expected equivalent relative and absolute aliases to share one bootstrap"
        );
        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path),
            1,
            "expected normalized bootstrap count to be recorded under the canonical path"
        );

        drop(_cwd_guard);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dot_dot_aliases_share_one_runtime_after_normalization() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-alias-dotdot-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        let cwd = tmp.join("workspace").join("nested");
        fs::create_dir_all(&cwd).expect("create nested cwd dir");
        let db_path = tmp.join("workspace").join("data").join("alias.sqlite3");
        let _cwd_guard = ScopedCurrentDir::new(&cwd);

        let alias_a =
            sqlite_test_config_with_profile("../data/alias.sqlite3", MemoryProfile::WindowOnly, 2);
        let alias_b = sqlite_test_config_with_profile(
            "../nested/../data/./alias.sqlite3",
            MemoryProfile::WindowOnly,
            2,
        );

        ensure_memory_db_ready(None, &alias_a).expect("ensure dot-dot alias a ready");
        window_direct_with_options("dotdot-alias-a", 2, true, &alias_a)
            .expect("dot-dot alias a window query should succeed");
        ensure_memory_db_ready(None, &alias_b).expect("ensure dot-dot alias b ready");
        window_direct_with_options("dotdot-alias-b", 2, true, &alias_b)
            .expect("dot-dot alias b window query should succeed");

        assert_eq!(
            sqlite_bootstrap_count_under_prefix_for_tests(&tmp),
            1,
            "expected dot-dot aliases to resolve to one runtime bootstrap"
        );
        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path),
            1,
            "expected normalized bootstrap count to land on the canonical db path"
        );

        drop(_cwd_guard);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_stamps_current_schema_version() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-schema-version-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("schema-version.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config).expect("ensure memory db ready");

        let runtime = acquire_memory_runtime(&config).expect("acquire runtime");
        let user_version = runtime
            .with_connection("test.read_user_version", |conn| {
                conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .map_err(|error| format!("read sqlite user_version failed: {error}"))
            })
            .expect("read sqlite user_version");

        assert_eq!(user_version, SQLITE_MEMORY_SCHEMA_VERSION);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_repairs_session_terminal_outcome_frozen_result_column() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-session-terminal-outcome-frozen-column-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("terminal-outcome.sqlite3");
        let _ = fs::remove_file(&db_path);

        let conn = Connection::open(&db_path).expect("open legacy sqlite db");
        configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
        conn.execute_batch(
            "
            CREATE TABLE turns(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              ts INTEGER NOT NULL
            );
            CREATE TABLE session_terminal_outcomes(
              session_id TEXT PRIMARY KEY,
              status TEXT NOT NULL,
              payload_json TEXT NOT NULL,
              recorded_at INTEGER NOT NULL
            );
            PRAGMA user_version = 9;
            ",
        )
        .expect("create legacy terminal outcome schema");
        drop(conn);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config).expect("repair sqlite db");

        let conn = Connection::open(&db_path).expect("open repaired sqlite db");
        let columns = sqlite_table_columns(&conn, "session_terminal_outcomes")
            .expect("session_terminal_outcomes columns");

        assert!(columns.iter().any(|column| column == "frozen_result_json"));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_repairs_session_event_search_storage() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-session-event-search-storage-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("session-event-search.sqlite3");
        let _ = fs::remove_file(&db_path);

        let conn = Connection::open(&db_path).expect("open legacy sqlite db");
        configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
        conn.execute_batch(
            "
            CREATE TABLE turns(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              ts INTEGER NOT NULL
            );
            CREATE INDEX idx_turns_session_id ON turns(session_id, id);
            CREATE TABLE session_events(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              event_kind TEXT NOT NULL,
              actor_session_id TEXT NULL,
              payload_json TEXT NOT NULL,
              ts INTEGER NOT NULL
            );
            INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, ts)
            VALUES ('root-session', 'memory_indexed', NULL, '{\"summary\":\"中文分词已经启用\"}', 100);
            PRAGMA user_version = 10;
            ",
        )
        .expect("create legacy session event schema");
        drop(conn);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config).expect("repair sqlite db");

        let conn = Connection::open(&db_path).expect("open repaired sqlite db");
        let columns =
            sqlite_table_columns(&conn, "session_events").expect("session_events columns");
        assert!(columns.iter().any(|column| column == "search_text"));

        let search_text = conn
            .query_row(
                "SELECT search_text FROM session_events WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("load session event search_text");
        assert!(search_text.contains("中文"), "search_text={search_text}");
        assert!(search_text.contains("分词"), "search_text={search_text}");

        let match_query = crate::search_text::build_search_fts_query("中文 分词", 6)
            .expect("session event search query");
        let fts_count = conn
            .query_row(
                "SELECT COUNT(*) FROM session_events_fts WHERE session_events_fts MATCH ?1",
                rusqlite::params![match_query],
                |row| row.get::<_, i64>(0),
            )
            .expect("query session event FTS");
        assert_eq!(fts_count, 1);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_repairs_session_tool_consent_mode_check_constraint() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-session-tool-consent-mode-check-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("session-tool-consent.sqlite3");
        let _ = fs::remove_file(&db_path);

        let conn = Connection::open(&db_path).expect("open legacy sqlite db");
        configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
        conn.execute_batch(
            "
            CREATE TABLE turns(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              ts INTEGER NOT NULL
            );
            CREATE INDEX idx_turns_session_id ON turns(session_id, id);
            CREATE TABLE session_tool_consent(
              scope_session_id TEXT PRIMARY KEY,
              mode TEXT NOT NULL,
              updated_by_session_id TEXT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            );
            PRAGMA user_version = 6;
            ",
        )
        .expect("create legacy schema");
        conn.execute(
            "INSERT INTO session_tool_consent(
                scope_session_id,
                mode,
                updated_by_session_id,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "root-session",
                "full",
                "root-session",
                unix_ts_now(),
                unix_ts_now(),
            ],
        )
        .expect("insert legacy session tool consent");
        drop(conn);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("migrate legacy sqlite memory db");

        let repaired_conn = Connection::open(&db_path).expect("open repaired sqlite db");
        let session_tool_consent_sql = repaired_conn
            .query_row(
                "SELECT sql
                 FROM sqlite_master
                 WHERE type = 'table' AND name = 'session_tool_consent'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("read repaired session_tool_consent sql");

        assert!(
            session_tool_consent_sql.contains(SESSION_TOOL_CONSENT_MODE_CHECK_SQL),
            "expected repaired DDL to contain mode check: {session_tool_consent_sql}"
        );

        let invalid_insert = repaired_conn.execute(
            "INSERT INTO session_tool_consent(
                scope_session_id,
                mode,
                updated_by_session_id,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "other-session",
                "bogus",
                "root-session",
                unix_ts_now(),
                unix_ts_now(),
            ],
        );
        assert!(
            invalid_insert.is_err(),
            "invalid mode should be rejected after repair"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn repeated_same_path_runtime_lookup_reuses_normalized_path_alias_cache() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-alias-cache-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        let cwd = tmp.join("workspace").join("nested");
        fs::create_dir_all(&cwd).expect("create nested cwd dir");
        let db_path = tmp
            .join("workspace")
            .join("data")
            .join("alias-cache.sqlite3");
        let _cwd_guard = ScopedCurrentDir::new(&cwd);

        let config = sqlite_test_config_with_profile(
            PathBuf::from("../data/alias-cache.sqlite3"),
            MemoryProfile::WindowOnly,
            2,
        );

        ensure_memory_db_ready(None, &config).expect("ensure alias-cache db ready");

        let _metrics = begin_sqlite_metric_capture_for_tests();
        window_direct_with_options("alias-cache-session", 2, true, &config)
            .expect("first alias-cache window query should succeed");
        window_direct_with_options("alias-cache-session", 2, true, &config)
            .expect("second alias-cache window query should succeed");

        assert_eq!(
            runtime_path_normalization_full_count_for_tests(),
            0,
            "expected repeated same-path runtime lookups to reuse cached normalized path aliases instead of re-running full normalization"
        );
        assert!(
            runtime_path_normalization_alias_hit_count_for_tests() >= 2,
            "expected repeated same-path runtime lookups to hit the normalized path alias cache on each hot-path access"
        );
        assert_eq!(
            sqlite_bootstrap_count_for_tests(&db_path),
            1,
            "expected repeated same-path runtime lookups to reuse the existing cached sqlite runtime"
        );

        drop(_cwd_guard);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn reopening_current_schema_db_skips_metadata_repairs() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-schema-repair-skip-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("schema-repair-skip.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("bootstrap current schema db");
        drop_cached_sqlite_runtime_for_tests(&db_path);

        reset_sqlite_schema_repair_metrics_for_tests();
        ensure_memory_db_ready(Some(db_path.clone()), &config).expect("reopen current schema db");

        assert_eq!(
            sqlite_schema_repair_count_for_tests("turn_session_index"),
            0,
            "expected current-schema reopen to skip turn/session metadata repairs"
        );
        assert_eq!(
            sqlite_schema_repair_count_for_tests("summary_checkpoint_metadata"),
            0,
            "expected current-schema reopen to skip summary checkpoint metadata repairs"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn reopening_current_schema_db_skips_schema_init_batch() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-schema-init-skip-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("schema-init-skip.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("bootstrap current schema db");
        assert_eq!(
            sqlite_schema_init_count_for_tests(&db_path),
            1,
            "expected the initial bootstrap to execute schema initialization exactly once"
        );

        drop_cached_sqlite_runtime_for_tests(&db_path);
        ensure_memory_db_ready(Some(db_path.clone()), &config).expect("reopen current schema db");

        assert_eq!(
            sqlite_schema_init_count_for_tests(&db_path),
            1,
            "expected reopening a current-schema db to skip the unconditional schema initialization batch"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_diagnostics_distinguish_cache_miss_and_hit() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-runtime-diagnostics-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("runtime-diagnostics.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        let (_, cold_bootstrap) =
            ensure_memory_db_ready_with_diagnostics(Some(db_path.clone()), &config)
                .expect("cold bootstrap diagnostics");
        let (_, hot_bootstrap) =
            ensure_memory_db_ready_with_diagnostics(Some(db_path.clone()), &config)
                .expect("hot bootstrap diagnostics");

        assert!(!cold_bootstrap.cache_hit);
        assert!(hot_bootstrap.cache_hit);
        assert_eq!(hot_bootstrap.runtime_create_ms, 0.0);
        assert_eq!(hot_bootstrap.connection_open_ms, 0.0);
        assert_eq!(hot_bootstrap.configure_connection_ms, 0.0);
        assert_eq!(hot_bootstrap.schema_init_ms, 0.0);
        assert_eq!(hot_bootstrap.schema_upgrade_ms, 0.0);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn window_reads_route_through_cached_statement_preparation() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-prepared-window-cache-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("prepared-window.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        window_direct_with_options("prepared-window-session", 2, true, &config)
            .expect("window query should succeed");
        window_direct_with_options("prepared-window-session", 2, true, &config)
            .expect("second window query should succeed");

        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("FROM turns"),
            2,
            "expected hot window query to use cached statement preparation on both executions"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn window_only_context_snapshot_avoids_indexed_recent_turn_query() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-window-only-snapshot-query-shape-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("window-only-snapshot.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        append_turn_direct("window-only-snapshot-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(
            "window-only-snapshot-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct("window-only-snapshot-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let snapshot = load_context_snapshot("window-only-snapshot-session", &config)
            .expect("load context snapshot");

        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT id, role, content\n             FROM turns"
            ),
            1,
            "expected window-only prompt snapshots to use the visible-turn prompt query that can skip internal persisted records without fetching timestamps"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT role, content\n             FROM turns"
            ),
            0,
            "expected window-only prompt snapshots to retire the older lean query shape now that internal records require visible-turn filtering by id"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("SELECT role, content, ts"),
            0,
            "expected window-only prompt snapshots to avoid the full window query shape with timestamps"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_context_snapshot_avoids_indexed_window_materialization_query_shape() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-snapshot-window-query-shape-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-snapshot-window.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-snapshot-query-shape-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-snapshot-query-shape-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-snapshot-query-shape-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        reset_cached_prepare_metrics_for_tests();
        let snapshot = load_context_snapshot("summary-snapshot-query-shape-session", &config)
            .expect("load context snapshot");

        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT turn_count\n             FROM memory_session_state"
            ),
            1,
            "expected summary prompt snapshots to consult per-session turn-count metadata before selecting the active-window query shape"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "LEFT JOIN memory_summary_checkpoints checkpoint"
            ),
            1,
            "expected summary prompt snapshots to co-load the active window and checkpoint metadata once turn-count metadata proves overflow"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT id, role, content\n             FROM turns"
            ),
            0,
            "expected summary prompt snapshots to retire the older id-only active-window query now that checkpoint metadata is folded into the fast path"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
            0,
            "expected summary prompt snapshots to retire the heavier joined turn-count query shape"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
            0,
            "expected summary prompt snapshots to avoid session_turn_index metadata when turn-count metadata can derive the active window boundary"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("SELECT role, content, ts, id"),
            0,
            "expected summary prompt snapshots to avoid the window+boundary query shape that still fetches timestamps"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT role, content, id, session_turn_index"
            ),
            0,
            "expected summary prompt snapshots to retire the older query shape that depended on session_turn_index metadata"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_append_path_routes_multiple_sqls_through_cached_preparation() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-sqlite-prepared-summary-cache-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("prepared-summary.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct("prepared-summary-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("prepared-summary-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("prepared-summary-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");
        append_turn_direct("prepared-summary-session", "assistant", "turn 4", &config)
            .expect("append turn 4 should succeed");

        assert!(
            cached_prepare_count_for_sql_fragment_for_tests("memory_summary_checkpoints") >= 2,
            "expected post-overflow summary append path to route repeated checkpoint SQL through cached preparation without touching summary maintenance before the window overflows"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("SELECT id, role, content, ts"),
            0,
            "expected summary append path to avoid the full indexed active-window query when only the summary boundary id is needed"
        );
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests("session_turn_index <= 2") >= 1,
            "expected first overflow summary append to use the dedicated two-row initial checkpoint query"
        );
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests("LIMIT 2") >= 1,
            "expected first overflow summary append to cap the dedicated initial checkpoint query at the two boundary rows"
        );
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests("AS summary_before_turn_id") >= 1,
            "expected post-initial summary append maintenance to keep routing boundary and checkpoint metadata reads through one append-maintenance state query"
        );
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests("summary_body_bytes") >= 2,
            "expected active summary append maintenance to read persisted summary body bytes metadata instead of recomputing text length inside SQLite"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("LENGTH(CAST(summary_body AS BLOB))"),
            0,
            "expected summary append path to avoid recomputing summary body length inside SQLite"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_append_path_avoids_empty_checkpoint_delete_before_window_overflow() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-append-empty-delete-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-append-empty-delete.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 4, 256);

        append_turn_direct(
            "summary-append-empty-delete-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-append-empty-delete-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");

        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "DELETE FROM memory_summary_checkpoints"
            ),
            0,
            "expected append maintenance to avoid preparing checkpoint delete statements before the active window overflows"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_append_path_skips_summary_maintenance_queries_before_window_overflow() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-append-pre-overflow-maintenance-skip-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-append-pre-overflow-maintenance-skip.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 4, 256);

        append_turn_direct(
            "summary-append-pre-overflow-maintenance-skip-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-append-pre-overflow-maintenance-skip-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");

        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("AS summary_before_turn_id"),
            0,
            "expected pre-overflow appends to skip summary maintenance state queries entirely"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "turns.session_turn_index = state.turn_count - ?2 + 1"
            ),
            0,
            "expected pre-overflow appends to skip summary boundary probes entirely"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_append_hot_path_advances_boundary_without_window_offset_probe() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-append-hot-boundary-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-append-hot-boundary.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-append-hot-boundary-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-append-hot-boundary-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-append-hot-boundary-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "summary-append-hot-boundary-session",
            "assistant",
            "turn 4",
            &config,
        )
        .expect("append turn 4 should succeed");

        let boundary_before = read_summary_checkpoint_boundary_turn_id(
            &config,
            "summary-append-hot-boundary-session",
        )
        .expect("read summary checkpoint boundary after warmup");
        assert_eq!(boundary_before, Some(3));

        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();
        append_turn_direct(
            "summary-append-hot-boundary-session",
            "user",
            "turn 5",
            &config,
        )
        .expect("append turn 5 should succeed");

        assert!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "id > checkpoint.summary_before_turn_id"
            ) >= 1,
            "expected steady-state append to advance the summary boundary from checkpoint metadata instead of re-probing the full window"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("LIMIT 1 OFFSET ?2"),
            0,
            "expected steady-state append to avoid the window-offset boundary probe once checkpoint metadata is available"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
            ),
            0,
            "expected steady-state append to reuse checkpoint metadata already loaded by append maintenance instead of re-querying checkpoint meta"
        );

        let boundary_after = read_summary_checkpoint_boundary_turn_id(
            &config,
            "summary-append-hot-boundary-session",
        )
        .expect("read summary checkpoint boundary after hot append");
        assert_eq!(boundary_after, Some(4));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_append_cold_path_uses_dedicated_initial_checkpoint_query() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-append-cold-boundary-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-append-cold-boundary.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-append-cold-boundary-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-append-cold-boundary-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");

        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();
        append_turn_direct(
            "summary-append-cold-boundary-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "turns.session_turn_index = state.turn_count - ?2 + 1"
            ),
            0,
            "expected first summary checkpoint materialization to bypass the separate boundary lookup by per-session turn-count metadata"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("LIMIT 1 OFFSET ?2"),
            0,
            "expected cold summary boundary lookup to avoid the window-offset probe"
        );
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests("session_turn_index <= 2") >= 1,
            "expected first summary checkpoint materialization to use a dedicated two-row range query"
        );
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests("LIMIT 2") >= 1,
            "expected first summary checkpoint materialization to cap the dedicated range query at the two boundary rows"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
            ),
            0,
            "expected first summary checkpoint materialization to avoid reloading a checkpoint row that append maintenance already knows does not exist"
        );
        assert_eq!(
            summary_streaming_query_count_for_tests("rebuild"),
            0,
            "expected first summary checkpoint materialization to avoid the generic rebuild streaming query"
        );
        assert_eq!(
            summary_row_observed_count_for_tests(),
            1,
            "expected first summary checkpoint materialization to observe exactly one payload row"
        );
        assert_eq!(
            summary_payload_decode_count_for_tests(),
            1,
            "expected first summary checkpoint materialization to decode exactly one payload row"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_rebuild_routes_through_streaming_row_accumulation() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-streaming-rebuild-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-streaming-rebuild.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-streaming-rebuild-session",
            "user",
            "turn 1",
            &config_window_two,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-streaming-rebuild-session",
            "assistant",
            "turn 2",
            &config_window_two,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-streaming-rebuild-session",
            "user",
            "turn 3",
            &config_window_two,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "summary-streaming-rebuild-session",
            "assistant",
            "turn 4",
            &config_window_two,
        )
        .expect("append turn 4 should succeed");

        reset_summary_materialization_metrics_for_tests();
        let config_window_three = MemoryRuntimeConfig {
            sliding_window: 3,
            ..config_window_two
        };
        let _snapshot =
            load_context_snapshot("summary-streaming-rebuild-session", &config_window_three)
                .expect("load context snapshot after window change");

        assert_eq!(
            summary_streaming_query_count_for_tests("rebuild"),
            1,
            "expected rebuild path to route through streaming summary accumulation"
        );
        assert_eq!(
            summary_buffered_query_count_for_tests("rebuild"),
            0,
            "expected rebuild path to stop buffering full turn vectors"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_catch_up_routes_through_streaming_row_accumulation() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-streaming-catch-up-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-streaming-catch-up.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-streaming-catch-up-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-streaming-catch-up-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-streaming-catch-up-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        reset_summary_materialization_metrics_for_tests();
        append_turn_direct(
            "summary-streaming-catch-up-session",
            "assistant",
            "turn 4",
            &config,
        )
        .expect("append turn 4 should succeed");

        assert_eq!(
            summary_streaming_query_count_for_tests("catch_up"),
            1,
            "expected catch-up path to route through streaming summary accumulation"
        );
        assert_eq!(
            summary_buffered_query_count_for_tests("catch_up"),
            0,
            "expected catch-up path to stop buffering delta turn vectors"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_rebuild_skips_summary_formatting_after_budget_saturation() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-saturation-rebuild-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-saturation-rebuild.sqlite3");
        let _ = fs::remove_file(&db_path);

        let first_turn = "FIRST-MARKER ".repeat(40);
        let second_turn = "SECOND-MARKER ".repeat(20);

        let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-saturation-rebuild-session",
            "user",
            &first_turn,
            &config_window_two,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-saturation-rebuild-session",
            "assistant",
            &second_turn,
            &config_window_two,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-saturation-rebuild-session",
            "user",
            "turn 3",
            &config_window_two,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "summary-saturation-rebuild-session",
            "assistant",
            "turn 4",
            &config_window_two,
        )
        .expect("append turn 4 should succeed");
        append_turn_direct(
            "summary-saturation-rebuild-session",
            "user",
            "turn 5",
            &config_window_two,
        )
        .expect("append turn 5 should succeed");

        reset_summary_materialization_metrics_for_tests();
        let config_window_three = MemoryRuntimeConfig {
            sliding_window: 3,
            ..config_window_two
        };
        let snapshot =
            load_context_snapshot("summary-saturation-rebuild-session", &config_window_three)
                .expect("load context snapshot after window change");
        let (summarized_through_turn_id, summary_body, _summary_budget, summary_window_size) =
            read_summary_checkpoint(&config_window_three, "summary-saturation-rebuild-session")
                .expect("summary checkpoint row should exist after rebuild");

        assert_eq!(summarized_through_turn_id, 2);
        assert_eq!(summary_window_size, 3);
        assert_eq!(snapshot.window_turns.len(), 3);
        assert!(summary_body.contains("FIRST-MARKER"));
        assert!(!summary_body.contains("SECOND-MARKER"));
        assert_eq!(
            summary_payload_decode_count_for_tests(),
            1,
            "expected rebuild path to stop decoding role/content after summary saturation"
        );
        assert_eq!(
            summary_normalization_count_for_tests(),
            0,
            "expected rebuild path to avoid scratch normalization before and after summary saturation"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_rebuild_fast_forwards_frontier_after_budget_saturation() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-frontier-fast-forward-rebuild-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-frontier-fast-forward-rebuild.sqlite3");
        let _ = fs::remove_file(&db_path);

        let first_turn = "FIRST-MARKER ".repeat(40);
        let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-frontier-fast-forward-rebuild-session",
            "user",
            &first_turn,
            &config_window_two,
        )
        .expect("append turn 1 should succeed");
        for turn_index in 2..=32 {
            let role = if turn_index % 2 == 0 {
                "assistant"
            } else {
                "user"
            };
            append_turn_direct(
                "summary-frontier-fast-forward-rebuild-session",
                role,
                &format!("turn {turn_index}"),
                &config_window_two,
            )
            .expect("append tail turn should succeed");
        }

        reset_summary_materialization_metrics_for_tests();
        let config_window_four = MemoryRuntimeConfig {
            sliding_window: 4,
            ..config_window_two
        };
        let snapshot = load_context_snapshot(
            "summary-frontier-fast-forward-rebuild-session",
            &config_window_four,
        )
        .expect("load context snapshot after window change");
        let (summarized_through_turn_id, summary_body, _summary_budget, summary_window_size) =
            read_summary_checkpoint(
                &config_window_four,
                "summary-frontier-fast-forward-rebuild-session",
            )
            .expect("summary checkpoint row should exist after rebuild");

        assert_eq!(summarized_through_turn_id, 28);
        assert_eq!(summary_window_size, 4);
        assert_eq!(snapshot.window_turns.len(), 4);
        assert!(summary_body.contains("FIRST-MARKER"));
        assert_eq!(
            summary_payload_decode_count_for_tests(),
            1,
            "expected rebuild path to decode only the first saturating payload"
        );
        assert_eq!(
            summary_row_observed_count_for_tests(),
            1,
            "expected rebuild path to stop streaming rows once the summary budget saturates"
        );
        assert_eq!(
            summary_frontier_probe_count_for_tests("rebuild"),
            1,
            "expected rebuild path to perform one frontier lookup after summary saturation instead of carrying frontier metadata in every streamed row"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_rebuild_load_diagnostics_split_stream_and_checkpoint_upsert_costs() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-rebuild-diagnostics-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-rebuild-diagnostics.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

        for (role, content) in [
            ("user", "turn 1"),
            ("assistant", "turn 2"),
            ("user", "turn 3"),
            ("assistant", "turn 4"),
        ] {
            append_turn_direct(
                "summary-rebuild-diagnostics-session",
                role,
                content,
                &config_window_two,
            )
            .expect("append turn should succeed");
        }

        let config_window_three = MemoryRuntimeConfig {
            sliding_window: 3,
            ..config_window_two
        };
        let (_snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
            "summary-rebuild-diagnostics-session",
            &config_window_three,
        )
        .expect("load context snapshot after window change");

        assert!(
            diagnostics.summary_rebuild_ms > 0.0,
            "expected summary rebuild diagnostics to record total rebuild time"
        );
        assert!(
            diagnostics.summary_rebuild_stream_ms > 0.0,
            "expected summary rebuild diagnostics to split out stream accumulation time"
        );
        assert!(
            diagnostics.summary_rebuild_checkpoint_upsert_ms > 0.0,
            "expected summary rebuild diagnostics to split out checkpoint upsert time"
        );
        assert!(
            diagnostics.summary_rebuild_checkpoint_metadata_upsert_ms > 0.0,
            "expected summary rebuild diagnostics to split out checkpoint metadata upsert time"
        );
        assert!(
            diagnostics.summary_rebuild_checkpoint_body_upsert_ms > 0.0,
            "expected summary rebuild diagnostics to split out checkpoint body upsert time"
        );
        assert!(
            diagnostics.summary_rebuild_checkpoint_commit_ms > 0.0,
            "expected summary rebuild diagnostics to split out checkpoint commit time"
        );
        assert!(
            diagnostics.summary_rebuild_checkpoint_metadata_upsert_ms
                + diagnostics.summary_rebuild_checkpoint_body_upsert_ms
                + diagnostics.summary_rebuild_checkpoint_commit_ms
                <= diagnostics.summary_rebuild_checkpoint_upsert_ms + 1.0,
            "expected checkpoint upsert subphases to stay within the measured checkpoint upsert envelope"
        );
        assert!(
            diagnostics.summary_rebuild_stream_ms
                + diagnostics.summary_rebuild_checkpoint_upsert_ms
                <= diagnostics.summary_rebuild_ms + 1.0,
            "expected rebuild subphases to stay within the measured rebuild envelope"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_catch_up_advances_frontier_after_budget_saturation_without_reformatting() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-saturation-catch-up-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-saturation-catch-up.sqlite3");
        let _ = fs::remove_file(&db_path);

        let first_turn = "FIRST-MARKER ".repeat(40);
        let second_turn = "SECOND-MARKER ".repeat(20);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-saturation-catch-up-session",
            "user",
            &first_turn,
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-saturation-catch-up-session",
            "assistant",
            &second_turn,
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-saturation-catch-up-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        let (through_before, summary_before, _budget_before, window_before) =
            read_summary_checkpoint(&config, "summary-saturation-catch-up-session")
                .expect("summary checkpoint should exist before catch-up");
        assert_eq!(through_before, 1);
        assert_eq!(window_before, 2);
        assert!(summary_before.contains("FIRST-MARKER"));
        assert!(!summary_before.contains("SECOND-MARKER"));

        reset_summary_materialization_metrics_for_tests();
        append_turn_direct(
            "summary-saturation-catch-up-session",
            "assistant",
            "turn 4",
            &config,
        )
        .expect("append turn 4 should succeed");

        let (through_after, summary_after, _budget_after, window_after) =
            read_summary_checkpoint(&config, "summary-saturation-catch-up-session")
                .expect("summary checkpoint should exist after catch-up");

        assert_eq!(through_after, 2);
        assert_eq!(window_after, 2);
        assert_eq!(summary_after, summary_before);
        assert_eq!(
            summary_payload_decode_count_for_tests(),
            0,
            "expected catch-up to advance the frontier without decoding saturated summary payloads"
        );
        assert_eq!(
            summary_normalization_count_for_tests(),
            0,
            "expected catch-up to advance the frontier without normalizing saturated summary payloads"
        );
        assert_eq!(
            summary_streaming_query_count_for_tests("catch_up"),
            0,
            "expected saturated append maintenance to skip catch-up streaming when the summary body is already at budget"
        );
        assert_eq!(
            summary_frontier_probe_count_for_tests("catch_up"),
            0,
            "expected catch-up to avoid a separate frontier lookup once streaming rows carry the session max id"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_window_shrink_catch_up_avoids_scratch_normalization_buffer() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-fused-append-rebuild-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-fused-append-rebuild.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-fused-append-rebuild-session",
            "user",
            "turn 1",
            &config_window_two,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-fused-append-rebuild-session",
            "assistant",
            "turn 2",
            &config_window_two,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-fused-append-rebuild-session",
            "user",
            "turn 3",
            &config_window_two,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "summary-fused-append-rebuild-session",
            "assistant",
            "turn 4",
            &config_window_two,
        )
        .expect("append turn 4 should succeed");

        reset_summary_materialization_metrics_for_tests();
        let config_window_one = MemoryRuntimeConfig {
            sliding_window: 1,
            ..config_window_two
        };
        let snapshot =
            load_context_snapshot("summary-fused-append-rebuild-session", &config_window_one)
                .expect("load context snapshot after window change");

        assert_eq!(summary_streaming_query_count_for_tests("rebuild"), 0);
        assert_eq!(summary_streaming_query_count_for_tests("catch_up"), 1);
        assert_eq!(
            summary_normalization_count_for_tests(),
            0,
            "expected window shrink catch-up path to stop materializing scratch normalization buffers"
        );
        assert_eq!(snapshot.window_turns.len(), 1);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_catch_up_avoids_scratch_normalization_buffer() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-fused-append-catch-up-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-fused-append-catch-up.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "summary-fused-append-catch-up-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "summary-fused-append-catch-up-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "summary-fused-append-catch-up-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        reset_summary_materialization_metrics_for_tests();
        append_turn_direct(
            "summary-fused-append-catch-up-session",
            "assistant",
            "turn 4",
            &config,
        )
        .expect("append turn 4 should succeed");

        assert_eq!(summary_streaming_query_count_for_tests("catch_up"), 1);
        assert_eq!(
            summary_normalization_count_for_tests(),
            0,
            "expected catch-up path to stop materializing scratch normalization buffers"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn append_summary_line_preserves_whitespace_collapse_and_utf8_safe_truncation() {
        let mut summary_body = String::new();

        append_summary_line(
            &mut summary_body,
            "user",
            " \n 你好\t世界  again \r\n next ",
            19,
        );

        assert_eq!(summary_body, "- user: 你好 世");
    }

    #[test]
    fn context_snapshot_separates_materialized_summary_from_active_window() {
        let tmp =
            std::env::temp_dir().join(format!("loong-context-snapshot-{}", std::process::id()));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("context-snapshot.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config =
            sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

        append_turn_direct("snapshot-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("snapshot-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("snapshot-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");
        append_turn_direct("snapshot-session", "assistant", "turn 4", &config)
            .expect("append turn 4 should succeed");

        let snapshot =
            load_context_snapshot("snapshot-session", &config).expect("load context snapshot");

        assert!(
            snapshot
                .summary_body
                .as_deref()
                .is_some_and(|summary| summary.contains("turn 1")),
            "expected summary body to include turn 1"
        );
        assert!(
            snapshot
                .summary_body
                .as_deref()
                .is_some_and(|summary| summary.contains("turn 2")),
            "expected summary body to include turn 2"
        );

        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(snapshot.window_turns[0].content, "turn 3");
        assert_eq!(snapshot.window_turns[1].content, "turn 4");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn append_turn_materializes_summary_checkpoint_once_window_overflows() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-materialized-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary-checkpoint.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct("checkpoint-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("checkpoint-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("checkpoint-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");
        append_turn_direct("checkpoint-session", "assistant", "turn 4", &config)
            .expect("append turn 4 should succeed");

        let (summarized_through_turn_id, summary_body, summary_budget_chars, summary_window_size) =
            read_summary_checkpoint(&config, "checkpoint-session")
                .expect("summary checkpoint row should exist");

        assert_eq!(summarized_through_turn_id, 2);
        assert!(summary_body.contains("turn 1"));
        assert!(summary_body.contains("turn 2"));
        assert_eq!(summary_budget_chars, 256);
        assert_eq!(summary_window_size, 2);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn initial_summary_checkpoint_waits_for_visible_overflow() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-initial-summary-visible-overflow-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("initial-summary-visible-overflow.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);
        let hidden_turn = crate::memory::build_conversation_event_content(
            "provider_prompt_frame_snapshot",
            serde_json::json!({"phase": "initial"}),
        );

        append_turn_direct(
            "initial-summary-visible-overflow",
            "user",
            "visible 1",
            &config,
        )
        .expect("append visible turn 1 should succeed");
        append_turn_direct(
            "initial-summary-visible-overflow",
            "assistant",
            hidden_turn.as_str(),
            &config,
        )
        .expect("append hidden prompt-frame turn should succeed");
        append_turn_direct(
            "initial-summary-visible-overflow",
            "assistant",
            "visible 2",
            &config,
        )
        .expect("append visible turn 2 should succeed");

        assert_eq!(
            count_summary_checkpoints(&config, "initial-summary-visible-overflow")
                .expect("count checkpoints after raw overflow"),
            0,
            "raw overflow without visible overflow must not materialize a checkpoint",
        );

        let pre_overflow_snapshot =
            load_context_snapshot("initial-summary-visible-overflow", &config)
                .expect("load pre-overflow context snapshot");
        let pre_overflow_contents = pre_overflow_snapshot
            .window_turns
            .iter()
            .map(|turn| turn.content.as_str())
            .collect::<Vec<_>>();

        assert_eq!(pre_overflow_contents, vec!["visible 1", "visible 2"]);
        assert!(pre_overflow_snapshot.summary_body.is_none());

        append_turn_direct(
            "initial-summary-visible-overflow",
            "user",
            "visible 3",
            &config,
        )
        .expect("append visible turn 3 should succeed");

        assert_eq!(
            count_summary_checkpoints(&config, "initial-summary-visible-overflow")
                .expect("count checkpoints after visible overflow"),
            1,
            "checkpoint should materialize once the visible window actually overflows",
        );

        let post_overflow_snapshot =
            load_context_snapshot("initial-summary-visible-overflow", &config)
                .expect("load post-overflow context snapshot");
        let post_overflow_contents = post_overflow_snapshot
            .window_turns
            .iter()
            .map(|turn| turn.content.as_str())
            .collect::<Vec<_>>();

        assert_eq!(post_overflow_contents, vec!["visible 2", "visible 3"]);
        assert!(post_overflow_snapshot.summary_body.is_some());

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn load_context_snapshot_rebuilds_materialized_summary_when_window_size_changes() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-window-rebuild-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary-window-rebuild.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "window-rebuild-session",
            "user",
            "turn 1",
            &config_window_two,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "window-rebuild-session",
            "assistant",
            "turn 2",
            &config_window_two,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "window-rebuild-session",
            "user",
            "turn 3",
            &config_window_two,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "window-rebuild-session",
            "assistant",
            "turn 4",
            &config_window_two,
        )
        .expect("append turn 4 should succeed");

        let config_window_three = MemoryRuntimeConfig {
            sliding_window: 3,
            ..config_window_two
        };
        let snapshot = load_context_snapshot("window-rebuild-session", &config_window_three)
            .expect("load context snapshot after window change");
        let (summarized_through_turn_id, summary_body, _summary_budget_chars, summary_window_size) =
            read_summary_checkpoint(&config_window_three, "window-rebuild-session")
                .expect("summary checkpoint row should exist after rebuild");

        assert_eq!(summarized_through_turn_id, 1);
        assert!(summary_body.contains("turn 1"));
        assert!(!summary_body.contains("turn 2"));
        assert_eq!(summary_window_size, 3);
        assert_eq!(snapshot.window_turns.len(), 3);
        assert_eq!(snapshot.window_turns[0].content, "turn 2");
        assert_eq!(snapshot.window_turns[2].content, "turn 4");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn load_context_snapshot_updates_checkpoint_metadata_without_rewriting_body_when_frontier_is_stable()
     {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-window-metadata-only-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-window-metadata-only.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);
        let mut config_window_only =
            sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);
        config_window_only.summary_max_chars = 256;
        let config_window_three = MemoryRuntimeConfig {
            sliding_window: 3,
            ..config_window_two.clone()
        };

        for (role, content) in [
            ("user", "turn 1"),
            ("assistant", "turn 2"),
            ("user", "turn 3"),
            ("assistant", "turn 4"),
            ("user", "turn 5"),
        ] {
            append_turn_direct(
                "window-metadata-only-session",
                role,
                content,
                &config_window_two,
            )
            .expect("append turn under summary config should succeed");
        }

        let (through_before, summary_before, _budget_before, window_before) =
            read_summary_checkpoint(&config_window_two, "window-metadata-only-session")
                .expect("summary checkpoint should exist before metadata-only update");
        assert_eq!(through_before, 3);
        assert!(summary_before.contains("turn 1"));
        assert!(summary_before.contains("turn 3"));
        assert_eq!(window_before, 2);

        append_turn_direct(
            "window-metadata-only-session",
            "assistant",
            "turn 6",
            &config_window_only,
        )
        .expect("append turn under window-only config should succeed");

        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();
        let snapshot = load_context_snapshot("window-metadata-only-session", &config_window_three)
            .expect("load context snapshot after metadata-only window drift");
        let (through_after, summary_after, _budget_after, window_after) =
            read_summary_checkpoint(&config_window_three, "window-metadata-only-session")
                .expect("summary checkpoint should exist after metadata-only update");

        assert_eq!(through_after, 3);
        assert_eq!(summary_after, summary_before);
        assert_eq!(window_after, 3);
        assert_eq!(
            snapshot.summary_body.as_deref(),
            Some(summary_before.as_str())
        );
        assert_eq!(snapshot.window_turns.len(), 3);
        assert_eq!(snapshot.window_turns[0].content, "turn 4");
        assert_eq!(snapshot.window_turns[2].content, "turn 6");
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
            ) >= 1,
            "expected metadata-only window drift to hydrate the persisted summary through the detached checkpoint body table"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("RETURNING summary_body"),
            0,
            "expected metadata-only window drift to avoid UPDATE ... RETURNING body hydration after splitting summary storage"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_context_snapshot_uses_compatible_checkpoint_body_fast_path() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-compatible-fast-path-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-compatible-fast-path.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        for (role, content) in [
            ("user", "turn 1"),
            ("assistant", "turn 2"),
            ("user", "turn 3"),
            ("assistant", "turn 4"),
        ] {
            append_turn_direct(
                "summary-compatible-fast-path-session",
                role,
                content,
                &config,
            )
            .expect("append turn under summary config should succeed");
        }

        let first_snapshot = load_context_snapshot("summary-compatible-fast-path-session", &config)
            .expect("initial summary snapshot should succeed");
        assert!(first_snapshot.summary_body.is_some());
        assert_eq!(first_snapshot.window_turns.len(), 2);

        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();
        let second_snapshot =
            load_context_snapshot("summary-compatible-fast-path-session", &config)
                .expect("compatible summary snapshot should succeed");

        assert_eq!(second_snapshot.summary_body, first_snapshot.summary_body);
        assert_eq!(second_snapshot.window_turns.len(), 2);
        assert_eq!(second_snapshot.window_turns[0].content, "turn 3");
        assert_eq!(second_snapshot.window_turns[1].content, "turn 4");
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
            ) >= 1,
            "expected a fully compatible summary snapshot to load the checkpoint body from the detached body table"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("RETURNING summary_body"),
            0,
            "expected a fully compatible summary snapshot to avoid metadata repair writes"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_context_snapshot_uses_catch_up_when_window_shrinks() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-window-shrink-catch-up-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-window-shrink-catch-up.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_window_three = sqlite_test_summary_config(db_path.clone(), 3, 256);

        append_turn_direct(
            "window-shrink-catch-up-session",
            "user",
            "turn 1",
            &config_window_three,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "window-shrink-catch-up-session",
            "assistant",
            "turn 2",
            &config_window_three,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "window-shrink-catch-up-session",
            "user",
            "turn 3",
            &config_window_three,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "window-shrink-catch-up-session",
            "assistant",
            "turn 4",
            &config_window_three,
        )
        .expect("append turn 4 should succeed");
        append_turn_direct(
            "window-shrink-catch-up-session",
            "user",
            "turn 5",
            &config_window_three,
        )
        .expect("append turn 5 should succeed");

        let (through_before, summary_before, _budget_before, window_before) =
            read_summary_checkpoint(&config_window_three, "window-shrink-catch-up-session")
                .expect("summary checkpoint should exist before shrink");
        assert_eq!(through_before, 2);
        assert_eq!(window_before, 3);
        assert!(summary_before.contains("turn 1"));
        assert!(summary_before.contains("turn 2"));

        reset_summary_materialization_metrics_for_tests();
        let config_window_two = MemoryRuntimeConfig {
            sliding_window: 2,
            ..config_window_three
        };
        let snapshot = load_context_snapshot("window-shrink-catch-up-session", &config_window_two)
            .expect("load context snapshot after shrinking window");
        let (through_after, summary_after, _budget_after, window_after) =
            read_summary_checkpoint(&config_window_two, "window-shrink-catch-up-session")
                .expect("summary checkpoint should exist after shrink");

        assert_eq!(through_after, 3);
        assert_eq!(window_after, 2);
        assert!(summary_after.contains("turn 3"));
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(snapshot.window_turns[0].content, "turn 4");
        assert_eq!(snapshot.window_turns[1].content, "turn 5");
        assert_eq!(
            summary_streaming_query_count_for_tests("rebuild"),
            0,
            "expected shrink path to avoid full rebuild when existing checkpoint can catch up"
        );
        assert_eq!(
            summary_streaming_query_count_for_tests("catch_up"),
            1,
            "expected shrink path to extend the existing checkpoint incrementally"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_context_snapshot_catch_up_probes_frontier_when_saturated() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-window-shrink-saturated-catch-up-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("summary-window-shrink-saturated-catch-up.sqlite3");
        let _ = fs::remove_file(&db_path);

        let first_turn = "FIRST-MARKER ".repeat(40);
        let second_turn = "SECOND-MARKER ".repeat(20);
        let config_window_three = sqlite_test_summary_config(db_path.clone(), 3, 256);

        append_turn_direct(
            "window-shrink-saturated-catch-up-session",
            "user",
            &first_turn,
            &config_window_three,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "window-shrink-saturated-catch-up-session",
            "assistant",
            &second_turn,
            &config_window_three,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "window-shrink-saturated-catch-up-session",
            "user",
            "turn 3",
            &config_window_three,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "window-shrink-saturated-catch-up-session",
            "assistant",
            "turn 4",
            &config_window_three,
        )
        .expect("append turn 4 should succeed");
        append_turn_direct(
            "window-shrink-saturated-catch-up-session",
            "user",
            "turn 5",
            &config_window_three,
        )
        .expect("append turn 5 should succeed");

        let (through_before, summary_before, _budget_before, window_before) =
            read_summary_checkpoint(
                &config_window_three,
                "window-shrink-saturated-catch-up-session",
            )
            .expect("summary checkpoint should exist before shrink");
        assert_eq!(through_before, 2);
        assert_eq!(window_before, 3);
        assert!(summary_before.contains("FIRST-MARKER"));
        assert!(!summary_before.contains("SECOND-MARKER"));

        reset_summary_materialization_metrics_for_tests();
        let config_window_two = MemoryRuntimeConfig {
            sliding_window: 2,
            ..config_window_three
        };
        let snapshot = load_context_snapshot(
            "window-shrink-saturated-catch-up-session",
            &config_window_two,
        )
        .expect("load context snapshot after shrinking saturated window");
        let (through_after, summary_after, _budget_after, window_after) = read_summary_checkpoint(
            &config_window_two,
            "window-shrink-saturated-catch-up-session",
        )
        .expect("summary checkpoint should exist after saturated shrink catch-up");

        assert_eq!(through_after, 3);
        assert_eq!(window_after, 2);
        assert_eq!(summary_after, summary_before);
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(
            summary_streaming_query_count_for_tests("catch_up"),
            1,
            "expected saturated shrink path to run one catch-up stream"
        );
        assert_eq!(
            summary_payload_decode_count_for_tests(),
            0,
            "expected saturated shrink catch-up to avoid decoding additional payload fragments"
        );
        assert_eq!(
            summary_frontier_probe_count_for_tests("catch_up"),
            1,
            "expected saturated catch-up to use one frontier probe after summary saturation"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn recent_turn_query_with_boundary_id_returns_oldest_active_window_turn() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-window-boundary-query-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("window-boundary.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

        append_turn_direct("window-boundary-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("window-boundary-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("window-boundary-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let runtime = acquire_memory_runtime(&config).expect("acquire runtime");
        let recent_window = runtime
            .with_connection("test.query_recent_turns_with_boundary_id", |conn| {
                query_recent_turns_with_boundary_id(conn, "window-boundary-session", 2)
            })
            .expect("query turns with boundary id");

        assert_eq!(recent_window.turns.len(), 2);
        assert_eq!(recent_window.turns[0].content, "turn 2");
        assert_eq!(recent_window.turns[1].content, "turn 3");
        assert_eq!(recent_window.summary_before_turn_id, Some(2));
        assert!(
            !recent_window.window_starts_at_session_origin,
            "expected a three-turn session with a two-turn window to preserve older-turn context"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_context_snapshot_rebuilds_materialized_summary_when_budget_changes() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-budget-rebuild-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary-budget-rebuild.sqlite3");
        let _ = fs::remove_file(&db_path);

        let first_turn = "alpha ".repeat(40);
        let second_turn = "SECOND-MARKER ".repeat(8);

        let config_small_budget = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "budget-rebuild-session",
            "user",
            &first_turn,
            &config_small_budget,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "budget-rebuild-session",
            "assistant",
            &second_turn,
            &config_small_budget,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "budget-rebuild-session",
            "user",
            "turn 3",
            &config_small_budget,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "budget-rebuild-session",
            "assistant",
            "turn 4",
            &config_small_budget,
        )
        .expect("append turn 4 should succeed");

        let (_through_small, small_summary_body, small_budget, _window_small) =
            read_summary_checkpoint(&config_small_budget, "budget-rebuild-session")
                .expect("small-budget checkpoint should exist");
        assert_eq!(small_budget, 256);

        let config_large_budget = MemoryRuntimeConfig {
            summary_max_chars: 512,
            ..config_small_budget
        };
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();
        let snapshot = load_context_snapshot("budget-rebuild-session", &config_large_budget)
            .expect("load context snapshot after budget change");
        let (_through_large, large_summary_body, large_budget, _window_large) =
            read_summary_checkpoint(&config_large_budget, "budget-rebuild-session")
                .expect("large-budget checkpoint should exist");

        assert_eq!(large_budget, 512);
        assert!(small_summary_body.len() <= 256);
        assert!(!small_summary_body.contains("SECOND-MARKER"));
        assert!(large_summary_body.contains("SECOND-MARKER"));
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
            ),
            0,
            "expected budget-change rebuild to avoid a second checkpoint metadata lookup after the known-overflow window query already loaded that metadata"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "LEFT JOIN memory_summary_checkpoints checkpoint"
            ),
            1,
            "expected budget-change rebuild to co-load checkpoint metadata with the known-overflow window query"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
            ),
            0,
            "expected budget-change rebuild to avoid loading the existing summary body when metadata already proves a rebuild is required"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn load_context_snapshot_skips_rebuild_when_budget_changes_but_summary_is_unsaturated() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_summary_materialization_metrics_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-budget-metadata-only-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary-budget-metadata-only.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_small_budget = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "budget-metadata-only-session",
            "user",
            "turn 1",
            &config_small_budget,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "budget-metadata-only-session",
            "assistant",
            "turn 2",
            &config_small_budget,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "budget-metadata-only-session",
            "user",
            "turn 3",
            &config_small_budget,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "budget-metadata-only-session",
            "assistant",
            "turn 4",
            &config_small_budget,
        )
        .expect("append turn 4 should succeed");

        let (_through_small, small_summary_body, small_budget, _window_small) =
            read_summary_checkpoint(&config_small_budget, "budget-metadata-only-session")
                .expect("small-budget checkpoint should exist");
        assert_eq!(small_budget, 256);
        assert!(small_summary_body.len() < 256);

        let config_large_budget = MemoryRuntimeConfig {
            summary_max_chars: 512,
            ..config_small_budget
        };
        reset_cached_prepare_metrics_for_tests();
        reset_summary_materialization_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();
        let snapshot = load_context_snapshot("budget-metadata-only-session", &config_large_budget)
            .expect("load context snapshot after unsaturated budget change");
        let (_through_large, large_summary_body, large_budget, _window_large) =
            read_summary_checkpoint(&config_large_budget, "budget-metadata-only-session")
                .expect("large-budget checkpoint should exist");

        assert_eq!(large_budget, 512);
        assert_eq!(large_summary_body, small_summary_body);
        assert_eq!(
            snapshot.summary_body.as_deref(),
            Some(small_summary_body.as_str())
        );
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(
            summary_streaming_query_count_for_tests("rebuild"),
            0,
            "expected unsaturated budget changes to avoid a full summary rebuild when the existing checkpoint already covers all summarized turns"
        );
        assert!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
            ) >= 1,
            "expected unsaturated budget changes to load the reusable summary body from the detached body table"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("RETURNING summary_body"),
            0,
            "expected unsaturated budget changes to avoid UPDATE ... RETURNING body hydration after splitting summary storage"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn load_context_snapshot_diagnostics_identify_metadata_only_budget_change_fast_path() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-budget-load-diagnostics-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary-budget-load-diagnostics.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config_small_budget = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "budget-load-diagnostics-session",
            "user",
            "turn 1",
            &config_small_budget,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "budget-load-diagnostics-session",
            "assistant",
            "turn 2",
            &config_small_budget,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "budget-load-diagnostics-session",
            "user",
            "turn 3",
            &config_small_budget,
        )
        .expect("append turn 3 should succeed");
        append_turn_direct(
            "budget-load-diagnostics-session",
            "assistant",
            "turn 4",
            &config_small_budget,
        )
        .expect("append turn 4 should succeed");

        let (_through_small, small_summary_body, _small_budget, _window_small) =
            read_summary_checkpoint(&config_small_budget, "budget-load-diagnostics-session")
                .expect("small-budget checkpoint should exist");
        assert!(small_summary_body.len() < 256);

        let config_large_budget = MemoryRuntimeConfig {
            summary_max_chars: 512,
            ..config_small_budget
        };
        let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
            "budget-load-diagnostics-session",
            &config_large_budget,
        )
        .expect("load context snapshot diagnostics after unsaturated budget change");

        assert_eq!(
            snapshot.summary_body.as_deref(),
            Some(small_summary_body.as_str())
        );
        assert_eq!(snapshot.window_turns.len(), 2);
        assert!(diagnostics.window_query_ms > 0.0);
        assert_eq!(
            diagnostics.summary_checkpoint_meta_query_ms, 0.0,
            "expected known-overflow diagnostics to fold checkpoint metadata into the window query instead of issuing a second metadata lookup"
        );
        assert!(
            diagnostics.summary_checkpoint_metadata_update_ms > 0.0,
            "expected metadata-only budget change to reuse the loaded checkpoint body and finish with a metadata-only UPDATE"
        );
        assert!(
            diagnostics.summary_checkpoint_body_load_ms > 0.0,
            "expected metadata-only budget change to load the reusable checkpoint body separately from the metadata query"
        );
        assert_eq!(
            diagnostics.summary_checkpoint_metadata_update_returning_body_ms,
            0.0
        );
        assert_eq!(diagnostics.summary_rebuild_ms, 0.0);
        assert_eq!(diagnostics.summary_catch_up_ms, 0.0);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn load_context_snapshot_skips_redundant_meta_query_when_window_probe_already_proves_checkpoint_absent()
     {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-known-absent-checkpoint-meta-query-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("known-absent-checkpoint-meta-query.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        for (role, content) in [
            ("user", "turn 1"),
            ("assistant", "turn 2"),
            ("user", "turn 3"),
            ("assistant", "turn 4"),
        ] {
            append_turn_direct(
                "known-absent-checkpoint-meta-query-session",
                role,
                content,
                &config,
            )
            .expect("append turn should succeed");
        }

        assert_eq!(
            count_summary_checkpoints(&config, "known-absent-checkpoint-meta-query-session")
                .expect("count summary checkpoints before deletion"),
            1,
            "expected overflowing append path to materialize a checkpoint before the test deletes it"
        );

        let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
        runtime
            .with_connection_mut("test.delete_summary_checkpoint_before_rebuild", |conn| {
                delete_summary_checkpoint(conn, "known-absent-checkpoint-meta-query-session")
            })
            .expect("delete summary checkpoint before rebuild");

        assert_eq!(
            count_summary_checkpoints(&config, "known-absent-checkpoint-meta-query-session")
                .expect("count summary checkpoints after deletion"),
            0
        );

        let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
            "known-absent-checkpoint-meta-query-session",
            &config,
        )
        .expect("load context snapshot after deleting checkpoint");

        assert!(snapshot.summary_body.is_some());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert!(diagnostics.window_query_ms > 0.0);
        assert_eq!(
            diagnostics.summary_checkpoint_meta_query_ms, 0.0,
            "expected known-overflow window probe to carry enough checkpoint absence state to avoid a second metadata lookup before rebuild"
        );
        assert!(
            diagnostics.summary_rebuild_ms > 0.0,
            "expected rebuild path to recreate the missing checkpoint"
        );
        assert_eq!(
            count_summary_checkpoints(&config, "known-absent-checkpoint-meta-query-session")
                .expect("count summary checkpoints after rebuild"),
            1
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn load_context_snapshot_diagnostics_split_exact_window_query_costs() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-exact-window-query-diagnostics-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("exact-window-query-diagnostics.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "exact-window-query-diagnostics-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "exact-window-query-diagnostics-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");

        let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
            "exact-window-query-diagnostics-session",
            &config,
        )
        .expect("load exact-window snapshot diagnostics");

        assert!(snapshot.summary_body.is_none());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert!(diagnostics.window_query_ms > 0.0);
        assert!(diagnostics.window_turn_count_query_ms > 0.0);
        assert!(diagnostics.window_exact_rows_query_ms > 0.0);
        assert_eq!(diagnostics.window_known_overflow_rows_query_ms, 0.0);
        assert_eq!(diagnostics.window_fallback_rows_query_ms, 0.0);
        assert!(
            diagnostics.window_turn_count_query_ms + diagnostics.window_exact_rows_query_ms
                <= diagnostics.window_query_ms + 1.0
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_context_snapshot_diagnostics_split_known_overflow_window_query_costs() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-known-overflow-window-query-diagnostics-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("known-overflow-window-query-diagnostics.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "known-overflow-window-query-diagnostics-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "known-overflow-window-query-diagnostics-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "known-overflow-window-query-diagnostics-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
            "known-overflow-window-query-diagnostics-session",
            &config,
        )
        .expect("load known-overflow snapshot diagnostics");

        assert!(snapshot.summary_body.is_some());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert!(diagnostics.window_query_ms > 0.0);
        assert!(diagnostics.window_turn_count_query_ms > 0.0);
        assert!(diagnostics.window_known_overflow_rows_query_ms > 0.0);
        assert_eq!(diagnostics.window_exact_rows_query_ms, 0.0);
        assert_eq!(diagnostics.window_fallback_rows_query_ms, 0.0);
        assert_eq!(diagnostics.summary_checkpoint_meta_query_ms, 0.0);
        assert!(
            diagnostics.window_turn_count_query_ms
                + diagnostics.window_known_overflow_rows_query_ms
                <= diagnostics.window_query_ms + 1.0
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_context_snapshot_diagnostics_split_fallback_window_query_costs_when_turn_count_is_missing()
     {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-fallback-window-query-diagnostics-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("fallback-window-query-diagnostics.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "fallback-window-query-diagnostics-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "fallback-window-query-diagnostics-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "fallback-window-query-diagnostics-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
        runtime
            .with_connection_mut(
                "test.delete_turn_count_before_fallback_diagnostics",
                |conn| {
                    conn.execute(
                        "DELETE FROM memory_session_state
                     WHERE session_id = ?1",
                        rusqlite::params!["fallback-window-query-diagnostics-session"],
                    )
                    .map_err(|error| format!("delete session turn count failed: {error}"))?;
                    Ok(())
                },
            )
            .expect("delete turn count before fallback diagnostics");

        let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
            "fallback-window-query-diagnostics-session",
            &config,
        )
        .expect("load fallback snapshot diagnostics");

        assert!(snapshot.summary_body.is_some());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert!(diagnostics.window_query_ms > 0.0);
        assert!(diagnostics.window_turn_count_query_ms > 0.0);
        assert!(diagnostics.window_fallback_rows_query_ms > 0.0);
        assert_eq!(diagnostics.window_exact_rows_query_ms, 0.0);
        assert_eq!(diagnostics.window_known_overflow_rows_query_ms, 0.0);
        assert!(
            diagnostics.summary_checkpoint_meta_query_ms > 0.0,
            "expected fallback window diagnostics to keep the checkpoint metadata lookup once the initial window probe lacks checkpoint state"
        );
        assert!(
            diagnostics.window_turn_count_query_ms + diagnostics.window_fallback_rows_query_ms
                <= diagnostics.window_query_ms + 1.0
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clear_session_removes_materialized_summary_checkpoint() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-clear-session-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary-clear-session.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct("clear-checkpoint-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("clear-checkpoint-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("clear-checkpoint-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let before_clear = count_summary_checkpoints(&config, "clear-checkpoint-session")
            .expect("count checkpoint rows before clear");
        assert_eq!(before_clear, 1);

        clear_session(
            MemoryCoreRequest {
                operation: MEMORY_OP_CLEAR_SESSION.to_owned(),
                payload: json!({
                    "session_id": "clear-checkpoint-session",
                }),
            },
            &config,
        )
        .expect("clear session should succeed");

        let after_clear = count_summary_checkpoints(&config, "clear-checkpoint-session")
            .expect("count checkpoint rows after clear");
        assert_eq!(after_clear, 0);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_migrates_legacy_summary_checkpoint_body_bytes() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-summary-checkpoint-legacy-migration-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("legacy-summary-checkpoint.sqlite3");
        let _ = fs::remove_file(&db_path);

        let conn = Connection::open(&db_path).expect("open legacy sqlite db");
        configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
        conn.execute_batch(
            "
            CREATE TABLE turns(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              ts INTEGER NOT NULL
            );
            CREATE INDEX idx_turns_session_id ON turns(session_id, id);
            CREATE TABLE memory_summary_checkpoints(
              session_id TEXT PRIMARY KEY,
              summarized_through_turn_id INTEGER NOT NULL,
              summary_body TEXT NOT NULL,
              summary_budget_chars INTEGER NOT NULL,
              summary_window_size INTEGER NOT NULL,
              summary_format_version INTEGER NOT NULL,
              updated_at_ts INTEGER NOT NULL
            );
            ",
        )
        .expect("create legacy schema");
        for (role, content) in [
            ("user", "turn 1"),
            ("assistant", "turn 2"),
            ("user", "turn 3"),
            ("assistant", "turn 4"),
            ("user", "turn 5"),
            ("assistant", "turn 6"),
            ("user", "turn 7"),
            ("assistant", "turn 8"),
            ("user", "turn 9"),
            ("assistant", "turn 10"),
        ] {
            conn.execute(
                "INSERT INTO turns(session_id, role, content, ts) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["legacy-session", role, content, unix_ts_now()],
            )
            .expect("insert legacy turn");
        }
        conn.execute(
            "INSERT INTO memory_summary_checkpoints(
                session_id,
                summarized_through_turn_id,
                summary_body,
                summary_budget_chars,
                summary_window_size,
                summary_format_version,
                updated_at_ts
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "legacy-session",
                7_i64,
                "legacy summary body",
                256_i64,
                4_i64,
                SUMMARY_FORMAT_VERSION,
                unix_ts_now(),
            ],
        )
        .expect("insert legacy checkpoint");
        drop(conn);

        let config = sqlite_test_summary_config(db_path.clone(), 4, 256);

        ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("migrate legacy sqlite memory db");

        let runtime = acquire_memory_runtime(&config).expect("acquire migrated runtime");
        let (summary_body_bytes, summary_before_turn_id) = runtime
            .with_connection("test.read_summary_checkpoint_metadata", |conn| {
                conn.query_row(
                    "SELECT summary_body_bytes, summary_before_turn_id
                     FROM memory_summary_checkpoints
                     WHERE session_id = ?1",
                    rusqlite::params!["legacy-session"],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
                )
                .map_err(|error| {
                    format!("read migrated summary checkpoint metadata failed: {error}")
                })
            })
            .expect("read migrated summary checkpoint metadata");
        let migrated_summary_body = runtime
            .with_connection("test.read_summary_checkpoint_body", |conn| {
                conn.query_row(
                    "SELECT summary_body
                     FROM memory_summary_checkpoint_bodies
                     WHERE session_id = ?1",
                    rusqlite::params!["legacy-session"],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| format!("read migrated summary checkpoint body failed: {error}"))
            })
            .expect("read migrated summary checkpoint body");
        let checkpoint_columns = runtime
            .with_connection("test.read_summary_checkpoint_column_order", |conn| {
                let mut stmt = conn
                    .prepare("PRAGMA table_info(memory_summary_checkpoints)")
                    .map_err(|error| {
                        format!("prepare summary checkpoint table info query failed: {error}")
                    })?;
                let mut rows = stmt.query([]).map_err(|error| {
                    format!("query summary checkpoint table info failed: {error}")
                })?;
                let mut names = Vec::new();
                while let Some(row) = rows.next().map_err(|error| {
                    format!("read summary checkpoint table info row failed: {error}")
                })? {
                    names.push(row.get::<_, String>(1).map_err(|error| {
                        format!("decode summary checkpoint table info column failed: {error}")
                    })?);
                }
                Ok(names)
            })
            .expect("read summary checkpoint table info");

        assert_eq!(summary_body_bytes, "legacy summary body".len() as i64);
        assert_eq!(summary_before_turn_id, Some(8));
        assert_eq!(migrated_summary_body, "legacy summary body");
        assert_eq!(
            checkpoint_columns,
            vec![
                "session_id".to_owned(),
                "summarized_through_turn_id".to_owned(),
                "summary_before_turn_id".to_owned(),
                "summary_body_bytes".to_owned(),
                "summary_budget_chars".to_owned(),
                "summary_window_size".to_owned(),
                "summary_format_version".to_owned(),
                "updated_at_ts".to_owned(),
            ]
        );
        assert_eq!(
            read_session_turn_indices(&config, "legacy-session")
                .expect("read migrated session turn indices"),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
        );
        assert_eq!(
            read_session_turn_count(&config, "legacy-session")
                .expect("read migrated session turn count"),
            Some(10)
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn context_snapshot_returns_no_materialized_summary_when_window_covers_session() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-context-snapshot-short-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&tmp);
        let db_path = tmp.join("context-snapshot-short.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config =
            sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 4);

        append_turn_direct("snapshot-short-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("snapshot-short-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");

        let snapshot = load_context_snapshot("snapshot-short-session", &config)
            .expect("load short context snapshot");

        assert!(snapshot.summary_body.is_none());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(snapshot.window_turns[0].content, "turn 1");
        assert_eq!(snapshot.window_turns[1].content, "turn 2");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn summary_context_snapshot_avoids_checkpoint_query_when_window_exactly_covers_session() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-context-snapshot-exact-window-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("context-snapshot-exact-window.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config =
            sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

        append_turn_direct("snapshot-exact-window-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(
            "snapshot-exact-window-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");

        reset_cached_prepare_metrics_for_tests();
        let snapshot = load_context_snapshot("snapshot-exact-window-session", &config)
            .expect("load exact-window context snapshot");

        assert!(snapshot.summary_body.is_none());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT turn_count\n             FROM memory_session_state"
            ),
            1,
            "expected exact-window summary snapshots to consult session turn-count metadata before choosing the lighter prompt-window query shape"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT id, role, content\n             FROM turns"
            ),
            1,
            "expected exact-window summary snapshots to reuse the visible-turn prompt query once turn-count metadata proves there is no summarized prefix"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT role, content\n             FROM turns"
            ),
            0,
            "expected exact-window summary snapshots to retire the older lean prompt query shape now that internal records require visible-turn filtering by id"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
            0,
            "expected exact-window summary snapshots to retire the heavier joined turn-count query shape"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
            0,
            "expected exact-window summary snapshots to avoid session_turn_index metadata when turn-count metadata already rules out a summarized prefix"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("memory_summary_checkpoints"),
            0,
            "expected summary snapshot to avoid checkpoint queries when the active window already starts at the first session turn"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_context_snapshot_falls_back_to_payload_overflow_probe_when_turn_count_metadata_is_missing()
     {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-context-snapshot-missing-turn-count-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("context-snapshot-missing-turn-count.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct(
            "snapshot-missing-turn-count-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "snapshot-missing-turn-count-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "snapshot-missing-turn-count-session",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
        runtime
            .with_connection_mut("test.delete_missing_turn_count_state", |conn| {
                conn.execute(
                    "DELETE FROM memory_session_state
                     WHERE session_id = ?1",
                    rusqlite::params!["snapshot-missing-turn-count-session"],
                )
                .map_err(|error| format!("delete session turn count failed: {error}"))?;
                Ok(())
            })
            .expect("delete session turn count");

        reset_cached_prepare_metrics_for_tests();
        let snapshot = load_context_snapshot("snapshot-missing-turn-count-session", &config)
            .expect("load context snapshot with missing turn-count metadata");

        assert!(snapshot.summary_body.is_some());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(snapshot.window_turns[0].content, "turn 2");
        assert_eq!(snapshot.window_turns[1].content, "turn 3");
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT turn_count\n             FROM memory_session_state"
            ),
            1,
            "expected summary snapshot to attempt the turn-count-aware fast path before deciding metadata is missing"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT id, role, content\n             FROM turns\n             WHERE session_id = ?1\n             ORDER BY id DESC\n             LIMIT ?2"
            ),
            1,
            "expected summary snapshot to fall back to the legacy overflow-probe query when turn-count metadata is unavailable"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
            0,
            "expected missing turn-count metadata fallback to avoid the older joined turn-count query shape"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
            0,
            "expected missing turn-count metadata fallback to avoid indexed session_turn_index probes"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summary_context_snapshot_uses_turn_count_metadata_to_choose_known_overflow_query_shape() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();
        reset_cached_prepare_metrics_for_tests();
        let _metrics = begin_sqlite_metric_capture_for_tests();

        let tmp = std::env::temp_dir().join(format!(
            "loong-context-snapshot-known-overflow-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("context-snapshot-known-overflow.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

        append_turn_direct("snapshot-known-overflow-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(
            "snapshot-known-overflow-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct("snapshot-known-overflow-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        reset_cached_prepare_metrics_for_tests();
        let snapshot = load_context_snapshot("snapshot-known-overflow-session", &config)
            .expect("load context snapshot with known overflow");

        assert!(snapshot.summary_body.is_some());
        assert_eq!(snapshot.window_turns.len(), 2);
        assert_eq!(snapshot.window_turns[0].content, "turn 2");
        assert_eq!(snapshot.window_turns[1].content, "turn 3");
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT turn_count\n             FROM memory_session_state"
            ),
            1,
            "expected overflowing summary snapshots to consult session turn-count metadata once"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "LEFT JOIN memory_summary_checkpoints checkpoint"
            ),
            1,
            "expected overflowing summary snapshots to co-load the active window and checkpoint metadata once turn-count metadata proves overflow"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT id, role, content\n             FROM turns"
            ),
            0,
            "expected known-overflow summary snapshots to retire the older id-only window query once checkpoint metadata is folded into the fast path"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
            0,
            "expected known-overflow summary snapshots to retire the joined turn-count query shape"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
            0,
            "expected known-overflow summary snapshots to avoid indexed session_turn_index probes on the fast path"
        );
        assert_eq!(
            cached_prepare_count_for_sql_fragment_for_tests(
                "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
            ),
            0,
            "expected known-overflow summary snapshots to avoid a second checkpoint metadata lookup after the window query already proved overflow"
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn canonical_memory_search_returns_prior_session_hits_and_excludes_current_session() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-canonical-memory-search-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("canonical-search.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());

        append_turn_direct(
            "prior-session",
            "assistant",
            "Deployment cutoff is 17:00 Beijing time and requires a release note.",
            &config,
        )
        .expect("append prior session recall candidate");
        append_turn_direct(
            "active-session",
            "assistant",
            "Deployment cutoff draft that should not be recalled from the active session.",
            &config,
        )
        .expect("append active session recall candidate");
        append_turn_direct(
            "delegate-child",
            "assistant",
            "Delegate child turn that should stay out of root-session recall.",
            &config,
        )
        .expect("append delegate child recall candidate");
        append_turn_direct(
            "root-archived",
            "assistant",
            "Archived root turn that should stay out of resumable recall.",
            &config,
        )
        .expect("append archived root recall candidate");

        let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
        runtime
            .with_connection_mut("test.seed_canonical_search_session_metadata", |conn| {
                conn.execute_batch(
                    "
                    INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
                    VALUES
                      ('prior-session', 'root', NULL, NULL, 'ready', 100, 100, NULL),
                      ('delegate-child', 'delegate_child', 'prior-session', NULL, 'ready', 200, 200, NULL),
                      ('root-archived', 'root', NULL, NULL, 'ready', 300, 300, NULL);
                    INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, ts)
                    VALUES ('root-archived', 'session_archived', NULL, '{}', 400);
                    ",
                )
                .map_err(|error| format!("seed canonical search session metadata failed: {error}"))?;
                Ok(())
            })
            .expect("seed canonical search session metadata");

        let hits = search_canonical_records_for_recall(
            "deployment cutoff release note",
            4,
            Some("active-session"),
            &config,
        )
        .expect("search canonical memory");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.session_id, "prior-session");
        assert_eq!(hits[0].record.kind, CanonicalMemoryKind::AssistantTurn);
        assert_eq!(hits[0].record.scope, MemoryScope::Session);
        assert_eq!(hits[0].session_turn_index, Some(1));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn canonical_memory_search_preserves_structured_scope_and_kind_metadata() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-canonical-memory-structured-search-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("canonical-structured-search.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());

        let payload = json!({
            "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "Workspace release checklist includes rollback and smoke test steps.",
            "metadata": {
                "source": "workspace-import"
            },
        })
        .to_string();

        append_turn_direct("workspace-session", "assistant", &payload, &config)
            .expect("append structured canonical payload");

        let hits = search_canonical_records_for_recall("rollback smoke test", 4, None, &config)
            .expect("search canonical memory");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.scope, MemoryScope::Workspace);
        assert_eq!(hits[0].record.kind, CanonicalMemoryKind::ImportedProfile);
        assert_eq!(hits[0].record.metadata["source"], "workspace-import");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn canonical_memory_search_matches_segmented_chinese_queries() {
        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-canonical-memory-chinese-search-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("canonical-chinese-search.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());

        append_turn_direct(
            "workspace-session",
            "assistant",
            "中文分词用于数据库搜索和记忆召回。",
            &config,
        )
        .expect("append chinese canonical payload");

        let hits = search_canonical_records_for_recall("中文 分词", 4, None, &config)
            .expect("search canonical memory by segmented chinese query");

        assert_eq!(hits.len(), 1, "hits={hits:?}");
        assert_eq!(hits[0].record.session_id, "workspace-session");
        assert!(hits[0].record.content.contains("数据库搜索"));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn canonical_memory_search_matches_metadata_only_queries() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-canonical-memory-metadata-only-search-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("canonical-metadata-only-search.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());

        let payload = json!({
            "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "release checklist",
            "metadata": {
                "source": "workspace-import"
            },
        })
        .to_string();

        append_turn_direct("workspace-session", "assistant", &payload, &config)
            .expect("append structured canonical payload");

        let hits = search_canonical_records_for_recall("workspace-import", 4, None, &config)
            .expect("search canonical memory by metadata");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.scope, MemoryScope::Workspace);
        assert_eq!(hits[0].record.kind, CanonicalMemoryKind::ImportedProfile);
        assert_eq!(hits[0].record.metadata["source"], "workspace-import");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_repairs_stale_canonical_fts_metadata_schema() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-canonical-memory-stale-fts-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("stale-canonical-fts.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());
        ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

        let conn = Connection::open(&db_path).expect("open sqlite db");
        conn.execute_batch(
            "
            DROP TRIGGER IF EXISTS memory_canonical_records_ai;
            DROP TRIGGER IF EXISTS memory_canonical_records_ad;
            DROP TRIGGER IF EXISTS memory_canonical_records_au;
            DROP TABLE IF EXISTS memory_canonical_records_fts;
            CREATE VIRTUAL TABLE memory_canonical_records_fts
              USING fts5(content, content='memory_canonical_records', content_rowid='record_id');
            CREATE TRIGGER memory_canonical_records_ai
              AFTER INSERT ON memory_canonical_records
            BEGIN
              INSERT INTO memory_canonical_records_fts(rowid, content)
              VALUES (new.record_id, new.content);
            END;
            CREATE TRIGGER memory_canonical_records_ad
              AFTER DELETE ON memory_canonical_records
            BEGIN
              INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
              VALUES ('delete', old.record_id, old.content);
            END;
            CREATE TRIGGER memory_canonical_records_au
              AFTER UPDATE ON memory_canonical_records
            BEGIN
              INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
              VALUES ('delete', old.record_id, old.content);
              INSERT INTO memory_canonical_records_fts(rowid, content)
              VALUES (new.record_id, new.content);
            END;
            PRAGMA user_version = 8;
            ",
        )
        .expect("degrade canonical FTS schema");
        drop(conn);

        let payload = json!({
            "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "release checklist",
            "metadata": {
                "source": "workspace-import"
            },
        })
        .to_string();
        append_turn_direct("workspace-session", "assistant", &payload, &config)
            .expect("append structured canonical payload");

        ensure_memory_db_ready(None, &config).expect("repair stale canonical FTS schema");

        let hits = search_canonical_records_for_recall("workspace-import", 4, None, &config)
            .expect("search canonical memory after repair");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.metadata["source"], "workspace-import");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cached_runtime_repair_path_recovers_stale_canonical_fts_metadata_schema() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-cached-runtime-stale-fts-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("cached-runtime-stale-fts.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());
        ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

        let runtime = acquire_memory_runtime(&config).expect("cached sqlite runtime");
        runtime
            .with_connection_mut("test.degrade_cached_runtime_canonical_fts", |conn| {
                conn.execute_batch(
                    "
                    DROP TRIGGER IF EXISTS memory_canonical_records_ai;
                    DROP TRIGGER IF EXISTS memory_canonical_records_ad;
                    DROP TRIGGER IF EXISTS memory_canonical_records_au;
                    DROP TABLE IF EXISTS memory_canonical_records_fts;
                    CREATE VIRTUAL TABLE memory_canonical_records_fts
                      USING fts5(content, content='memory_canonical_records', content_rowid='record_id');
                    CREATE TRIGGER memory_canonical_records_ai
                      AFTER INSERT ON memory_canonical_records
                    BEGIN
                      INSERT INTO memory_canonical_records_fts(rowid, content)
                      VALUES (new.record_id, new.content);
                    END;
                    CREATE TRIGGER memory_canonical_records_ad
                      AFTER DELETE ON memory_canonical_records
                    BEGIN
                      INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
                      VALUES ('delete', old.record_id, old.content);
                    END;
                    CREATE TRIGGER memory_canonical_records_au
                      AFTER UPDATE ON memory_canonical_records
                    BEGIN
                      INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
                      VALUES ('delete', old.record_id, old.content);
                      INSERT INTO memory_canonical_records_fts(rowid, content)
                      VALUES (new.record_id, new.content);
                    END;
                    PRAGMA user_version = 8;
                    ",
                )
                .map_err(|error| format!("degrade cached canonical FTS schema failed: {error}"))?;
                Ok(())
            })
            .expect("degrade cached canonical FTS schema");

        let payload = json!({
            "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "release checklist",
            "metadata": {
                "source": "workspace-import"
            },
        })
        .to_string();
        append_turn_direct("workspace-session", "assistant", &payload, &config)
            .expect("append structured canonical payload");

        reset_sqlite_schema_repair_metrics_for_tests();
        ensure_memory_db_ready_with_diagnostics(None, &config)
            .expect("repair cached stale canonical FTS schema");

        let canonical_record_repair_count =
            sqlite_schema_repair_count_for_tests("canonical_records");
        assert!(
            canonical_record_repair_count >= 1,
            "expected cached runtime repair path to trigger canonical record repair, got: {canonical_record_repair_count}"
        );

        let hits = search_canonical_records_for_recall("release checklist", 5, None, &config)
            .expect("search canonical memory after cached runtime repair");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.scope, MemoryScope::Workspace);
        assert_eq!(hits[0].record.kind, CanonicalMemoryKind::ImportedProfile);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cached_runtime_repair_path_restores_control_plane_pairing_tables() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-cached-runtime-pairing-schema-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("cached-runtime-pairing-schema.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());
        ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

        let runtime = acquire_memory_runtime(&config).expect("cached sqlite runtime");
        runtime
            .with_connection_mut("test.drop_cached_pairing_tables", |conn| {
                conn.execute_batch(
                    "
                    DROP INDEX IF EXISTS idx_control_plane_pairing_requests_status_requested_at;
                    DROP INDEX IF EXISTS idx_control_plane_device_tokens_device_id;
                    DROP TABLE IF EXISTS control_plane_pairing_requests;
                    DROP TABLE IF EXISTS control_plane_device_tokens;
                    ",
                )
                .map_err(|error| format!("drop cached pairing tables failed: {error}"))?;
                Ok(())
            })
            .expect("drop cached pairing tables");

        ensure_memory_db_ready_with_diagnostics(None, &config)
            .expect("repair cached control-plane pairing schema");

        let pairing_requests_table_exists = runtime
            .with_connection("test.verify_cached_pairing_requests_table", |conn| {
                conn.query_row(
                    "SELECT COUNT(*)
                     FROM sqlite_master
                     WHERE type = 'table' AND name = 'control_plane_pairing_requests'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map(|count| count == 1)
                .map_err(|error| format!("query cached pairing requests table failed: {error}"))
            })
            .expect("query cached pairing requests table");
        let device_tokens_table_exists = runtime
            .with_connection("test.verify_cached_device_tokens_table", |conn| {
                conn.query_row(
                    "SELECT COUNT(*)
                     FROM sqlite_master
                     WHERE type = 'table' AND name = 'control_plane_device_tokens'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map(|count| count == 1)
                .map_err(|error| format!("query cached device tokens table failed: {error}"))
            })
            .expect("query cached device tokens table");

        assert!(pairing_requests_table_exists);
        assert!(device_tokens_table_exists);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_preserves_newer_schema_versions_without_current_repairs() {
        let _guard = sqlite_runtime_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-future-sqlite-schema-version-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("future-schema-version.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = sqlite_test_config(db_path.clone());
        ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

        let runtime = acquire_memory_runtime(&config).expect("cached sqlite runtime");
        let future_schema_version = SQLITE_MEMORY_SCHEMA_VERSION + 1;
        runtime
            .with_connection_mut("test.bump_sqlite_user_version", |conn| {
                write_sqlite_user_version(conn, future_schema_version)
            })
            .expect("bump sqlite user_version");

        reset_sqlite_schema_repair_metrics_for_tests();
        ensure_memory_db_ready_with_diagnostics(None, &config)
            .expect("recheck cached newer sqlite schema");

        let cached_user_version = runtime
            .with_connection("test.read_cached_future_user_version", |conn| {
                read_sqlite_user_version(conn)
            })
            .expect("read cached future sqlite user_version");
        assert_eq!(cached_user_version, future_schema_version);
        drop(runtime);

        reset_sqlite_runtime_test_state();
        ensure_memory_db_ready_with_diagnostics(Some(db_path.clone()), &config)
            .expect("reopen newer sqlite schema");

        let reopened_runtime =
            acquire_memory_runtime(&config).expect("reopen cached sqlite runtime");
        let reopened_user_version = reopened_runtime
            .with_connection("test.read_future_user_version", |conn| {
                read_sqlite_user_version(conn)
            })
            .expect("read future sqlite user_version");
        let schema_init_count = sqlite_schema_init_count_for_tests(&db_path);

        assert_eq!(reopened_user_version, future_schema_version);
        assert_eq!(schema_init_count, 0);

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_memory_db_ready_backfills_canonical_records_for_legacy_turns() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-canonical-memory-migration-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("legacy-canonical.sqlite3");
        let _ = fs::remove_file(&db_path);

        let conn = Connection::open(&db_path).expect("open legacy sqlite db");
        conn.execute_batch(
            "
            PRAGMA user_version = 4;
            CREATE TABLE turns(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              session_id TEXT NOT NULL,
              session_turn_index INTEGER,
              role TEXT NOT NULL,
              content TEXT NOT NULL,
              ts INTEGER NOT NULL
            );
            CREATE INDEX idx_turns_session_id ON turns(session_id, id);
            CREATE UNIQUE INDEX idx_turns_session_turn_index
              ON turns(session_id, session_turn_index);
            CREATE TABLE memory_session_state(
              session_id TEXT PRIMARY KEY,
              turn_count INTEGER NOT NULL
            );
            CREATE TABLE memory_summary_checkpoints(
              session_id TEXT PRIMARY KEY,
              summarized_through_turn_id INTEGER NOT NULL,
              summary_before_turn_id INTEGER,
              summary_body_bytes INTEGER NOT NULL DEFAULT 0,
              summary_budget_chars INTEGER NOT NULL,
              summary_window_size INTEGER NOT NULL,
              summary_format_version INTEGER NOT NULL,
              updated_at_ts INTEGER NOT NULL
            );
            CREATE TABLE memory_summary_checkpoint_bodies(
              session_id TEXT PRIMARY KEY
                REFERENCES memory_summary_checkpoints(session_id) ON DELETE CASCADE,
              summary_body TEXT NOT NULL
            );
            CREATE TABLE approval_requests(
              approval_request_id TEXT PRIMARY KEY,
              session_id TEXT NOT NULL,
              turn_id TEXT NOT NULL,
              tool_call_id TEXT NOT NULL,
              tool_name TEXT NOT NULL,
              approval_key TEXT NOT NULL,
              status TEXT NOT NULL,
              decision TEXT NULL,
              request_payload_json TEXT NOT NULL,
              governance_snapshot_json TEXT NOT NULL,
              requested_at INTEGER NOT NULL,
              resolved_at INTEGER NULL,
              resolved_by_session_id TEXT NULL,
              executed_at INTEGER NULL,
              last_error TEXT NULL
            );
            CREATE TABLE approval_grants(
              scope_session_id TEXT NOT NULL,
              approval_key TEXT NOT NULL,
              created_by_session_id TEXT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY(scope_session_id, approval_key)
            );
            CREATE INDEX idx_approval_requests_session_status_requested_at
              ON approval_requests(session_id, status, requested_at DESC, approval_request_id);
            ",
        )
        .expect("create legacy schema");
        conn.execute(
            "INSERT INTO turns(session_id, session_turn_index, role, content, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "legacy-session",
                1_i64,
                "assistant",
                "Legacy rollout fix includes rollback and smoke test verification.",
                1_717_000_000_i64
            ],
        )
        .expect("insert legacy turn");
        conn.execute(
            "INSERT INTO memory_session_state(session_id, turn_count)
             VALUES (?1, ?2)",
            rusqlite::params!["legacy-session", 1_i64],
        )
        .expect("insert session state");
        drop(conn);

        let config = sqlite_test_config(db_path.clone());
        let _ = ensure_memory_db_ready(None, &config).expect("upgrade legacy sqlite db");

        let hits = search_canonical_records_for_recall("rollback smoke test", 4, None, &config)
            .expect("search canonical memory after migration");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.session_id, "legacy-session");

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_window_size_prefers_injected_config() {
        let config = MemoryRuntimeConfig {
            sqlite_path: None,
            sliding_window: 24,
            ..MemoryRuntimeConfig::default()
        };

        assert_eq!(default_window_size(&config), 24);
    }

    #[test]
    fn default_window_size_falls_back_to_default_without_config() {
        assert_eq!(default_window_size(&MemoryRuntimeConfig::default()), 12);
    }
}

#[cfg(test)]
mod test_support {
    use super::*;
    use crate::config::{MemoryMode, MemoryProfile};
    use std::sync::Condvar;

    #[derive(Default)]
    struct SqliteMetricCapture {
        active_thread: Option<ThreadId>,
        cached_prepare_counts: HashMap<&'static str, usize>,
        summary_materialization_counts: HashMap<&'static str, usize>,
        runtime_path_normalization_counts: HashMap<&'static str, usize>,
    }

    #[derive(Default)]
    struct SqliteRuntimeCacheMissGate {
        path: Option<PathBuf>,
        target_waiters: usize,
        waiting_threads: usize,
        released: bool,
    }

    fn sqlite_runtime_test_support_lock() -> &'static Mutex<()> {
        super::sqlite_runtime_test_lock()
    }

    fn bootstrap_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
        static BOOTSTRAP_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
        BOOTSTRAP_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn schema_init_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
        static SCHEMA_INIT_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
        SCHEMA_INIT_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn schema_repair_counts() -> &'static Mutex<HashMap<&'static str, usize>> {
        static SCHEMA_REPAIR_COUNTS: OnceLock<Mutex<HashMap<&'static str, usize>>> =
            OnceLock::new();
        SCHEMA_REPAIR_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn sqlite_metric_capture() -> &'static Mutex<SqliteMetricCapture> {
        static SQLITE_METRIC_CAPTURE: OnceLock<Mutex<SqliteMetricCapture>> = OnceLock::new();
        SQLITE_METRIC_CAPTURE.get_or_init(|| Mutex::new(SqliteMetricCapture::default()))
    }

    fn sqlite_runtime_cache_miss_gate() -> &'static (Mutex<SqliteRuntimeCacheMissGate>, Condvar) {
        static SQLITE_RUNTIME_CACHE_MISS_GATE: OnceLock<(
            Mutex<SqliteRuntimeCacheMissGate>,
            Condvar,
        )> = OnceLock::new();
        SQLITE_RUNTIME_CACHE_MISS_GATE.get_or_init(|| {
            (
                Mutex::new(SqliteRuntimeCacheMissGate::default()),
                Condvar::new(),
            )
        })
    }

    fn lock_sqlite_metric_capture() -> std::sync::MutexGuard<'static, SqliteMetricCapture> {
        sqlite_metric_capture()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn record_sqlite_bootstrap(path: &Path) {
        let normalized_path = normalize_runtime_db_path_best_effort(path);
        let mut counts = bootstrap_counts().lock().expect("bootstrap counts lock");
        let entry = counts.entry(normalized_path).or_insert(0);
        *entry += 1;
    }

    pub(super) fn record_sqlite_schema_init(path: &Path) {
        let normalized_path = normalize_runtime_db_path_best_effort(path);
        let mut counts = schema_init_counts()
            .lock()
            .expect("schema init counts lock");
        let entry = counts.entry(normalized_path).or_insert(0);
        *entry += 1;
    }

    pub(super) fn sqlite_bootstrap_count(path: &Path) -> usize {
        let normalized_path = normalize_runtime_db_path_best_effort(path);
        let counts = bootstrap_counts().lock().expect("bootstrap counts lock");
        counts.get(&normalized_path).copied().unwrap_or_default()
    }

    pub(super) fn sqlite_schema_init_count(path: &Path) -> usize {
        let normalized_path = normalize_runtime_db_path_best_effort(path);
        let counts = schema_init_counts()
            .lock()
            .expect("schema init counts lock");
        counts.get(&normalized_path).copied().unwrap_or_default()
    }

    pub(super) fn sqlite_bootstrap_count_under_prefix(prefix: &Path) -> usize {
        let normalized_prefix = normalize_runtime_db_path_best_effort(prefix);
        let counts = bootstrap_counts().lock().expect("bootstrap counts lock");
        counts
            .iter()
            .filter(|(path, _)| path.starts_with(&normalized_prefix))
            .map(|(_, count)| *count)
            .sum()
    }

    pub(super) fn record_sqlite_schema_repair(kind: &'static str) {
        let mut counts = schema_repair_counts()
            .lock()
            .expect("schema repair counts lock");
        let entry = counts.entry(kind).or_insert(0);
        *entry += 1;
    }

    pub(super) fn sqlite_schema_repair_count(kind: &'static str) -> usize {
        let counts = schema_repair_counts()
            .lock()
            .expect("schema repair counts lock");
        counts.get(kind).copied().unwrap_or_default()
    }

    pub(super) fn reset_sqlite_schema_repair_metrics() {
        schema_repair_counts()
            .lock()
            .expect("schema repair counts lock")
            .clear();
    }

    pub(super) fn record_cached_prepare(sql: &'static str) {
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        if capture.active_thread == Some(current_thread) {
            let entry = capture.cached_prepare_counts.entry(sql).or_insert(0);
            *entry += 1;
        }
    }

    pub(super) fn cached_prepare_count_for_sql_fragment(fragment: &str) -> usize {
        let capture = lock_sqlite_metric_capture();
        capture
            .cached_prepare_counts
            .iter()
            .filter(|(sql, _)| sql.contains(fragment))
            .map(|(_, count)| *count)
            .sum()
    }

    pub(super) fn reset_cached_prepare_metrics() {
        lock_sqlite_metric_capture().cached_prepare_counts.clear();
    }

    pub(super) fn record_runtime_path_normalization_full() {
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        if capture.active_thread == Some(current_thread) {
            let entry = capture
                .runtime_path_normalization_counts
                .entry("full")
                .or_insert(0);
            *entry += 1;
        }
    }

    pub(super) fn record_runtime_path_normalization_alias_hit() {
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        if capture.active_thread == Some(current_thread) {
            let entry = capture
                .runtime_path_normalization_counts
                .entry("alias_hit")
                .or_insert(0);
            *entry += 1;
        }
    }

    pub(super) fn runtime_path_normalization_full_count() -> usize {
        let capture = lock_sqlite_metric_capture();
        capture
            .runtime_path_normalization_counts
            .get("full")
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn runtime_path_normalization_alias_hit_count() -> usize {
        let capture = lock_sqlite_metric_capture();
        capture
            .runtime_path_normalization_counts
            .get("alias_hit")
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn configure_sqlite_runtime_cache_miss(path: &Path, target_waiters: usize) {
        let normalized_path = normalize_runtime_db_path_best_effort(path);
        let (gate_lock, gate_condvar) = sqlite_runtime_cache_miss_gate();
        let mut gate = gate_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        gate.path = Some(normalized_path);
        gate.target_waiters = target_waiters;
        gate.waiting_threads = 0;
        gate.released = false;
        gate_condvar.notify_all();
    }

    pub(super) fn wait_for_sqlite_runtime_cache_miss(path: &Path) {
        let normalized_path = normalize_runtime_db_path_best_effort(path);
        let (gate_lock, gate_condvar) = sqlite_runtime_cache_miss_gate();
        let mut gate = gate_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(configured_path) = gate.path.as_ref() else {
            return;
        };
        if *configured_path != normalized_path {
            return;
        }
        if gate.released {
            return;
        }

        gate.waiting_threads += 1;
        if gate.waiting_threads >= gate.target_waiters {
            gate.released = true;
            gate_condvar.notify_all();
            return;
        }

        while !gate.released {
            gate = gate_condvar
                .wait(gate)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub(super) fn clear_sqlite_runtime_cache_miss() {
        let (gate_lock, gate_condvar) = sqlite_runtime_cache_miss_gate();
        let mut gate = gate_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        gate.path = None;
        gate.target_waiters = 0;
        gate.waiting_threads = 0;
        gate.released = false;
        gate_condvar.notify_all();
    }

    pub(super) fn record_summary_streaming_query(kind: &'static str) {
        let key = match kind {
            "rebuild" => "streaming_rebuild",
            "catch_up" => "streaming_catch_up",
            _ => kind,
        };
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        if capture.active_thread == Some(current_thread) {
            let entry = capture
                .summary_materialization_counts
                .entry(key)
                .or_insert(0);
            *entry += 1;
        }
    }

    pub(super) fn summary_buffered_query_count(kind: &'static str) -> usize {
        let key = match kind {
            "rebuild" => "buffered_rebuild",
            "catch_up" => "buffered_catch_up",
            _ => kind,
        };
        let capture = lock_sqlite_metric_capture();
        capture
            .summary_materialization_counts
            .get(key)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn summary_streaming_query_count(kind: &'static str) -> usize {
        let key = match kind {
            "rebuild" => "streaming_rebuild",
            "catch_up" => "streaming_catch_up",
            _ => kind,
        };
        let capture = lock_sqlite_metric_capture();
        capture
            .summary_materialization_counts
            .get(key)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn record_summary_payload_decode() {
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        if capture.active_thread == Some(current_thread) {
            let entry = capture
                .summary_materialization_counts
                .entry("payload_decode")
                .or_insert(0);
            *entry += 1;
        }
    }

    pub(super) fn record_summary_row_observed() {
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        if capture.active_thread == Some(current_thread) {
            let entry = capture
                .summary_materialization_counts
                .entry("row_observed")
                .or_insert(0);
            *entry += 1;
        }
    }

    pub(super) fn summary_row_observed_count() -> usize {
        let capture = lock_sqlite_metric_capture();
        capture
            .summary_materialization_counts
            .get("row_observed")
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn summary_frontier_probe_count(kind: &'static str) -> usize {
        let key = match kind {
            "rebuild" => "frontier_probe_rebuild",
            "catch_up" => "frontier_probe_catch_up",
            _ => kind,
        };
        let capture = lock_sqlite_metric_capture();
        capture
            .summary_materialization_counts
            .get(key)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn record_summary_frontier_probe(kind: &'static str) {
        let key = match kind {
            "rebuild" => "frontier_probe_rebuild",
            "catch_up" => "frontier_probe_catch_up",
            _ => kind,
        };
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        if capture.active_thread == Some(current_thread) {
            let entry = capture
                .summary_materialization_counts
                .entry(key)
                .or_insert(0);
            *entry += 1;
        }
    }

    pub(super) fn summary_payload_decode_count() -> usize {
        let capture = lock_sqlite_metric_capture();
        capture
            .summary_materialization_counts
            .get("payload_decode")
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn summary_normalization_count() -> usize {
        let capture = lock_sqlite_metric_capture();
        capture
            .summary_materialization_counts
            .get("normalization")
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn reset_summary_materialization_metrics() {
        lock_sqlite_metric_capture()
            .summary_materialization_counts
            .clear();
    }

    pub(super) fn begin_sqlite_metric_capture() {
        let current_thread = std::thread::current().id();
        let mut capture = lock_sqlite_metric_capture();
        capture.active_thread = Some(current_thread);
        capture.cached_prepare_counts.clear();
        capture.summary_materialization_counts.clear();
        capture.runtime_path_normalization_counts.clear();
    }

    pub(super) fn end_sqlite_metric_capture() {
        let mut capture = lock_sqlite_metric_capture();
        capture.active_thread = None;
        capture.cached_prepare_counts.clear();
        capture.summary_materialization_counts.clear();
        capture.runtime_path_normalization_counts.clear();
    }

    #[test]
    fn prompt_window_turn_is_visible_filters_internal_persisted_records() {
        let conversation_event = crate::memory::build_conversation_event_content(
            "provider_prompt_frame_snapshot",
            serde_json::json!({"phase": "initial"}),
        );
        let tool_decision = crate::memory::build_tool_decision_content(
            "turn-1",
            "call-1",
            serde_json::json!({"decision": "allow"}),
        );
        let plain_assistant = "assistant reply";
        let plain_user = "user prompt";

        assert!(!prompt_window_turn_is_visible(
            "session-visible-filter",
            "assistant",
            conversation_event.as_str()
        ));
        assert!(!prompt_window_turn_is_visible(
            "session-visible-filter",
            "assistant",
            tool_decision.as_str()
        ));
        assert!(prompt_window_turn_is_visible(
            "session-visible-filter",
            "assistant",
            plain_assistant
        ));
        assert!(prompt_window_turn_is_visible(
            "session-visible-filter",
            "user",
            plain_user
        ));
    }

    #[test]
    fn prompt_window_mixed_overflow_regression() {
        let runtime_test_support_lock = sqlite_runtime_test_support_lock();
        let guard_result = runtime_test_support_lock.lock();
        let _guard = match guard_result {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        reset_sqlite_runtime_test_state();

        let tmp = std::env::temp_dir().join(format!(
            "loong-prompt-window-mixed-overflow-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create temp dir");
        let db_path = tmp.join("prompt-window-mixed-overflow.sqlite3");
        let _ = fs::remove_file(&db_path);

        let config = MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 3,
            ..MemoryRuntimeConfig::default()
        };
        let hidden_inner = crate::memory::build_conversation_event_content(
            "provider_prompt_frame_snapshot",
            serde_json::json!({"phase": "initial"}),
        );
        let hidden_tail = crate::memory::build_tool_decision_content(
            "turn-4",
            "call-1",
            serde_json::json!({"decision": "allow"}),
        );

        assert!(!prompt_window_turn_is_visible(
            "prompt-window-mixed-overflow-session",
            "assistant",
            hidden_inner.as_str()
        ));
        assert!(!prompt_window_turn_is_visible(
            "prompt-window-mixed-overflow-session",
            "assistant",
            hidden_tail.as_str()
        ));
        assert!(prompt_window_turn_is_visible(
            "prompt-window-mixed-overflow-session",
            "assistant",
            "visible 4"
        ));

        append_turn_direct(
            "prompt-window-mixed-overflow-session",
            "user",
            "visible 1",
            &config,
        )
        .expect("append visible turn 1");
        append_turn_direct(
            "prompt-window-mixed-overflow-session",
            "assistant",
            hidden_inner.as_str(),
            &config,
        )
        .expect("append hidden inner record");
        append_turn_direct(
            "prompt-window-mixed-overflow-session",
            "assistant",
            "visible 2",
            &config,
        )
        .expect("append visible turn 2");
        append_turn_direct(
            "prompt-window-mixed-overflow-session",
            "user",
            "visible 3",
            &config,
        )
        .expect("append visible turn 3");
        append_turn_direct(
            "prompt-window-mixed-overflow-session",
            "assistant",
            hidden_tail.as_str(),
            &config,
        )
        .expect("append hidden tail record");
        append_turn_direct(
            "prompt-window-mixed-overflow-session",
            "assistant",
            "visible 4",
            &config,
        )
        .expect("append visible turn 4");
        append_turn_direct(
            "prompt-window-mixed-overflow-session",
            "user",
            "visible 5",
            &config,
        )
        .expect("append visible turn 5");

        let snapshot = load_context_snapshot("prompt-window-mixed-overflow-session", &config)
            .expect("load mixed-overflow context snapshot");
        let window_contents = snapshot
            .window_turns
            .iter()
            .map(|turn| turn.content.as_str())
            .collect::<Vec<_>>();

        assert_eq!(snapshot.window_turns.len(), 3);
        assert_eq!(window_contents, vec!["visible 3", "visible 4", "visible 5"]);
        assert!(snapshot.summary_body.is_none());

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir_all(&tmp);
    }

    pub(super) fn reset_test_state() {
        bootstrap_counts()
            .lock()
            .expect("bootstrap counts lock")
            .clear();
        schema_init_counts()
            .lock()
            .expect("schema init counts lock")
            .clear();
        reset_sqlite_schema_repair_metrics();
        end_sqlite_metric_capture();
        reset_cached_prepare_metrics();
        reset_summary_materialization_metrics();
        clear_sqlite_runtime_cache_miss();
    }
}
