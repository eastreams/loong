use super::*;

pub(super) fn ensure_turn_session_index_and_state_metadata(
    conn: &Connection,
) -> Result<(), String> {
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
        CREATE TABLE IF NOT EXISTS session_route_bindings(
          route_session_id TEXT PRIMARY KEY,
          active_session_id TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_route_bindings_active_session_id
          ON session_route_bindings(active_session_id, updated_at, route_session_id);
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

pub(super) fn ensure_session_tree_storage(conn: &Connection) -> Result<(), String> {
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

pub(super) fn ensure_session_terminal_outcome_storage(conn: &Connection) -> Result<(), String> {
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

pub(super) fn ensure_session_event_search_storage(conn: &Connection) -> Result<(), String> {
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

pub(super) fn session_event_fts_needs_rebuild(conn: &Connection) -> Result<bool, String> {
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

pub(super) fn session_event_search_text(event_kind: &str, payload_json: &str) -> String {
    build_search_index_text(&[event_kind, payload_json])
}

pub(super) fn ensure_approval_lifecycle_tables(conn: &Connection) -> Result<(), String> {
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

pub(super) fn ensure_control_plane_pairing_tables(conn: &Connection) -> Result<(), String> {
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

pub(super) fn ensure_session_tool_consent_storage(conn: &mut Connection) -> Result<(), String> {
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

pub(super) fn ensure_session_tool_policy_storage(conn: &Connection) -> Result<(), String> {
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

pub(super) fn sqlite_table_has_column(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, String> {
    Ok(sqlite_table_columns(conn, table_name)?
        .iter()
        .any(|current_name| current_name == column_name))
}

pub(super) fn sqlite_table_sql_contains(
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

pub(super) fn sqlite_table_columns(
    conn: &Connection,
    table_name: &str,
) -> Result<Vec<String>, String> {
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
