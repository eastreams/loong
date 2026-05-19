use super::*;

impl SessionRepository {
    pub fn load_session_summary(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::load_session_summary_with_conn(&conn, &session_id)
    }

    pub fn load_session_summary_with_legacy_fallback(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::load_session_summary_with_legacy_fallback_with_conn(&conn, &session_id)
    }

    pub fn latest_resumable_root_session_summary(
        &self,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        let conn = self.open_connection()?;
        Self::latest_resumable_root_session_summary_with_conn(&conn)
    }

    pub fn load_session_observation(
        &self,
        session_id: &str,
        recent_event_limit: usize,
        tail_after_id: Option<i64>,
        tail_page_limit: usize,
    ) -> Result<Option<SessionObservationRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session observation transaction failed: {error}"))?;
        let observation = Self::load_session_observation_with_conn(
            &tx,
            &session_id,
            recent_event_limit,
            tail_after_id,
            tail_page_limit,
        )?;
        tx.commit()
            .map_err(|error| format!("commit session observation transaction failed: {error}"))?;
        Ok(observation)
    }

    pub fn update_session_state(
        &self,
        session_id: &str,
        state: SessionState,
        last_error: Option<String>,
    ) -> Result<SessionRecord, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "UPDATE sessions
                 SET state = ?2, updated_at = ?3, last_error = ?4
                 WHERE session_id = ?1",
                params![
                    session_id,
                    state.as_str(),
                    unix_ts_now(),
                    normalize_optional_text(last_error),
                ],
            )
            .map_err(|error| format!("update session state failed: {error}"))?;
        if affected == 0 {
            return Err(format!("session `{session_id}` not found"));
        }
        self.load_session(&session_id)?
            .ok_or_else(|| format!("session `{session_id}` missing after update"))
    }

    pub fn update_session_state_if_current(
        &self,
        session_id: &str,
        expected_state: SessionState,
        next_state: SessionState,
        last_error: Option<String>,
    ) -> Result<Option<SessionRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "UPDATE sessions
                 SET state = ?3, updated_at = ?4, last_error = ?5
                 WHERE session_id = ?1 AND state = ?2",
                params![
                    session_id,
                    expected_state.as_str(),
                    next_state.as_str(),
                    unix_ts_now(),
                    normalize_optional_text(last_error),
                ],
            )
            .map_err(|error| format!("conditionally update session state failed: {error}"))?;
        if affected == 0 {
            return Ok(None);
        }
        self.load_session(&session_id)?
            .map(Some)
            .ok_or_else(|| format!("session `{session_id}` missing after conditional update"))
    }

    pub fn transition_session_with_event_if_current(
        &self,
        session_id: &str,
        request: TransitionSessionWithEventIfCurrentRequest,
    ) -> Result<Option<TransitionSessionWithEventResult>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let event_payload_json = request.event_payload_json;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session transition event payload failed: {error}"))?;
        let event_search_text =
            session_event_search_text(event_kind.as_str(), encoded_event_payload.as_str());
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session transition transaction failed: {error}"))?;
        let affected = tx
            .execute(
                "UPDATE sessions
                 SET state = ?3, updated_at = ?4, last_error = ?5
                 WHERE session_id = ?1 AND state = ?2",
                params![
                    session_id,
                    request.expected_state.as_str(),
                    request.next_state.as_str(),
                    ts,
                    last_error.as_deref(),
                ],
            )
            .map_err(|error| {
                format!("conditionally update session state in transition failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                event_search_text,
                ts
            ],
        )
        .map_err(|error| format!("insert session transition event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        let session = Self::load_session_with_conn(&tx, &session_id)?.ok_or_else(|| {
            format!("session `{session_id}` missing after conditional transition")
        })?;
        tx.commit()
            .map_err(|error| format!("commit session transition failed: {error}"))?;

        Ok(Some(TransitionSessionWithEventResult {
            session,
            event: SessionEventRecord {
                id: event_id,
                session_id,
                event_kind,
                actor_session_id,
                payload_json: event_payload_json,
                ts,
            },
        }))
    }

    pub fn transition_session_with_event_and_clear_terminal_outcome_if_current(
        &self,
        session_id: &str,
        request: TransitionSessionWithEventIfCurrentRequest,
    ) -> Result<Option<TransitionSessionWithEventResult>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let event_payload_json = request.event_payload_json;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session transition event payload failed: {error}"))?;
        let event_search_text =
            session_event_search_text(event_kind.as_str(), encoded_event_payload.as_str());
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session transition transaction failed: {error}"))?;
        let affected = tx
            .execute(
                "UPDATE sessions
                 SET state = ?3, updated_at = ?4, last_error = ?5
                 WHERE session_id = ?1 AND state = ?2",
                params![
                    session_id,
                    request.expected_state.as_str(),
                    request.next_state.as_str(),
                    ts,
                    last_error.as_deref(),
                ],
            )
            .map_err(|error| {
                format!("conditionally update session state in transition failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }
        tx.execute(
            "DELETE FROM session_terminal_outcomes WHERE session_id = ?1",
            params![session_id],
        )
        .map_err(|error| format!("clear session terminal outcome failed: {error}"))?;
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, search_text, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                event_search_text,
                ts
            ],
        )
        .map_err(|error| format!("insert session transition event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        let session = Self::load_session_with_conn(&tx, &session_id)?.ok_or_else(|| {
            format!("session `{session_id}` missing after conditional transition")
        })?;
        tx.commit()
            .map_err(|error| format!("commit session transition failed: {error}"))?;

        Ok(Some(TransitionSessionWithEventResult {
            session,
            event: SessionEventRecord {
                id: event_id,
                session_id,
                event_kind,
                actor_session_id,
                payload_json: event_payload_json,
                ts,
            },
        }))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>, String> {
        let conn = self.open_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error
                 FROM sessions
                 ORDER BY updated_at DESC, session_id ASC",
            )
            .map_err(|error| format!("prepare session list query failed: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
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
            })
            .map_err(|error| format!("query session list failed: {error}"))?;

        let mut sessions = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| format!("decode session row failed: {error}"))?;
            sessions.push(SessionRecord::try_from_raw(raw)?);
        }
        Ok(sessions)
    }

    pub fn list_visible_sessions(
        &self,
        current_session_id: &str,
    ) -> Result<Vec<SessionSummaryRecord>, String> {
        let current_session_id = normalize_required_text(current_session_id, "current_session_id")?;
        let conn = self.open_connection()?;
        let mut stmt = conn
            .prepare(
                "WITH RECURSIVE visible(session_id) AS (
                    SELECT session_id
                    FROM sessions
                    WHERE session_id = ?1
                    UNION
                    SELECT s.session_id
                    FROM sessions s
                    JOIN visible v ON s.parent_session_id = v.session_id
                 )
                 SELECT
                    s.session_id,
                    s.kind,
                    s.parent_session_id,
                    s.label,
                    s.state,
                    s.created_at,
                    s.updated_at,
                    s.last_error,
                    archived.archived_at,
                    COUNT(t.id) AS turn_count,
                    MAX(t.ts) AS last_turn_at
                 FROM sessions s
                 LEFT JOIN (
                    SELECT session_id, MAX(ts) AS archived_at
                    FROM session_events
                    WHERE event_kind = 'session_archived'
                    GROUP BY session_id
                 ) archived ON archived.session_id = s.session_id
                 JOIN visible v ON v.session_id = s.session_id
                 LEFT JOIN turns t ON t.session_id = s.session_id
                 GROUP BY
                    s.session_id,
                    s.kind,
                    s.parent_session_id,
                    s.label,
                    s.state,
                    s.created_at,
                    s.updated_at,
                    s.last_error,
                    archived.archived_at
                 ORDER BY s.updated_at DESC, s.session_id ASC",
            )
            .map_err(|error| format!("prepare visible session query failed: {error}"))?;
        let rows = stmt
            .query_map(params![current_session_id], |row| {
                Ok(RawSessionSummaryRecord {
                    session_id: row.get(0)?,
                    kind: row.get(1)?,
                    parent_session_id: row.get(2)?,
                    label: row.get(3)?,
                    state: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    last_error: row.get(7)?,
                    archived_at: row.get(8)?,
                    turn_count: row.get(9)?,
                    last_turn_at: row.get(10)?,
                })
            })
            .map_err(|error| format!("query visible sessions failed: {error}"))?;

        let mut sessions = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| format!("decode visible session row failed: {error}"))?;
            sessions.push(SessionSummaryRecord::try_from_raw(raw)?);
        }
        if !sessions
            .iter()
            .any(|session| session.session_id == current_session_id)
            && let Some(legacy) = self.infer_legacy_session_summary(&current_session_id)?
        {
            sessions.push(legacy);
            sort_session_summaries(&mut sessions);
        }
        Ok(sessions)
    }

    pub fn is_session_visible(
        &self,
        current_session_id: &str,
        target_session_id: &str,
    ) -> Result<bool, String> {
        let current_session_id = normalize_required_text(current_session_id, "current_session_id")?;
        let target_session_id = normalize_required_text(target_session_id, "target_session_id")?;
        if current_session_id == target_session_id {
            return Ok(true);
        }

        let mut seen = BTreeSet::new();
        let mut next_session_id = Some(target_session_id);
        while let Some(session_id) = next_session_id {
            if !seen.insert(session_id.to_owned()) {
                return Err(format!(
                    "session_lineage_cycle_detected: `{session_id}` reappeared while checking visibility"
                ));
            }
            let session = match self.load_session(&session_id)? {
                Some(session) => session,
                None => return Ok(false),
            };
            match session.parent_session_id {
                Some(parent_session_id) if parent_session_id == current_session_id => {
                    return Ok(true);
                }
                Some(parent_session_id) => next_session_id = Some(parent_session_id),
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    pub fn session_lineage_depth(&self, session_id: &str) -> Result<usize, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::session_lineage_depth_with_conn(&conn, &session_id)
    }

    pub fn count_active_direct_children(&self, parent_session_id: &str) -> Result<usize, String> {
        let parent_session_id = normalize_required_text(parent_session_id, "parent_session_id")?;
        let conn = self.open_connection()?;
        Self::count_active_direct_children_with_conn(&conn, &parent_session_id)
    }

    pub fn lineage_root_session_id(&self, session_id: &str) -> Result<Option<String>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::lineage_root_session_id_with_conn(&conn, &session_id)
    }

    pub(crate) fn list_all_events_with_conn(
        conn: &Connection,
        session_id: &str,
        page_limit: usize,
    ) -> Result<Vec<SessionEventRecord>, String> {
        if page_limit == 0 {
            return Err("page_limit must be >= 1".to_owned());
        }
        Self::drain_events_after_with_conn(conn, session_id, 0, page_limit)
    }

    pub(crate) fn session_lineage_depth_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<usize, String> {
        let mut seen = BTreeSet::new();
        let mut next_session_id = Some(session_id.to_owned());
        let mut depth = 0usize;

        while let Some(current_session_id) = next_session_id {
            if !seen.insert(current_session_id.clone()) {
                return Err(format!(
                    "session_lineage_cycle_detected: `{current_session_id}` reappeared while computing lineage depth"
                ));
            }
            let session = match Self::load_session_with_conn(conn, &current_session_id)? {
                Some(session) => session,
                None if depth == 0 => return Ok(0),
                None => {
                    return Err(format!(
                        "session_lineage_broken: missing parent row for `{current_session_id}`"
                    ));
                }
            };
            match session.parent_session_id {
                Some(parent_session_id) => {
                    depth += 1;
                    next_session_id = Some(parent_session_id);
                }
                None => return Ok(depth),
            }
        }

        Ok(depth)
    }

    pub(crate) fn lineage_root_session_id_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        let mut seen = BTreeSet::new();
        let mut next_session_id = Some(session_id.to_owned());

        while let Some(current_session_id) = next_session_id {
            if !seen.insert(current_session_id.clone()) {
                return Err(format!(
                    "session_lineage_cycle_detected: `{current_session_id}` reappeared while computing lineage root"
                ));
            }
            let session = match Self::load_session_with_conn(conn, &current_session_id)? {
                Some(session) => session,
                None if seen.len() == 1 => return Ok(None),
                None => {
                    return Err(format!(
                        "session_lineage_broken: missing parent row for `{current_session_id}`"
                    ));
                }
            };
            match session.parent_session_id {
                Some(parent_session_id) => next_session_id = Some(parent_session_id),
                None => return Ok(Some(session.session_id)),
            }
        }

        Ok(None)
    }

    pub fn list_recent_events(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::list_recent_events_with_conn(&conn, &session_id, limit)
    }

    pub fn load_latest_event_by_kind(
        &self,
        session_id: &str,
        event_kind: &str,
    ) -> Result<Option<SessionEventRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let event_kind = normalize_required_text(event_kind, "event_kind")?;
        let conn = self.open_connection()?;
        Self::load_latest_event_by_kind_with_conn(&conn, &session_id, &event_kind)
    }

    pub fn list_events_after(
        &self,
        session_id: &str,
        after_id: i64,
        limit: usize,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::list_events_after_with_conn(&conn, &session_id, after_id, limit)
    }

    pub fn list_delegate_lifecycle_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::list_delegate_lifecycle_events_with_conn(&conn, &session_id)
    }

    pub fn search_session_content(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let raw_query = normalize_required_text(query, "query")?;
        let normalized_query = normalize_search_text(raw_query.as_str());
        let conn = self.open_connection()?;
        Self::search_session_content_with_conn(&conn, &session_id, &normalized_query, limit)
    }

    pub fn list_all_events(
        &self,
        session_id: &str,
        page_limit: usize,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        if page_limit == 0 {
            return Err("page_limit must be >= 1".to_owned());
        }
        let conn = self.open_connection()?;
        Self::drain_events_after_with_conn(&conn, &session_id, 0, page_limit)
    }

    pub fn load_session_trajectory_read_snapshot(
        &self,
        session_id: &str,
        page_limit: usize,
    ) -> Result<Option<SessionTrajectoryReadSnapshot>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        if page_limit == 0 {
            return Err("page_limit must be >= 1".to_owned());
        }

        self.with_read_snapshot(|conn| {
            let Some(summary) =
                Self::load_session_summary_with_legacy_fallback_with_conn(conn, &session_id)?
            else {
                return Ok(None);
            };
            let lineage_root_session_id =
                Self::lineage_root_session_id_with_conn(conn, &session_id)?;
            let lineage_depth = Self::session_lineage_depth_with_conn(conn, &session_id)?;
            let turns = store::transcript_session_turns_paged_with_conn(
                conn,
                &session_id,
                SESSION_TRAJECTORY_TRANSCRIPT_PAGE_SIZE,
            )?;
            let events = Self::list_all_events_with_conn(conn, &session_id, page_limit)?;
            let approval_requests =
                Self::list_approval_requests_for_session_with_conn(conn, &session_id, None)?;
            let terminal_outcome = Self::load_terminal_outcome_with_conn(conn, &session_id)?;

            Ok(Some(SessionTrajectoryReadSnapshot {
                summary,
                lineage_root_session_id,
                lineage_depth,
                turns,
                events,
                approval_requests,
                terminal_outcome,
            }))
        })
    }
    pub(crate) fn load_session_summary_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        let raw = conn
            .query_row(
                "SELECT
                    s.session_id,
                    s.kind,
                    s.parent_session_id,
                    s.label,
                    s.state,
                    s.created_at,
                    s.updated_at,
                    s.last_error,
                    archived.archived_at,
                    COUNT(t.id) AS turn_count,
                    MAX(t.ts) AS last_turn_at
                 FROM sessions s
                 LEFT JOIN (
                    SELECT session_id, MAX(ts) AS archived_at
                    FROM session_events
                    WHERE event_kind = 'session_archived'
                    GROUP BY session_id
                 ) archived ON archived.session_id = s.session_id
                 LEFT JOIN turns t ON t.session_id = s.session_id
                 WHERE s.session_id = ?1
                 GROUP BY
                    s.session_id,
                    s.kind,
                    s.parent_session_id,
                    s.label,
                    s.state,
                    s.created_at,
                    s.updated_at,
                    s.last_error,
                    archived.archived_at",
                params![session_id],
                |row| {
                    Ok(RawSessionSummaryRecord {
                        session_id: row.get(0)?,
                        kind: row.get(1)?,
                        parent_session_id: row.get(2)?,
                        label: row.get(3)?,
                        state: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                        last_error: row.get(7)?,
                        archived_at: row.get(8)?,
                        turn_count: row.get(9)?,
                        last_turn_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session summary failed: {error}"))?;
        raw.map(SessionSummaryRecord::try_from_raw).transpose()
    }

    pub(crate) fn load_session_summary_with_legacy_fallback_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        if let Some(summary) = Self::load_session_summary_with_conn(conn, session_id)? {
            return Ok(Some(summary));
        }
        Self::infer_legacy_session_summary_with_conn(conn, session_id)
    }

    fn latest_resumable_root_session_summary_with_conn(
        conn: &Connection,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        let mut candidates = Self::list_resumable_root_session_summaries_with_conn(conn)?;
        sort_session_summaries(&mut candidates);
        Ok(candidates.into_iter().next())
    }

    fn list_resumable_root_session_summaries_with_conn(
        conn: &Connection,
    ) -> Result<Vec<SessionSummaryRecord>, String> {
        let mut candidates = Self::list_concrete_session_summaries_with_conn(conn)?;
        let legacy_candidates = Self::list_legacy_turn_only_session_summaries_with_conn(conn)?;

        candidates.retain(is_resumable_root_session_summary);
        for candidate in legacy_candidates {
            if is_resumable_root_session_summary(&candidate) {
                candidates.push(candidate);
            }
        }

        Ok(candidates)
    }

    fn list_concrete_session_summaries_with_conn(
        conn: &Connection,
    ) -> Result<Vec<SessionSummaryRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT
                    s.session_id,
                    s.kind,
                    s.parent_session_id,
                    s.label,
                    s.state,
                    s.created_at,
                    s.updated_at,
                    s.last_error,
                    archived.archived_at,
                    COUNT(t.id) AS turn_count,
                    MAX(t.ts) AS last_turn_at
                 FROM sessions s
                 LEFT JOIN (
                    SELECT session_id, MAX(ts) AS archived_at
                    FROM session_events
                    WHERE event_kind = 'session_archived'
                    GROUP BY session_id
                 ) archived ON archived.session_id = s.session_id
                 LEFT JOIN turns t ON t.session_id = s.session_id
                 WHERE s.kind = 'root'
                 GROUP BY
                    s.session_id,
                    s.kind,
                    s.parent_session_id,
                    s.label,
                    s.state,
                    s.created_at,
                    s.updated_at,
                    s.last_error,
                    archived.archived_at",
            )
            .map_err(|error| format!("prepare concrete session summary query failed: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RawSessionSummaryRecord {
                    session_id: row.get(0)?,
                    kind: row.get(1)?,
                    parent_session_id: row.get(2)?,
                    label: row.get(3)?,
                    state: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    last_error: row.get(7)?,
                    archived_at: row.get(8)?,
                    turn_count: row.get(9)?,
                    last_turn_at: row.get(10)?,
                })
            })
            .map_err(|error| format!("query concrete session summaries failed: {error}"))?;

        let mut sessions = Vec::new();
        for row in rows {
            let raw =
                row.map_err(|error| format!("decode concrete session summary failed: {error}"))?;
            let summary = SessionSummaryRecord::try_from_raw(raw)?;
            sessions.push(summary);
        }

        Ok(sessions)
    }

    fn list_legacy_turn_only_session_summaries_with_conn(
        conn: &Connection,
    ) -> Result<Vec<SessionSummaryRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT
                    t.session_id,
                    MIN(t.ts) AS created_at,
                    MAX(t.ts) AS updated_at,
                    COUNT(t.id) AS turn_count
                 FROM turns t
                 LEFT JOIN sessions s ON s.session_id = t.session_id
                 WHERE s.session_id IS NULL
                 GROUP BY t.session_id",
            )
            .map_err(|error| format!("prepare legacy session summary query failed: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| format!("query legacy session summaries failed: {error}"))?;

        let mut sessions = Vec::new();
        for row in rows {
            let decoded_row =
                row.map_err(|error| format!("decode legacy session summary failed: {error}"))?;
            let (session_id, created_at_value, updated_at_value, turn_count_value) = decoded_row;
            let created_at = created_at_value.unwrap_or_default();
            let updated_at = updated_at_value.unwrap_or(created_at);
            let bounded_turn_count = turn_count_value.max(0);
            let turn_count = bounded_turn_count as usize;
            let kind = infer_legacy_session_kind(&session_id);

            let summary = SessionSummaryRecord {
                session_id,
                kind,
                parent_session_id: None,
                label: None,
                state: SessionState::Ready,
                created_at,
                updated_at,
                archived_at: None,
                turn_count,
                last_turn_at: Some(updated_at),
                last_error: None,
            };
            sessions.push(summary);
        }

        Ok(sessions)
    }

    fn list_recent_events_with_conn(
        conn: &Connection,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, event_kind, actor_session_id, payload_json, ts
                 FROM session_events
                 WHERE session_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|error| format!("prepare session event query failed: {error}"))?;
        let rows = stmt
            .query_map(params![session_id, limit as i64], |row| {
                Ok(RawSessionEventRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_kind: row.get(2)?,
                    actor_session_id: row.get(3)?,
                    payload_json: row.get(4)?,
                    ts: row.get(5)?,
                })
            })
            .map_err(|error| format!("query session events failed: {error}"))?;

        let mut events = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| format!("decode session event row failed: {error}"))?;
            events.push(SessionEventRecord::try_from_raw(raw)?);
        }
        events.reverse();
        Ok(events)
    }

    fn load_latest_event_by_kind_with_conn(
        conn: &Connection,
        session_id: &str,
        event_kind: &str,
    ) -> Result<Option<SessionEventRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, event_kind, actor_session_id, payload_json, ts
                 FROM session_events
                 WHERE session_id = ?1 AND event_kind = ?2
                 ORDER BY id DESC
                 LIMIT 1",
            )
            .map_err(|error| format!("prepare latest session event query failed: {error}"))?;
        let raw = stmt
            .query_row(params![session_id, event_kind], |row| {
                Ok(RawSessionEventRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_kind: row.get(2)?,
                    actor_session_id: row.get(3)?,
                    payload_json: row.get(4)?,
                    ts: row.get(5)?,
                })
            })
            .optional()
            .map_err(|error| format!("query latest session event failed: {error}"))?;
        let raw = match raw {
            Some(raw) => raw,
            None => return Ok(None),
        };
        let event = SessionEventRecord::try_from_raw(raw)?;
        Ok(Some(event))
    }

    fn list_events_after_with_conn(
        conn: &Connection,
        session_id: &str,
        after_id: i64,
        limit: usize,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, event_kind, actor_session_id, payload_json, ts
                 FROM session_events
                 WHERE session_id = ?1 AND id > ?2
                 ORDER BY id ASC
                 LIMIT ?3",
            )
            .map_err(|error| format!("prepare session event tail query failed: {error}"))?;
        let rows = stmt
            .query_map(params![session_id, after_id, limit as i64], |row| {
                Ok(RawSessionEventRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_kind: row.get(2)?,
                    actor_session_id: row.get(3)?,
                    payload_json: row.get(4)?,
                    ts: row.get(5)?,
                })
            })
            .map_err(|error| format!("query session event tail failed: {error}"))?;

        let mut events = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| format!("decode session event row failed: {error}"))?;
            events.push(SessionEventRecord::try_from_raw(raw)?);
        }
        Ok(events)
    }

    pub(crate) fn list_delegate_lifecycle_events_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Vec<SessionEventRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, event_kind, actor_session_id, payload_json, ts
                 FROM session_events
                 WHERE session_id = ?1
                   AND event_kind IN (
                        'delegate_queued',
                        'delegate_started',
                        'delegate_cancel_requested'
                   )
                 ORDER BY id ASC",
            )
            .map_err(|error| format!("prepare delegate lifecycle event query failed: {error}"))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(RawSessionEventRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_kind: row.get(2)?,
                    actor_session_id: row.get(3)?,
                    payload_json: row.get(4)?,
                    ts: row.get(5)?,
                })
            })
            .map_err(|error| format!("query delegate lifecycle events failed: {error}"))?;

        let mut events = Vec::new();
        for row in rows {
            let raw = row
                .map_err(|error| format!("decode delegate lifecycle event row failed: {error}"))?;
            events.push(SessionEventRecord::try_from_raw(raw)?);
        }
        Ok(events)
    }

    fn search_session_content_with_conn(
        conn: &Connection,
        session_id: &str,
        normalized_query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchRecord>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let Some(match_query) = build_search_fts_query(normalized_query) else {
            return Ok(Vec::new());
        };

        let mut hits = Vec::new();
        let turn_hits =
            Self::search_session_turns_with_conn(conn, session_id, &match_query, limit)?;
        hits.extend(turn_hits);
        let event_hits =
            Self::search_session_events_with_conn(conn, session_id, &match_query, limit)?;
        hits.extend(event_hits);
        Ok(hits)
    }

    fn search_session_turns_with_conn(
        conn: &Connection,
        session_id: &str,
        match_query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT persisted_turn.id,
                        persisted_turn.session_id,
                        persisted_turn.role,
                        persisted_turn.content,
                        persisted_turn.ts
                 FROM memory_canonical_records_fts AS fts
                 JOIN memory_canonical_records AS record
                   ON record.record_id = fts.rowid
                 JOIN turns AS persisted_turn
                   ON persisted_turn.session_id = record.session_id
                  AND persisted_turn.session_turn_index = record.session_turn_index
                 WHERE memory_canonical_records_fts MATCH ?1
                   AND persisted_turn.session_id = ?2
                   AND record.kind IN ('user_turn', 'assistant_turn')
                 ORDER BY bm25(memory_canonical_records_fts),
                          persisted_turn.ts DESC,
                          persisted_turn.id DESC
                 LIMIT ?3",
            )
            .map_err(|error| format!("prepare session search turns query failed: {error}"))?;
        let rows = stmt
            .query_map(params![match_query, session_id, limit as i64], |row| {
                Ok(RawSessionSearchTurnRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    ts: row.get(4)?,
                })
            })
            .map_err(|error| format!("query session search turns failed: {error}"))?;

        let mut results = Vec::new();
        for row in rows {
            let raw =
                row.map_err(|error| format!("decode session search turn row failed: {error}"))?;
            results.push(SessionSearchRecord {
                session_id: raw.session_id,
                source_kind: SessionSearchSourceKind::Turn,
                source_id: raw.id,
                role: Some(raw.role),
                event_kind: None,
                content_text: raw.content,
                ts: raw.ts,
            });
        }
        Ok(results)
    }

    fn search_session_events_with_conn(
        conn: &Connection,
        session_id: &str,
        match_query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchRecord>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT event.id,
                        event.session_id,
                        event.event_kind,
                        event.payload_json,
                        event.ts
                 FROM session_events_fts AS fts
                 JOIN session_events AS event
                   ON event.id = fts.rowid
                 WHERE session_events_fts MATCH ?1
                   AND event.session_id = ?2
                 ORDER BY bm25(session_events_fts),
                          event.ts DESC,
                          event.id DESC
                 LIMIT ?3",
            )
            .map_err(|error| format!("prepare session search events query failed: {error}"))?;

        let rows = stmt
            .query_map(params![match_query, session_id, limit as i64], |row| {
                Ok(RawSessionSearchEventRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    event_kind: row.get(2)?,
                    payload_json: row.get(3)?,
                    ts: row.get(4)?,
                })
            })
            .map_err(|error| format!("query session search events failed: {error}"))?;

        let mut results = Vec::new();
        for row in rows {
            let raw =
                row.map_err(|error| format!("decode session search event row failed: {error}"))?;
            results.push(SessionSearchRecord {
                session_id: raw.session_id,
                source_kind: SessionSearchSourceKind::Event,
                source_id: raw.id,
                role: None,
                event_kind: Some(raw.event_kind.clone()),
                content_text: format!("event_kind={}\n{}", raw.event_kind, raw.payload_json),
                ts: raw.ts,
            });
        }
        Ok(results)
    }

    pub(crate) fn load_terminal_outcome_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<SessionTerminalOutcomeRecord>, String> {
        let raw = conn
            .query_row(
                "SELECT session_id, status, payload_json, frozen_result_json, recorded_at
                 FROM session_terminal_outcomes
                 WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(RawSessionTerminalOutcomeRecord {
                        session_id: row.get(0)?,
                        status: row.get(1)?,
                        payload_json: row.get(2)?,
                        frozen_result_json: row.get(3)?,
                        recorded_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session terminal outcome failed: {error}"))?;
        raw.map(SessionTerminalOutcomeRecord::try_from_raw)
            .transpose()
    }

    fn drain_events_after_with_conn(
        conn: &Connection,
        session_id: &str,
        after_id: i64,
        page_limit: usize,
    ) -> Result<Vec<SessionEventRecord>, String> {
        if page_limit == 0 {
            return Ok(Vec::new());
        }
        let mut next_after_id = after_id.max(0);
        let mut events = Vec::new();
        loop {
            let page =
                Self::list_events_after_with_conn(conn, session_id, next_after_id, page_limit)?;
            if page.is_empty() {
                break;
            }
            next_after_id = page.last().map(|event| event.id).unwrap_or(next_after_id);
            events.extend(page);
        }
        Ok(events)
    }

    pub(crate) fn load_session_observation_with_conn(
        conn: &Connection,
        session_id: &str,
        recent_event_limit: usize,
        tail_after_id: Option<i64>,
        tail_page_limit: usize,
    ) -> Result<Option<SessionObservationRecord>, String> {
        let Some(session) =
            Self::load_session_summary_with_legacy_fallback_with_conn(conn, session_id)?
        else {
            return Ok(None);
        };
        let recent_events =
            Self::list_recent_events_with_conn(conn, session_id, recent_event_limit)?;
        let terminal_outcome = Self::load_terminal_outcome_with_conn(conn, session_id)?;
        let tail_events = match tail_after_id {
            Some(after_id) => Self::drain_events_after_with_conn(
                conn,
                session_id,
                after_id.max(0),
                tail_page_limit,
            )?,
            None => Vec::new(),
        };
        Ok(Some(SessionObservationRecord {
            session,
            terminal_outcome,
            recent_events,
            tail_events,
        }))
    }

    fn infer_legacy_session_summary(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::infer_legacy_session_summary_with_conn(&conn, &session_id)
    }

    fn infer_legacy_session_summary_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        let aggregate = conn
            .query_row(
                "SELECT MIN(ts), MAX(ts), COUNT(id)
                 FROM turns
                 WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(|error| format!("load legacy session aggregate failed: {error}"))?;
        let (created_at, updated_at, turn_count) = aggregate;
        if turn_count <= 0 {
            return Ok(None);
        }

        let created_at = created_at.unwrap_or_default();
        let updated_at = updated_at.unwrap_or(created_at);
        let kind = infer_legacy_session_kind(session_id);
        Ok(Some(SessionSummaryRecord {
            session_id: session_id.to_owned(),
            kind,
            parent_session_id: None,
            label: None,
            state: SessionState::Ready,
            created_at,
            updated_at,
            archived_at: None,
            turn_count: turn_count.max(0) as usize,
            last_turn_at: Some(updated_at),
            last_error: None,
        }))
    }
}
