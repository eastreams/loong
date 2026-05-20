use super::*;
use super::schema::{
    ensure_approval_lifecycle_tables, ensure_control_plane_pairing_tables,
    ensure_session_event_search_storage, ensure_session_terminal_outcome_storage,
    ensure_session_tool_consent_storage, ensure_session_tool_policy_storage,
    ensure_session_tree_storage, ensure_turn_session_index_and_state_metadata,
    session_event_fts_needs_rebuild, sqlite_table_has_column,
};
use super::search::{
    canonical_record_fts_needs_rebuild, ensure_canonical_record_storage,
    ensure_workspace_memory_search_storage, workspace_memory_search_storage_needs_rebuild,
};
use super::summary::ensure_summary_checkpoint_storage_layout;

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

pub(super) fn default_window_size(config: &MemoryRuntimeConfig) -> usize {
    config.sliding_window.max(1)
}

pub(super) fn default_window_size_u64(config: &MemoryRuntimeConfig) -> u64 {
    default_window_size(config) as u64
}

pub(super) fn unix_ts_now() -> i64 {
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

pub(super) fn query_transcript_page_after_id(
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

pub(super) fn query_max_turn_id_for_session(
    conn: &Connection,
    session_id: &str,
) -> Result<i64, String> {
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

pub(super) fn normalize_runtime_db_path(path: &Path) -> Result<PathBuf, String> {
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
pub(super) fn normalize_runtime_db_path_best_effort(path: &Path) -> PathBuf {
    normalize_runtime_db_path(path)
        .or_else(|_| {
            absolutize_runtime_db_path(path)
                .map(|absolute| lexical_normalize_runtime_db_path(&absolute))
        })
        .unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn prepare_cached_sqlite_statement<'conn>(
    conn: &'conn Connection,
    sql: &'static str,
    error_context: &'static str,
) -> Result<rusqlite::CachedStatement<'conn>, String> {
    #[cfg(test)]
    test_support::record_cached_prepare(sql);

    conn.prepare_cached(sql)
        .map_err(|error| format!("{error_context}: {error}"))
}

pub(super) fn acquire_memory_runtime(
    config: &MemoryRuntimeConfig,
) -> Result<Arc<SqliteRuntime>, String> {
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

pub(super) fn sqlite_runtime_registry() -> &'static Mutex<HashMap<PathBuf, Arc<SqliteRuntime>>> {
    static SQLITE_RUNTIME_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<SqliteRuntime>>>> =
        OnceLock::new();
    SQLITE_RUNTIME_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn sqlite_runtime_bootstrap_lock_registry(
) -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<()>>>> {
    static SQLITE_RUNTIME_BOOTSTRAP_LOCK_REGISTRY: OnceLock<
        Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    > = OnceLock::new();
    SQLITE_RUNTIME_BOOTSTRAP_LOCK_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn sqlite_runtime_path_alias_registry() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    static SQLITE_RUNTIME_PATH_ALIAS_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> =
        OnceLock::new();
    SQLITE_RUNTIME_PATH_ALIAS_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn open_sqlite_connection_with_diagnostics(
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

pub(super) fn configure_sqlite_connection(conn: &Connection) -> Result<(), String> {
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

pub(super) fn read_sqlite_user_version(conn: &Connection) -> Result<i64, String> {
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

pub(super) fn write_sqlite_user_version(conn: &Connection, version: i64) -> Result<(), String> {
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


#[cfg(test)]
pub(super) fn clear_sqlite_runtime_registries_for_tests() {
    if let Ok(mut registry) = sqlite_runtime_registry().lock() {
        registry.clear();
    }
    if let Ok(mut bootstrap_registry) = sqlite_runtime_bootstrap_lock_registry().lock() {
        bootstrap_registry.clear();
    }
    if let Ok(mut alias_registry) = sqlite_runtime_path_alias_registry().lock() {
        alias_registry.clear();
    }
}
