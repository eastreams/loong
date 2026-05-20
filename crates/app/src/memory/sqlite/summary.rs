use super::bootstrap::unix_ts_now;
use super::*;

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

pub(super) fn ensure_summary_checkpoint_storage_layout(conn: &Connection) -> Result<(), String> {
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

pub(super) fn resolve_actual_turn_count(
    conn: &Connection,
    session_id: &str,
) -> Result<i64, String> {
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

pub(super) fn prompt_window_turn_is_visible(session_id: &str, role: &str, content: &str) -> bool {
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

pub(super) fn load_summary_append_maintenance_state(
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

pub(super) fn reserve_next_session_turn_index(
    conn: &Connection,
    session_id: &str,
) -> Result<i64, String> {
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

pub(super) fn delete_summary_checkpoint(conn: &Connection, session_id: &str) -> Result<(), String> {
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

pub(super) fn maintain_summary_checkpoint_after_append(
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

pub(super) fn materialize_initial_summary_checkpoint(
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

pub(super) fn append_summary_line(
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
