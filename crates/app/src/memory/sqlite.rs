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

mod bootstrap;
mod schema;
mod search;
mod summary;
#[cfg(test)]
#[path = "sqlite/sqlite_test_support_tests.rs"]
mod test_support;
#[cfg(test)]
#[path = "sqlite/sqlite_core_tests.rs"]
mod tests;

use self::bootstrap::{
    acquire_memory_runtime, default_window_size, default_window_size_u64,
    normalize_runtime_db_path, open_sqlite_connection_with_diagnostics,
    prepare_cached_sqlite_statement, query_max_turn_id_for_session, query_transcript_page_after_id,
    sqlite_runtime_bootstrap_lock_registry, sqlite_runtime_path_alias_registry,
    sqlite_runtime_registry, unix_ts_now,
};
#[cfg(test)]
use self::bootstrap::{
    configure_sqlite_connection, normalize_runtime_db_path_best_effort, read_sqlite_user_version,
    write_sqlite_user_version,
};
use self::schema::{session_event_search_text, sqlite_table_columns, sqlite_table_has_column};
use self::search::{build_canonical_insert_input, insert_canonical_record};
#[cfg(test)]
use self::summary::{append_summary_line, prompt_window_turn_is_visible};
use self::summary::{
    delete_summary_checkpoint, load_summary_append_maintenance_state,
    maintain_summary_checkpoint_after_append, materialize_initial_summary_checkpoint,
    reserve_next_session_turn_index, resolve_actual_turn_count,
};

pub(super) fn ensure_memory_db_ready(
    path: Option<PathBuf>,
    config: &MemoryRuntimeConfig,
) -> Result<PathBuf, String> {
    bootstrap::ensure_memory_db_ready(path, config)
}

pub(super) fn ensure_memory_db_ready_with_diagnostics(
    path: Option<PathBuf>,
    config: &MemoryRuntimeConfig,
) -> Result<(PathBuf, SqliteBootstrapDiagnostics), String> {
    bootstrap::ensure_memory_db_ready_with_diagnostics(path, config)
}

