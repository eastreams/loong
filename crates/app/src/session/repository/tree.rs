use super::*;

impl SessionRepository {
    fn enforce_session_artifact_retention_with_conn(
        &self,
        conn: &Connection,
        session_id: &str,
    ) -> Result<(), String> {
        let Some(max_records) = self.max_total_artifacts else {
            return Ok(());
        };

        conn.execute(
            "DELETE FROM session_artifacts
             WHERE session_id = ?1
               AND artifact_id NOT IN (
                    SELECT artifact_id
                    FROM session_artifacts
                    WHERE session_id = ?1
                    ORDER BY created_at DESC, artifact_id DESC
                    LIMIT ?2
               )",
            params![session_id, max_records as i64],
        )
        .map_err(|error| format!("delete excess session artifacts failed: {error}"))?;
        Ok(())
    }

    pub(crate) fn with_read_snapshot<T, F>(&self, operation: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session read snapshot failed: {error}"))?;
        let result = operation(&tx)?;
        tx.commit()
            .map_err(|error| format!("commit session read snapshot failed: {error}"))?;
        Ok(result)
    }

    pub fn create_session(&self, record: NewSessionRecord) -> Result<SessionRecord, String> {
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        let parent_session_id = normalize_optional_text(record.parent_session_id);
        let label = normalize_optional_text(record.label);
        let ts = unix_ts_now();
        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session create transaction failed: {error}"))?;
        tx.execute(
            "INSERT INTO sessions(
                session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![
                session_id,
                record.kind.as_str(),
                parent_session_id,
                label,
                record.state.as_str(),
                ts,
                ts,
            ],
        )
        .map_err(|error| format!("insert session row failed: {error}"))?;
        seed_session_tree_for_new_session(&tx, &session_id, ts)?;
        tx.commit()
            .map_err(|error| format!("commit session create transaction failed: {error}"))?;

        self.load_session(&session_id)?
            .ok_or_else(|| format!("session row `{session_id}` disappeared after insert"))
    }

    pub fn ensure_session(&self, record: NewSessionRecord) -> Result<SessionRecord, String> {
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        if let Some(existing) = self.load_session(&session_id)? {
            return Ok(existing);
        }

        match self.create_session(record) {
            Ok(created) => Ok(created),
            Err(error) if error.contains("UNIQUE constraint failed") => self
                .load_session(&session_id)?
                .ok_or_else(|| format!("session `{session_id}` missing after concurrent insert")),
            Err(error) => Err(error),
        }
    }

