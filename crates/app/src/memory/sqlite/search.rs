use super::*;

pub(super) fn build_canonical_insert_input(
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

pub(super) fn insert_canonical_record(
    conn: &Connection,
    input: CanonicalInsertInput,
) -> Result<(), String> {
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

pub(super) fn ensure_workspace_memory_search_storage(conn: &Connection) -> Result<(), String> {
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

pub(super) fn workspace_memory_search_storage_needs_rebuild(
    conn: &Connection,
) -> Result<bool, String> {
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

pub(super) fn ensure_canonical_record_storage(conn: &Connection) -> Result<(), String> {
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

pub(super) fn canonical_record_fts_needs_rebuild(conn: &Connection) -> Result<bool, String> {
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