pub(super) fn search_canonical_records_for_recall(
    query: &str,
    limit: usize,
    exclude_session_id: Option<&str>,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<CanonicalMemorySearchHit>, String> {
    search::search_canonical_records_for_recall(query, limit, exclude_session_id, config)
}

pub(super) fn search_workspace_memory_documents(
    query: &str,
    limit: usize,
    workspace_root: &Path,
    memory_system_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<WorkspaceMemoryIndexedSearchHit>, String> {
    search::search_workspace_memory_documents(
        query,
        limit,
        workspace_root,
        memory_system_id,
        config,
    )
}

pub(super) fn load_context_snapshot(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<ContextSnapshot, String> {
    summary::load_context_snapshot(session_id, config)
}

pub(super) fn load_context_snapshot_with_diagnostics(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<(ContextSnapshot, SqliteContextLoadDiagnostics), String> {
    summary::load_context_snapshot_with_diagnostics(session_id, config)
}

pub(super) fn load_summary_body_for_durable_flush(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Option<String>, String> {
    summary::load_summary_body_for_durable_flush(session_id, config)
}

pub(super) fn format_summary_block(summary_body: &str) -> Option<String> {
    summary::format_summary_block(summary_body)
}

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

fn preserve_session_tree_before_rebuild(
    conn: &Connection,
    session_id: &str,
    turns: &[WindowTurn],
    rewrite_ts: i64,
) -> Result<(), String> {
    let new_tail_turn_index: i64 = turns.len() as i64;
    let preservation_ts: i64 = rewrite_ts;

    // --- Drop non-active heads whose target would dangle.
    //
    // Includes: (a) heads pointing at a node past the new tail, (b) heads
    // pointing at a node that's already missing (legacy corruption).
    // The session root node (session_turn_index IS NULL) is always safe —
    // the rebuild below re-creates it.
    let stale_heads: Vec<(String, String, Option<String>)> = {
        let mut stmt = conn
            .prepare(
                "SELECT h.head_name, h.node_id, n.content
                 FROM session_heads h
                 LEFT JOIN session_nodes n ON n.node_id = h.node_id
                 WHERE h.session_id = ?1
                   AND h.head_name != ?2
                   AND (
                        n.node_id IS NULL
                        OR (n.session_turn_index IS NOT NULL
                            AND n.session_turn_index > ?3)
                   )",
            )
            .map_err(|error| format!("prepare stale head query failed: {error}"))?;
        let rows = stmt
            .query_map(
                rusqlite::params![session_id, ACTIVE_SESSION_HEAD_NAME, new_tail_turn_index],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(|error| format!("scan stale heads failed: {error}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|error| format!("read stale head row failed: {error}"))?);
        }
        out
    };

    for (head_name, stale_node_id, content_snapshot) in &stale_heads {
        let payload = json!({
            "head_name": head_name,
            "stale_node_id": stale_node_id,
            "new_tail_turn_index": new_tail_turn_index,
            "content_snapshot": content_snapshot,
        });
        let payload_json = payload.to_string();
        let search_text =
            session_event_search_text("session_tree_rewrite_dropped_head", payload_json.as_str());
        conn.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, 'session_tree_rewrite_dropped_head', NULL, ?2, ?3, ?4)",
            rusqlite::params![session_id, payload_json, search_text, preservation_ts],
        )
        .map_err(|error| format!("record dropped head event failed: {error}"))?;
        conn.execute(
            "DELETE FROM session_heads
             WHERE session_id = ?1 AND head_name = ?2",
            rusqlite::params![session_id, head_name],
        )
        .map_err(|error| format!("drop stale session head failed: {error}"))?;
    }

    // --- Null dangling *_node_id columns on artifacts.
    //
    // The artifact's own payload_json / summary_text is not node-referential
    // and is left intact — only the back-references are cleared.
    let stale_artifact_refs: Vec<(
        String,
        Option<String>, // current head_name
        Option<String>, // current anchor_node_id
        Option<String>, // current source_start_node_id
        Option<String>, // current source_end_node_id
        Option<i64>,    // anchor target's session_turn_index (NULL when absent)
        Option<i64>,    // source_start target's session_turn_index
        Option<i64>,    // source_end target's session_turn_index
        bool,           // anchor exists in session_nodes
        bool,           // source_start exists in session_nodes
        bool,           // source_end exists in session_nodes
    )> = {
        let mut stmt = conn
            .prepare(
                "SELECT a.artifact_id,
                        a.head_name,
                        a.anchor_node_id,       a.source_start_node_id,    a.source_end_node_id,
                        na.session_turn_index,  nss.session_turn_index,    nse.session_turn_index,
                        na.node_id IS NOT NULL, nss.node_id IS NOT NULL,   nse.node_id IS NOT NULL
                 FROM session_artifacts a
                 LEFT JOIN session_nodes na  ON na.node_id  = a.anchor_node_id
                 LEFT JOIN session_nodes nss ON nss.node_id = a.source_start_node_id
                 LEFT JOIN session_nodes nse ON nse.node_id = a.source_end_node_id
                 WHERE a.session_id = ?1",
            )
            .map_err(|error| format!("prepare stale artifact query failed: {error}"))?;
        let rows = stmt
            .query_map(rusqlite::params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, bool>(9)?,
                    row.get::<_, bool>(10)?,
                ))
            })
            .map_err(|error| format!("scan stale artifact refs failed: {error}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|error| format!("read stale artifact row failed: {error}"))?);
        }
        out
    };

    let ref_dangles = |node_id: &Option<String>, ti: Option<i64>, exists: bool| -> bool {
        node_id.is_some() && (!exists || ti.is_some_and(|v| v > new_tail_turn_index))
    };

    for (
        artifact_id,
        head_name,
        anchor_node_id,
        source_start_node_id,
        source_end_node_id,
        anchor_ti,
        source_start_ti,
        source_end_ti,
        anchor_exists,
        source_start_exists,
        source_end_exists,
    ) in &stale_artifact_refs
    {
        let anchor_drop = ref_dangles(anchor_node_id, *anchor_ti, *anchor_exists);
        let source_start_drop =
            ref_dangles(source_start_node_id, *source_start_ti, *source_start_exists);
        let source_end_drop = ref_dangles(source_end_node_id, *source_end_ti, *source_end_exists);
        let dropped_head_name = head_name.as_deref().filter(|current_head_name| {
            stale_heads
                .iter()
                .any(|(stale_head_name, _, _)| stale_head_name == current_head_name)
        });
        let head_name_drop = dropped_head_name.is_some();
        if !head_name_drop && !anchor_drop && !source_start_drop && !source_end_drop {
            continue;
        }

        let payload = json!({
            "artifact_id": artifact_id,
            "original_head_name": if head_name_drop { head_name.clone() } else { None },
            "original_anchor_node_id":       if anchor_drop       { anchor_node_id.clone()       } else { None },
            "original_source_start_node_id": if source_start_drop { source_start_node_id.clone() } else { None },
            "original_source_end_node_id":   if source_end_drop   { source_end_node_id.clone()   } else { None },
            "new_tail_turn_index": new_tail_turn_index,
        });
        let payload_json = payload.to_string();
        let search_text = session_event_search_text(
            "session_tree_rewrite_nulled_artifact_refs",
            payload_json.as_str(),
        );
        conn.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, 'session_tree_rewrite_nulled_artifact_refs', NULL, ?2, ?3, ?4)",
            rusqlite::params![session_id, payload_json, search_text, preservation_ts],
        )
        .map_err(|error| format!("record nulled artifact event failed: {error}"))?;

        conn.execute(
            "UPDATE session_artifacts
             SET head_name            = CASE WHEN ?2 THEN NULL ELSE head_name            END,
                 anchor_node_id       = CASE WHEN ?3 THEN NULL ELSE anchor_node_id       END,
                 source_start_node_id = CASE WHEN ?4 THEN NULL ELSE source_start_node_id END,
                 source_end_node_id   = CASE WHEN ?5 THEN NULL ELSE source_end_node_id   END
             WHERE artifact_id = ?1",
            rusqlite::params![
                artifact_id,
                head_name_drop,
                anchor_drop,
                source_start_drop,
                source_end_drop
            ],
        )
        .map_err(|error| format!("null stale artifact refs failed: {error}"))?;
    }

    Ok(())
}

fn rebuild_linear_session_tree_for_turns(
    conn: &Connection,
    session_id: &str,
    turns: &[WindowTurn],
    rewrite_ts: i64,
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

    // --- Pre-rewrite preservation phase.
    //
    // Non-`active` heads and artifacts that reference turn nodes past the new
    // tail would dangle after the DELETE below. Mirror jj's
    // `MutableRepo::update_rewritten_references` pattern: inside the same
    // transaction, drop stale heads (with a session_events audit trail
    // capturing original content) and null the out-of-range *_node_id
    // columns on artifacts (their own `payload_json` / `summary_text`
    // survive, so the artifact's self-contained content is not lost).
    preserve_session_tree_before_rebuild(conn, session_id, turns, rewrite_ts)?;

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

            let rewrite_ts = unix_ts_now();
            rebuild_linear_session_tree_for_turns(&tx, session_id, turns, rewrite_ts)?;

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
    bootstrap::clear_sqlite_runtime_registries_for_tests();
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