    pub fn load_session_route_binding(
        &self,
        route_session_id: &str,
    ) -> Result<Option<SessionRouteBindingRecord>, String> {
        let route_session_id = normalize_required_text(route_session_id, "route_session_id")?;
        let conn = self.open_connection()?;
        conn.query_row(
            "SELECT route_session_id, active_session_id, created_at, updated_at
             FROM session_route_bindings
             WHERE route_session_id = ?1",
            params![route_session_id],
            |row| {
                Ok(SessionRouteBindingRecord {
                    route_session_id: row.get(0)?,
                    active_session_id: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("load session route binding failed: {error}"))
    }

    pub fn upsert_session_route_binding(
        &self,
        route_session_id: &str,
        active_session_id: &str,
    ) -> Result<SessionRouteBindingRecord, String> {
        let route_session_id = normalize_required_text(route_session_id, "route_session_id")?;
        let active_session_id = normalize_required_text(active_session_id, "active_session_id")?;
        let ts = unix_ts_now();
        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session route binding transaction failed: {error}"))?;
        tx.execute(
            "INSERT INTO session_route_bindings(
                route_session_id,
                active_session_id,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(route_session_id) DO UPDATE SET
                active_session_id = excluded.active_session_id,
                updated_at = excluded.updated_at",
            params![route_session_id, active_session_id, ts, ts],
        )
        .map_err(|error| format!("upsert session route binding failed: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit session route binding transaction failed: {error}"))?;
        self.load_session_route_binding(route_session_id.as_str())?
            .ok_or_else(|| {
                format!("session route binding `{route_session_id}` disappeared after upsert")
            })
    }

    pub fn create_session_with_event(
        &self,
        request: CreateSessionWithEventRequest,
    ) -> Result<CreateSessionWithEventResult, String> {
        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session create transaction failed: {error}"))?;
        let event = Self::create_session_with_event_in_tx(&tx, request)?;
        let session_id = event.session_id.clone();
        tx.commit()
            .map_err(|error| format!("commit session create transaction failed: {error}"))?;

        let session = self
            .load_session(&session_id)?
            .ok_or_else(|| format!("session `{session_id}` disappeared after insert"))?;

        Ok(CreateSessionWithEventResult { session, event })
    }

    pub fn create_delegate_child_session_with_event_if_within_limit<T, F>(
        &self,
        parent_session_id: &str,
        max_active_children: usize,
        build_request: F,
    ) -> Result<(CreateSessionWithEventResult, T), String>
    where
        F: FnOnce(usize) -> Result<(CreateSessionWithEventRequest, T), String>,
    {
        let parent_session_id = normalize_required_text(parent_session_id, "parent_session_id")?;
        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open delegate child create transaction failed: {error}"))?;
        let active_children =
            Self::count_active_direct_children_with_conn(&tx, &parent_session_id)?;
        if active_children >= max_active_children {
            return Err(format!(
                "delegate_active_children_exceeded: active child count {active_children} reaches configured max_active_children {max_active_children}"
            ));
        }

        let (request, sidecar) = build_request(active_children)?;
        let request_parent_session_id = normalize_optional_text(
            request.session.parent_session_id.clone(),
        )
        .ok_or_else(|| "delegate child create request requires parent_session_id".to_owned())?;
        if request.session.kind != SessionKind::DelegateChild {
            return Err("delegate child create request requires kind `delegate_child`".to_owned());
        }
        if request_parent_session_id != parent_session_id {
            return Err(format!(
                "delegate child create request parent mismatch: expected `{parent_session_id}`, got `{request_parent_session_id}`"
            ));
        }

        let event = Self::create_session_with_event_in_tx(&tx, request)?;
        let session_id = event.session_id.clone();
        tx.commit()
            .map_err(|error| format!("commit delegate child create transaction failed: {error}"))?;

        let session = self
            .load_session(&session_id)?
            .ok_or_else(|| format!("session `{session_id}` disappeared after insert"))?;

        Ok((CreateSessionWithEventResult { session, event }, sidecar))
    }

    pub fn load_session(&self, session_id: &str) -> Result<Option<SessionRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::load_session_with_conn(&conn, &session_id)
    }

    pub(super) fn load_session_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<SessionRecord>, String> {
        let raw = conn
            .query_row(
                "SELECT session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error
                 FROM sessions
                 WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(RawSessionRecord {
                        session_id: row.get(0)?,
                        kind: row.get(1)?,
                        parent_session_id: row.get(2)?,
                        label: row.get(3)?,
                        state: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                        last_error: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session row failed: {error}"))?;
        raw.map(SessionRecord::try_from_raw).transpose()
    }

    pub(super) fn load_session_node_with_conn(
        conn: &Connection,
        node_id: &str,
    ) -> Result<Option<SessionNodeRecord>, String> {
        let raw = conn
            .query_row(
                "SELECT
                    node_id,
                    session_id,
                    parent_node_id,
                    node_kind,
                    role,
                    content,
                    session_turn_index,
                    metadata_json,
                    created_at
                 FROM session_nodes
                 WHERE node_id = ?1",
                params![node_id],
                |row| {
                    Ok(RawSessionNodeRecord {
                        node_id: row.get(0)?,
                        session_id: row.get(1)?,
                        parent_node_id: row.get(2)?,
                        node_kind: row.get(3)?,
                        role: row.get(4)?,
                        content: row.get(5)?,
                        session_turn_index: row.get(6)?,
                        metadata_json: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session node failed: {error}"))?;
        raw.map(SessionNodeRecord::try_from_raw).transpose()
    }

    pub(super) fn load_session_head_with_conn(
        conn: &Connection,
        session_id: &str,
        head_name: &str,
    ) -> Result<Option<SessionHeadRecord>, String> {
        let raw = conn
            .query_row(
                "SELECT session_id, head_name, node_id, head_mode, updated_at
                 FROM session_heads
                 WHERE session_id = ?1 AND head_name = ?2",
                params![session_id, head_name],
                |row| {
                    Ok(RawSessionHeadRecord {
                        session_id: row.get(0)?,
                        head_name: row.get(1)?,
                        node_id: row.get(2)?,
                        head_mode: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session head failed: {error}"))?;
        raw.map(SessionHeadRecord::from_raw).transpose()
    }

    pub(crate) fn list_session_heads_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<SessionHeadRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, head_name, node_id, head_mode, updated_at
                 FROM session_heads
                 WHERE session_id = ?1
                 ORDER BY head_name ASC",
            )
            .map_err(|error| format!("prepare session heads query failed: {error}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(RawSessionHeadRecord {
                    session_id: row.get(0)?,
                    head_name: row.get(1)?,
                    node_id: row.get(2)?,
                    head_mode: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(|error| format!("query session heads failed: {error}"))?;

        let mut heads = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| format!("decode session head row failed: {error}"))?;
            let head = SessionHeadRecord::from_raw(raw)?;
            heads.push(head);
        }
        Ok(heads)
    }

    fn list_session_node_children_with_conn(
        conn: &Connection,
        session_id: &str,
        parent_node_id: &str,
    ) -> Result<Vec<SessionNodeRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT
                    node_id,
                    session_id,
                    parent_node_id,
                    node_kind,
                    role,
                    content,
                    session_turn_index,
                    metadata_json,
                    created_at
                 FROM session_nodes
                 WHERE session_id = ?1 AND parent_node_id = ?2
                 ORDER BY created_at ASC, node_id ASC",
            )
            .map_err(|error| format!("prepare session node children query failed: {error}"))?;
        let rows = stmt
            .query_map(params![session_id, parent_node_id], |row| {
                Ok(RawSessionNodeRecord {
                    node_id: row.get(0)?,
                    session_id: row.get(1)?,
                    parent_node_id: row.get(2)?,
                    node_kind: row.get(3)?,
                    role: row.get(4)?,
                    content: row.get(5)?,
                    session_turn_index: row.get(6)?,
                    metadata_json: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(|error| format!("query session node children failed: {error}"))?;

        let mut nodes = Vec::new();
        for row in rows {
            let raw =
                row.map_err(|error| format!("decode session node child row failed: {error}"))?;
            nodes.push(SessionNodeRecord::try_from_raw(raw)?);
        }
        Ok(nodes)
    }

    pub(crate) fn load_session_path_for_head_with_conn(
        conn: &Connection,
        session_id: &str,
        head_name: &str,
    ) -> Result<Vec<SessionNodeRecord>, String> {
        let Some(head) = Self::load_session_head_with_conn(conn, session_id, head_name)? else {
            return Ok(Vec::new());
        };

        let mut path = Vec::new();
        let mut current_node_id = Some(head.node_id);
        while let Some(node_id) = current_node_id {
            let Some(node) = Self::load_session_node_with_conn(conn, &node_id)? else {
                break;
            };
            current_node_id = node.parent_node_id.clone();
            path.push(node);
        }
        path.reverse();
        Ok(path)
    }

    pub(crate) fn list_session_nodes_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<SessionNodeRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT
                    node_id,
                    session_id,
                    parent_node_id,
                    node_kind,
                    role,
                    content,
                    session_turn_index,
                    metadata_json,
                    created_at
                 FROM session_nodes
                 WHERE session_id = ?1
                 ORDER BY created_at ASC, node_id ASC",
            )
            .map_err(|error| format!("prepare session nodes query failed: {error}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(RawSessionNodeRecord {
                    node_id: row.get(0)?,
                    session_id: row.get(1)?,
                    parent_node_id: row.get(2)?,
                    node_kind: row.get(3)?,
                    role: row.get(4)?,
                    content: row.get(5)?,
                    session_turn_index: row.get(6)?,
                    metadata_json: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(|error| format!("query session nodes failed: {error}"))?;

        let mut nodes = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| format!("decode session node row failed: {error}"))?;
            nodes.push(SessionNodeRecord::try_from_raw(raw)?);
        }
        Ok(nodes)
    }

    pub(crate) fn list_session_artifacts_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<SessionArtifactRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT
                    artifact_id,
                    session_id,
                    artifact_type,
                    head_name,
                    anchor_node_id,
                    source_start_node_id,
                    source_end_node_id,
                    payload_json,
                    summary_text,
                    created_at
                 FROM session_artifacts
                 WHERE session_id = ?1
                 ORDER BY created_at ASC, artifact_id ASC",
            )
            .map_err(|error| format!("prepare session artifacts query failed: {error}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(RawSessionArtifactRecord {
                    artifact_id: row.get(0)?,
                    session_id: row.get(1)?,
                    artifact_type: row.get(2)?,
                    head_name: row.get(3)?,
                    anchor_node_id: row.get(4)?,
                    source_start_node_id: row.get(5)?,
                    source_end_node_id: row.get(6)?,
                    payload_json: row.get(7)?,
                    summary_text: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })
            .map_err(|error| format!("query session artifacts failed: {error}"))?;

        let mut artifacts = Vec::new();
        for row in rows {
            let raw =
                row.map_err(|error| format!("decode session artifact row failed: {error}"))?;
            artifacts.push(SessionArtifactRecord::try_from_raw(raw)?);
        }
        Ok(artifacts)
    }

    pub fn load_session_node(&self, node_id: &str) -> Result<Option<SessionNodeRecord>, String> {
        let node_id = normalize_required_text(node_id, "node_id")?;
        let conn = self.open_connection()?;
        Self::load_session_node_with_conn(&conn, &node_id)
    }

    pub fn load_session_head(
        &self,
        session_id: &str,
        head_name: &str,
    ) -> Result<Option<SessionHeadRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let head_name = normalize_required_text(head_name, "head_name")?;
        let conn = self.open_connection()?;
        Self::load_session_head_with_conn(&conn, &session_id, &head_name)
    }

    pub fn list_session_heads(&self, session_id: &str) -> Result<Vec<SessionHeadRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::list_session_heads_with_conn(&conn, &session_id)
    }

    pub fn list_session_node_children(
        &self,
        session_id: &str,
        parent_node_id: &str,
    ) -> Result<Vec<SessionNodeRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let parent_node_id = normalize_required_text(parent_node_id, "parent_node_id")?;
        let conn = self.open_connection()?;
        Self::list_session_node_children_with_conn(&conn, &session_id, &parent_node_id)
    }

    pub fn load_session_path_for_head(
        &self,
        session_id: &str,
        head_name: &str,
    ) -> Result<Vec<SessionNodeRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let head_name = normalize_required_text(head_name, "head_name")?;
        let conn = self.open_connection()?;
        Self::load_session_path_for_head_with_conn(&conn, &session_id, &head_name)
    }

    pub fn load_active_session_path(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionNodeRecord>, String> {
        self.load_session_path_for_head(session_id, ACTIVE_SESSION_HEAD_NAME)
    }

    pub fn set_session_head(
        &self,
        session_id: &str,
        head_name: &str,
        node_id: &str,
    ) -> Result<SessionHeadRecord, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let head_name = normalize_required_text(head_name, "head_name")?;
        let node_id = normalize_required_text(node_id, "node_id")?;
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session head transaction failed: {error}"))?;
        let node = Self::load_session_node_with_conn(&tx, &node_id)?
            .ok_or_else(|| format!("session node `{node_id}` not found"))?;
        if node.session_id != session_id {
            return Err(format!(
                "session node `{node_id}` belongs to `{}`, not `{session_id}`",
                node.session_id
            ));
        }
        let existing_head = Self::load_session_head_with_conn(&tx, &session_id, &head_name)?;
        let default_mode = SessionHeadMode::Live;
        let mut head_mode = existing_head.map(|head| head.mode).unwrap_or(default_mode);
        if head_name == ACTIVE_SESSION_HEAD_NAME {
            head_mode = SessionHeadMode::Live;
        }
        upsert_session_head_with_conn(&tx, &session_id, &head_name, &node_id, head_mode, ts)?;
        tx.commit()
            .map_err(|error| format!("commit session head transaction failed: {error}"))?;
        Self::load_session_head_with_conn(&self.open_connection()?, &session_id, &head_name)?
            .ok_or_else(|| format!("session head `{head_name}` missing after update"))
    }

    pub fn set_session_head_mode(
        &self,
        session_id: &str,
        head_name: &str,
        mode: SessionHeadMode,
    ) -> Result<SessionHeadRecord, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let head_name = normalize_required_text(head_name, "head_name")?;
        if head_name == ACTIVE_SESSION_HEAD_NAME && mode == SessionHeadMode::Pinned {
            return Err("active session head cannot be pinned".to_owned());
        }

        let ts = unix_ts_now();
        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session head mode transaction failed: {error}"))?;
        Self::load_session_head_with_conn(&tx, &session_id, &head_name)?
            .ok_or_else(|| format!("session head `{head_name}` not found"))?;
        tx.execute(
            "UPDATE session_heads
             SET head_mode = ?3, updated_at = ?4
             WHERE session_id = ?1 AND head_name = ?2",
            params![session_id, head_name, mode.as_str(), ts],
        )
        .map_err(|error| format!("update session head mode failed: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit session head mode transaction failed: {error}"))?;
        Self::load_session_head_with_conn(&self.open_connection()?, &session_id, &head_name)?
            .ok_or_else(|| format!("session head `{head_name}` missing after mode update"))
    }

    pub fn fork_session_head(
        &self,
        session_id: &str,
        source_node_id: &str,
        head_name: &str,
    ) -> Result<SessionHeadRecord, String> {
        self.set_session_head(session_id, head_name, source_node_id)
    }

    pub fn list_session_nodes(&self, session_id: &str) -> Result<Vec<SessionNodeRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::list_session_nodes_with_conn(&conn, &session_id)
    }

    pub fn create_session_artifact(
        &self,
        record: NewSessionArtifactRecord,
    ) -> Result<SessionArtifactRecord, String> {
        let artifact_id = normalize_required_text(&record.artifact_id, "artifact_id")?;
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        let head_name = normalize_optional_text(record.head_name);
        let anchor_node_id = normalize_optional_text(record.anchor_node_id);
        let source_start_node_id = normalize_optional_text(record.source_start_node_id);
        let source_end_node_id = normalize_optional_text(record.source_end_node_id);
        let summary_text = normalize_optional_text(record.summary_text);
        let payload_json = record.payload_json;
        let payload_text = serde_json::to_string(&payload_json)
            .map_err(|error| format!("encode session artifact payload failed: {error}"))?;
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session artifact transaction failed: {error}"))?;
        let session = Self::load_session_with_conn(&tx, &session_id)?
            .ok_or_else(|| format!("session `{session_id}` not found"))?;
        let _ = session;

        for maybe_node_id in [
            anchor_node_id.as_deref(),
            source_start_node_id.as_deref(),
            source_end_node_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let node = Self::load_session_node_with_conn(&tx, maybe_node_id)?
                .ok_or_else(|| format!("session node `{maybe_node_id}` not found"))?;
            if node.session_id != session_id {
                return Err(format!(
                    "session node `{maybe_node_id}` belongs to `{}`, not `{session_id}`",
                    node.session_id
                ));
            }
        }

        if let Some(head_name) = head_name.as_deref() {
            let head = Self::load_session_head_with_conn(&tx, &session_id, head_name)?
                .ok_or_else(|| format!("session head `{head_name}` not found"))?;
            let _ = head;
        }

        tx.execute(
            "INSERT INTO session_artifacts(
                artifact_id,
                session_id,
                artifact_type,
                head_name,
                anchor_node_id,
                source_start_node_id,
                source_end_node_id,
                payload_json,
                summary_text,
                created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                artifact_id,
                session_id,
                record.kind.as_str(),
                head_name.as_deref(),
                anchor_node_id.as_deref(),
                source_start_node_id.as_deref(),
                source_end_node_id.as_deref(),
                payload_text,
                summary_text.as_deref(),
                ts
            ],
        )
        .map_err(|error| format!("insert session artifact failed: {error}"))?;
        self.enforce_session_artifact_retention_with_conn(&tx, &session_id)?;
        tx.commit()
            .map_err(|error| format!("commit session artifact transaction failed: {error}"))?;

        self.list_session_artifacts(&session_id)?
            .into_iter()
            .find(|artifact| artifact.artifact_id == artifact_id)
            .ok_or_else(|| format!("session artifact `{artifact_id}` missing after insert"))
    }

    pub fn list_session_artifacts(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionArtifactRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::list_session_artifacts_with_conn(&conn, &session_id)
    }
}
