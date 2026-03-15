use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::memory;
use crate::memory::runtime_config::MemoryRuntimeConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Root,
    DelegateChild,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::DelegateChild => "delegate_child",
        }
    }

    fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "root" => Ok(Self::Root),
            "delegate_child" => Ok(Self::DelegateChild),
            _ => Err(format!("unknown session kind `{value}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Ready,
    Running,
    Completed,
    Failed,
    TimedOut,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
        }
    }

    fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "ready" => Ok(Self::Ready),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "timed_out" => Ok(Self::TimedOut),
            _ => Err(format!("unknown session state `{value}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_id: String,
    pub kind: SessionKind,
    pub parent_session_id: Option<String>,
    pub label: Option<String>,
    pub state: SessionState,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionEventRecord {
    pub id: i64,
    pub session_id: String,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub payload_json: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionTerminalOutcomeRecord {
    pub session_id: String,
    pub status: String,
    pub payload_json: Value,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRequestStatus {
    Pending,
    Approved,
    Executing,
    Executed,
    Denied,
    Expired,
    Cancelled,
}

impl ApprovalRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Executed => "executed",
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "executing" => Ok(Self::Executing),
            "executed" => Ok(Self::Executed),
            "denied" => Ok(Self::Denied),
            "expired" => Ok(Self::Expired),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("unknown approval request status `{value}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    ApproveOnce,
    ApproveAlways,
    Deny,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::ApproveAlways => "approve_always",
            Self::Deny => "deny",
        }
    }

    fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "approve_once" => Ok(Self::ApproveOnce),
            "approve_always" => Ok(Self::ApproveAlways),
            "deny" => Ok(Self::Deny),
            _ => Err(format!("unknown approval decision `{value}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequestRecord {
    pub approval_request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub approval_key: String,
    pub status: ApprovalRequestStatus,
    pub decision: Option<ApprovalDecision>,
    pub request_payload_json: Value,
    pub governance_snapshot_json: Value,
    pub requested_at: i64,
    pub resolved_at: Option<i64>,
    pub resolved_by_session_id: Option<String>,
    pub executed_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewApprovalRequestRecord {
    pub approval_request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub approval_key: String,
    pub request_payload_json: Value,
    pub governance_snapshot_json: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionApprovalRequestIfCurrentRequest {
    pub expected_status: ApprovalRequestStatus,
    pub next_status: ApprovalRequestStatus,
    pub decision: Option<ApprovalDecision>,
    pub resolved_by_session_id: Option<String>,
    pub executed_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalGrantRecord {
    pub scope_session_id: String,
    pub approval_key: String,
    pub created_by_session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewApprovalGrantRecord {
    pub scope_session_id: String,
    pub approval_key: String,
    pub created_by_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummaryRecord {
    pub session_id: String,
    pub kind: SessionKind,
    pub parent_session_id: Option<String>,
    pub label: Option<String>,
    pub state: SessionState,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub turn_count: usize,
    pub last_turn_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionObservationRecord {
    pub session: SessionSummaryRecord,
    pub terminal_outcome: Option<SessionTerminalOutcomeRecord>,
    pub recent_events: Vec<SessionEventRecord>,
    pub tail_events: Vec<SessionEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionRecord {
    pub session_id: String,
    pub kind: SessionKind,
    pub parent_session_id: Option<String>,
    pub label: Option<String>,
    pub state: SessionState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSessionEvent {
    pub session_id: String,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub payload_json: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateSessionWithEventRequest {
    pub session: NewSessionRecord,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub event_payload_json: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateSessionWithEventResult {
    pub session: SessionRecord,
    pub event: SessionEventRecord,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalizeSessionTerminalRequest {
    pub state: SessionState,
    pub last_error: Option<String>,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub event_payload_json: Value,
    pub outcome_status: String,
    pub outcome_payload_json: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalizeSessionTerminalResult {
    pub session: SessionRecord,
    pub event: SessionEventRecord,
    pub terminal_outcome: SessionTerminalOutcomeRecord,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionSessionWithEventIfCurrentRequest {
    pub expected_state: SessionState,
    pub next_state: SessionState,
    pub last_error: Option<String>,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub event_payload_json: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionSessionWithEventResult {
    pub session: SessionRecord,
    pub event: SessionEventRecord,
}

#[derive(Debug, Clone)]
pub struct SessionRepository {
    db_path: PathBuf,
}

impl SessionRepository {
    pub fn new(config: &MemoryRuntimeConfig) -> Result<Self, String> {
        let db_path = memory::ensure_memory_db_ready(config.sqlite_path.clone(), config)?;
        Ok(Self { db_path })
    }

    pub fn create_session(&self, record: NewSessionRecord) -> Result<SessionRecord, String> {
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        let parent_session_id = normalize_optional_text(record.parent_session_id);
        let label = normalize_optional_text(record.label);
        let ts = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
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

    pub fn create_session_with_event(
        &self,
        request: CreateSessionWithEventRequest,
    ) -> Result<CreateSessionWithEventResult, String> {
        let session_id = normalize_required_text(&request.session.session_id, "session_id")?;
        let parent_session_id = normalize_optional_text(request.session.parent_session_id);
        let label = normalize_optional_text(request.session.label);
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let event_payload_json = request.event_payload_json;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session event payload failed: {error}"))?;
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
                request.session.kind.as_str(),
                parent_session_id,
                label,
                request.session.state.as_str(),
                ts,
                ts,
            ],
        )
        .map_err(|error| format!("insert session row failed: {error}"))?;
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                ts
            ],
        )
        .map_err(|error| format!("insert session event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        tx.commit()
            .map_err(|error| format!("commit session create transaction failed: {error}"))?;

        let session = self
            .load_session(&session_id)?
            .ok_or_else(|| format!("session `{session_id}` disappeared after insert"))?;

        Ok(CreateSessionWithEventResult {
            session,
            event: SessionEventRecord {
                id: event_id,
                session_id,
                event_kind,
                actor_session_id,
                payload_json: event_payload_json,
                ts,
            },
        })
    }

    pub fn load_session(&self, session_id: &str) -> Result<Option<SessionRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
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
                session_id, event_kind, actor_session_id, payload_json, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                ts
            ],
        )
        .map_err(|error| format!("insert session transition event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        tx.commit()
            .map_err(|error| format!("commit session transition failed: {error}"))?;

        let session = self.load_session(&session_id)?.ok_or_else(|| {
            format!("session `{session_id}` missing after conditional transition")
        })?;

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
        let mut seen = BTreeSet::new();
        let mut depth = 0usize;
        let mut next_session_id = Some(session_id);

        while let Some(current_session_id) = next_session_id {
            if !seen.insert(current_session_id.clone()) {
                return Err(format!(
                    "session_lineage_cycle_detected: `{current_session_id}` reappeared while computing lineage depth"
                ));
            }
            let session = match self.load_session(&current_session_id)? {
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

    pub fn lineage_root_session_id(&self, session_id: &str) -> Result<Option<String>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let mut seen = BTreeSet::new();
        let mut next_session_id = Some(session_id);

        while let Some(current_session_id) = next_session_id {
            if !seen.insert(current_session_id.clone()) {
                return Err(format!(
                    "session_lineage_cycle_detected: `{current_session_id}` reappeared while computing lineage root"
                ));
            }
            let session = match self.load_session(&current_session_id)? {
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

    pub fn load_terminal_outcome(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionTerminalOutcomeRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        Self::load_terminal_outcome_with_conn(&conn, &session_id)
    }

    pub fn ensure_approval_request(
        &self,
        record: NewApprovalRequestRecord,
    ) -> Result<ApprovalRequestRecord, String> {
        let approval_request_id =
            normalize_required_text(&record.approval_request_id, "approval_request_id")?;
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        let turn_id = normalize_required_text(&record.turn_id, "turn_id")?;
        let tool_call_id = normalize_required_text(&record.tool_call_id, "tool_call_id")?;
        let tool_name = normalize_required_text(&record.tool_name, "tool_name")?;
        let approval_key = normalize_required_text(&record.approval_key, "approval_key")?;
        if self.load_session(&session_id)?.is_none() {
            return Err(format!("session `{session_id}` not found"));
        }

        let encoded_request_payload = serde_json::to_string(&record.request_payload_json)
            .map_err(|error| format!("encode approval request payload failed: {error}"))?;
        let encoded_governance_snapshot =
            serde_json::to_string(&record.governance_snapshot_json)
                .map_err(|error| format!("encode approval governance snapshot failed: {error}"))?;
        let requested_at = unix_ts_now();
        let conn = self.open_connection()?;
        match conn.execute(
            "INSERT INTO approval_requests(
                approval_request_id,
                session_id,
                turn_id,
                tool_call_id,
                tool_name,
                approval_key,
                status,
                decision,
                request_payload_json,
                governance_snapshot_json,
                requested_at,
                resolved_at,
                resolved_by_session_id,
                executed_at,
                last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, NULL, NULL, NULL, NULL)",
            params![
                approval_request_id,
                session_id,
                turn_id,
                tool_call_id,
                tool_name,
                approval_key,
                ApprovalRequestStatus::Pending.as_str(),
                encoded_request_payload,
                encoded_governance_snapshot,
                requested_at,
            ],
        ) {
            Ok(_) => {}
            Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
                return self
                    .load_approval_request(&approval_request_id)?
                    .ok_or_else(|| {
                        format!(
                            "approval request `{approval_request_id}` missing after concurrent insert"
                        )
                    });
            }
            Err(error) => return Err(format!("insert approval request row failed: {error}")),
        }

        self.load_approval_request(&approval_request_id)?
            .ok_or_else(|| {
                format!("approval request `{approval_request_id}` disappeared after insert")
            })
    }

    pub fn load_approval_request(
        &self,
        approval_request_id: &str,
    ) -> Result<Option<ApprovalRequestRecord>, String> {
        let approval_request_id =
            normalize_required_text(approval_request_id, "approval_request_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    approval_request_id,
                    session_id,
                    turn_id,
                    tool_call_id,
                    tool_name,
                    approval_key,
                    status,
                    decision,
                    request_payload_json,
                    governance_snapshot_json,
                    requested_at,
                    resolved_at,
                    resolved_by_session_id,
                    executed_at,
                    last_error
                 FROM approval_requests
                 WHERE approval_request_id = ?1",
                params![approval_request_id],
                |row| {
                    Ok(RawApprovalRequestRecord {
                        approval_request_id: row.get(0)?,
                        session_id: row.get(1)?,
                        turn_id: row.get(2)?,
                        tool_call_id: row.get(3)?,
                        tool_name: row.get(4)?,
                        approval_key: row.get(5)?,
                        status: row.get(6)?,
                        decision: row.get(7)?,
                        request_payload_json: row.get(8)?,
                        governance_snapshot_json: row.get(9)?,
                        requested_at: row.get(10)?,
                        resolved_at: row.get(11)?,
                        resolved_by_session_id: row.get(12)?,
                        executed_at: row.get(13)?,
                        last_error: row.get(14)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load approval request row failed: {error}"))?;
        raw.map(ApprovalRequestRecord::try_from_raw).transpose()
    }

    pub fn list_approval_requests_for_session(
        &self,
        session_id: &str,
        status: Option<ApprovalRequestStatus>,
    ) -> Result<Vec<ApprovalRequestRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        let mut requests = Vec::new();
        match status {
            Some(status) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            approval_request_id,
                            session_id,
                            turn_id,
                            tool_call_id,
                            tool_name,
                            approval_key,
                            status,
                            decision,
                            request_payload_json,
                            governance_snapshot_json,
                            requested_at,
                            resolved_at,
                            resolved_by_session_id,
                            executed_at,
                            last_error
                         FROM approval_requests
                         WHERE session_id = ?1 AND status = ?2
                         ORDER BY requested_at DESC, approval_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare approval request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map(params![session_id, status.as_str()], |row| {
                        Ok(RawApprovalRequestRecord {
                            approval_request_id: row.get(0)?,
                            session_id: row.get(1)?,
                            turn_id: row.get(2)?,
                            tool_call_id: row.get(3)?,
                            tool_name: row.get(4)?,
                            approval_key: row.get(5)?,
                            status: row.get(6)?,
                            decision: row.get(7)?,
                            request_payload_json: row.get(8)?,
                            governance_snapshot_json: row.get(9)?,
                            requested_at: row.get(10)?,
                            resolved_at: row.get(11)?,
                            resolved_by_session_id: row.get(12)?,
                            executed_at: row.get(13)?,
                            last_error: row.get(14)?,
                        })
                    })
                    .map_err(|error| format!("query approval request list failed: {error}"))?;
                for row in rows {
                    let raw = row
                        .map_err(|error| format!("decode approval request row failed: {error}"))?;
                    requests.push(ApprovalRequestRecord::try_from_raw(raw)?);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            approval_request_id,
                            session_id,
                            turn_id,
                            tool_call_id,
                            tool_name,
                            approval_key,
                            status,
                            decision,
                            request_payload_json,
                            governance_snapshot_json,
                            requested_at,
                            resolved_at,
                            resolved_by_session_id,
                            executed_at,
                            last_error
                         FROM approval_requests
                         WHERE session_id = ?1
                         ORDER BY requested_at DESC, approval_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare approval request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map(params![session_id], |row| {
                        Ok(RawApprovalRequestRecord {
                            approval_request_id: row.get(0)?,
                            session_id: row.get(1)?,
                            turn_id: row.get(2)?,
                            tool_call_id: row.get(3)?,
                            tool_name: row.get(4)?,
                            approval_key: row.get(5)?,
                            status: row.get(6)?,
                            decision: row.get(7)?,
                            request_payload_json: row.get(8)?,
                            governance_snapshot_json: row.get(9)?,
                            requested_at: row.get(10)?,
                            resolved_at: row.get(11)?,
                            resolved_by_session_id: row.get(12)?,
                            executed_at: row.get(13)?,
                            last_error: row.get(14)?,
                        })
                    })
                    .map_err(|error| format!("query approval request list failed: {error}"))?;
                for row in rows {
                    let raw = row
                        .map_err(|error| format!("decode approval request row failed: {error}"))?;
                    requests.push(ApprovalRequestRecord::try_from_raw(raw)?);
                }
            }
        }
        Ok(requests)
    }

    pub fn transition_approval_request_if_current(
        &self,
        approval_request_id: &str,
        request: TransitionApprovalRequestIfCurrentRequest,
    ) -> Result<Option<ApprovalRequestRecord>, String> {
        let approval_request_id =
            normalize_required_text(approval_request_id, "approval_request_id")?;
        let resolved_by_session_id = normalize_optional_text(request.resolved_by_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let decision = request.decision.map(ApprovalDecision::as_str);
        let resolution_ts = matches!(
            request.next_status,
            ApprovalRequestStatus::Approved | ApprovalRequestStatus::Denied
        )
        .then(unix_ts_now);
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "UPDATE approval_requests
                 SET status = ?3,
                     decision = CASE WHEN ?4 IS NULL THEN decision ELSE ?4 END,
                     resolved_at = CASE WHEN ?5 IS NULL THEN resolved_at ELSE ?5 END,
                     resolved_by_session_id = CASE WHEN ?6 IS NULL THEN resolved_by_session_id ELSE ?6 END,
                     executed_at = CASE WHEN ?7 IS NULL THEN executed_at ELSE ?7 END,
                     last_error = ?8
                 WHERE approval_request_id = ?1 AND status = ?2",
                params![
                    approval_request_id,
                    request.expected_status.as_str(),
                    request.next_status.as_str(),
                    decision,
                    resolution_ts,
                    resolved_by_session_id,
                    request.executed_at,
                    last_error,
                ],
            )
            .map_err(|error| format!("conditionally update approval request failed: {error}"))?;
        if affected == 0 {
            return Ok(None);
        }

        self.load_approval_request(&approval_request_id)?
            .map(Some)
            .ok_or_else(|| {
                format!("approval request `{approval_request_id}` missing after conditional update")
            })
    }

    pub fn upsert_approval_grant(
        &self,
        record: NewApprovalGrantRecord,
    ) -> Result<ApprovalGrantRecord, String> {
        let scope_session_id =
            normalize_required_text(&record.scope_session_id, "scope_session_id")?;
        let approval_key = normalize_required_text(&record.approval_key, "approval_key")?;
        if self.load_session(&scope_session_id)?.is_none() {
            return Err(format!("session `{scope_session_id}` not found"));
        }
        let created_by_session_id = normalize_optional_text(record.created_by_session_id);
        let ts = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO approval_grants(
                scope_session_id,
                approval_key,
                created_by_session_id,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(scope_session_id, approval_key) DO UPDATE SET
                created_by_session_id = COALESCE(excluded.created_by_session_id, approval_grants.created_by_session_id),
                updated_at = excluded.updated_at",
            params![scope_session_id, approval_key, created_by_session_id, ts, ts],
        )
        .map_err(|error| format!("upsert approval grant failed: {error}"))?;

        self.load_approval_grant(&scope_session_id, &approval_key)?
            .ok_or_else(|| {
                format!(
                    "approval grant `{}:{}` disappeared after upsert",
                    scope_session_id, approval_key
                )
            })
    }

    pub fn load_approval_grant(
        &self,
        scope_session_id: &str,
        approval_key: &str,
    ) -> Result<Option<ApprovalGrantRecord>, String> {
        let scope_session_id = normalize_required_text(scope_session_id, "scope_session_id")?;
        let approval_key = normalize_required_text(approval_key, "approval_key")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT scope_session_id, approval_key, created_by_session_id, created_at, updated_at
                 FROM approval_grants
                 WHERE scope_session_id = ?1 AND approval_key = ?2",
                params![scope_session_id, approval_key],
                |row| {
                    Ok(RawApprovalGrantRecord {
                        scope_session_id: row.get(0)?,
                        approval_key: row.get(1)?,
                        created_by_session_id: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load approval grant failed: {error}"))?;
        raw.map(ApprovalGrantRecord::try_from_raw).transpose()
    }

    pub fn upsert_terminal_outcome(
        &self,
        session_id: &str,
        status: &str,
        payload_json: Value,
    ) -> Result<SessionTerminalOutcomeRecord, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let status = normalize_required_text(status, "status")?;
        if self.load_session(&session_id)?.is_none() {
            return Err(format!("session `{session_id}` not found"));
        }

        let encoded_payload = serde_json::to_string(&payload_json)
            .map_err(|error| format!("encode session terminal outcome payload failed: {error}"))?;
        let recorded_at = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO session_terminal_outcomes(session_id, status, payload_json, recorded_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                payload_json = excluded.payload_json,
                recorded_at = excluded.recorded_at",
            params![session_id, status, encoded_payload, recorded_at],
        )
        .map_err(|error| format!("upsert session terminal outcome failed: {error}"))?;

        Ok(SessionTerminalOutcomeRecord {
            session_id,
            status,
            payload_json,
            recorded_at,
        })
    }

    pub fn finalize_session_terminal(
        &self,
        session_id: &str,
        request: FinalizeSessionTerminalRequest,
    ) -> Result<FinalizeSessionTerminalResult, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let outcome_status = normalize_required_text(&request.outcome_status, "outcome_status")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let event_payload_json = request.event_payload_json;
        let outcome_payload_json = request.outcome_payload_json;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session terminal event payload failed: {error}"))?;
        let encoded_outcome_payload = serde_json::to_string(&outcome_payload_json)
            .map_err(|error| format!("encode session terminal outcome payload failed: {error}"))?;
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn
            .transaction()
            .map_err(|error| format!("open session terminal transaction failed: {error}"))?;
        let affected = tx
            .execute(
                "UPDATE sessions
                 SET state = ?2, updated_at = ?3, last_error = ?4
                 WHERE session_id = ?1",
                params![
                    session_id,
                    request.state.as_str(),
                    ts,
                    last_error.as_deref(),
                ],
            )
            .map_err(|error| {
                format!("update session state in terminal finalize failed: {error}")
            })?;
        if affected == 0 {
            return Err(format!("session `{session_id}` not found"));
        }
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                ts
            ],
        )
        .map_err(|error| format!("insert session terminal event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO session_terminal_outcomes(session_id, status, payload_json, recorded_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                payload_json = excluded.payload_json,
                recorded_at = excluded.recorded_at",
            params![session_id, outcome_status, encoded_outcome_payload, ts],
        )
        .map_err(|error| format!("upsert session terminal outcome in finalize failed: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit session terminal finalize failed: {error}"))?;

        let session = self
            .load_session(&session_id)?
            .ok_or_else(|| format!("session `{session_id}` missing after terminal finalize"))?;

        Ok(FinalizeSessionTerminalResult {
            session,
            event: SessionEventRecord {
                id: event_id,
                session_id: session_id.clone(),
                event_kind,
                actor_session_id,
                payload_json: event_payload_json,
                ts,
            },
            terminal_outcome: SessionTerminalOutcomeRecord {
                session_id,
                status: outcome_status,
                payload_json: outcome_payload_json,
                recorded_at: ts,
            },
        })
    }

    pub fn finalize_session_terminal_if_current(
        &self,
        session_id: &str,
        expected_state: SessionState,
        request: FinalizeSessionTerminalRequest,
    ) -> Result<Option<FinalizeSessionTerminalResult>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let outcome_status = normalize_required_text(&request.outcome_status, "outcome_status")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let last_error = normalize_optional_text(request.last_error);
        let event_payload_json = request.event_payload_json;
        let outcome_payload_json = request.outcome_payload_json;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session terminal event payload failed: {error}"))?;
        let encoded_outcome_payload = serde_json::to_string(&outcome_payload_json)
            .map_err(|error| format!("encode session terminal outcome payload failed: {error}"))?;
        let ts = unix_ts_now();

        let mut conn = self.open_connection()?;
        let tx = conn.transaction().map_err(|error| {
            format!("open conditional session terminal transaction failed: {error}")
        })?;
        let affected = tx
            .execute(
                "UPDATE sessions
                 SET state = ?3, updated_at = ?4, last_error = ?5
                 WHERE session_id = ?1 AND state = ?2",
                params![
                    session_id,
                    expected_state.as_str(),
                    request.state.as_str(),
                    ts,
                    last_error.as_deref(),
                ],
            )
            .map_err(|error| {
                format!("conditionally update session state in terminal finalize failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                event_kind,
                actor_session_id.as_deref(),
                encoded_event_payload,
                ts
            ],
        )
        .map_err(|error| format!("insert conditional session terminal event failed: {error}"))?;
        let event_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO session_terminal_outcomes(session_id, status, payload_json, recorded_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                status = excluded.status,
                payload_json = excluded.payload_json,
                recorded_at = excluded.recorded_at",
            params![session_id, outcome_status, encoded_outcome_payload, ts],
        )
        .map_err(|error| {
            format!("upsert session terminal outcome in conditional finalize failed: {error}")
        })?;
        tx.commit().map_err(|error| {
            format!("commit conditional session terminal finalize failed: {error}")
        })?;

        let session = self.load_session(&session_id)?.ok_or_else(|| {
            format!("session `{session_id}` missing after conditional terminal finalize")
        })?;

        Ok(Some(FinalizeSessionTerminalResult {
            session,
            event: SessionEventRecord {
                id: event_id,
                session_id: session_id.clone(),
                event_kind,
                actor_session_id,
                payload_json: event_payload_json,
                ts,
            },
            terminal_outcome: SessionTerminalOutcomeRecord {
                session_id,
                status: outcome_status,
                payload_json: outcome_payload_json,
                recorded_at: ts,
            },
        }))
    }

    pub fn append_event(&self, event: NewSessionEvent) -> Result<SessionEventRecord, String> {
        let session_id = normalize_required_text(&event.session_id, "session_id")?;
        let event_kind = normalize_required_text(&event.event_kind, "event_kind")?;

        let ts = unix_ts_now();
        let payload_json = serde_json::to_string(&event.payload_json)
            .map_err(|error| format!("encode session event payload failed: {error}"))?;
        let actor_session_id = normalize_optional_text(event.actor_session_id);

        let conn = self.open_connection()?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check session exists failed: {error}"))?;
        if !exists {
            return Err(format!("session `{session_id}` not found"));
        }

        conn.execute(
            "INSERT INTO session_events(
                session_id, event_kind, actor_session_id, payload_json, ts
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, event_kind, actor_session_id, payload_json, ts],
        )
        .map_err(|error| format!("insert session event failed: {error}"))?;

        Ok(SessionEventRecord {
            id: conn.last_insert_rowid(),
            session_id,
            event_kind,
            actor_session_id,
            payload_json: event.payload_json,
            ts,
        })
    }

    fn open_connection(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path)
            .map_err(|error| format!("open session repository sqlite db failed: {error}"))
    }

    fn load_session_summary_with_conn(
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

    fn load_session_summary_with_legacy_fallback_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, String> {
        if let Some(summary) = Self::load_session_summary_with_conn(conn, session_id)? {
            return Ok(Some(summary));
        }
        Self::infer_legacy_session_summary_with_conn(conn, session_id)
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

    fn list_delegate_lifecycle_events_with_conn(
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

    fn load_terminal_outcome_with_conn(
        conn: &Connection,
        session_id: &str,
    ) -> Result<Option<SessionTerminalOutcomeRecord>, String> {
        let raw = conn
            .query_row(
                "SELECT session_id, status, payload_json, recorded_at
                 FROM session_terminal_outcomes
                 WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(RawSessionTerminalOutcomeRecord {
                        session_id: row.get(0)?,
                        status: row.get(1)?,
                        payload_json: row.get(2)?,
                        recorded_at: row.get(3)?,
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

    fn load_session_observation_with_conn(
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

#[derive(Debug)]
struct RawSessionRecord {
    session_id: String,
    kind: String,
    parent_session_id: Option<String>,
    label: Option<String>,
    state: String,
    created_at: i64,
    updated_at: i64,
    last_error: Option<String>,
}

#[derive(Debug)]
struct RawSessionSummaryRecord {
    session_id: String,
    kind: String,
    parent_session_id: Option<String>,
    label: Option<String>,
    state: String,
    created_at: i64,
    updated_at: i64,
    last_error: Option<String>,
    archived_at: Option<i64>,
    turn_count: i64,
    last_turn_at: Option<i64>,
}

#[derive(Debug)]
struct RawSessionEventRecord {
    id: i64,
    session_id: String,
    event_kind: String,
    actor_session_id: Option<String>,
    payload_json: String,
    ts: i64,
}

#[derive(Debug)]
struct RawSessionTerminalOutcomeRecord {
    session_id: String,
    status: String,
    payload_json: String,
    recorded_at: i64,
}

#[derive(Debug)]
struct RawApprovalRequestRecord {
    approval_request_id: String,
    session_id: String,
    turn_id: String,
    tool_call_id: String,
    tool_name: String,
    approval_key: String,
    status: String,
    decision: Option<String>,
    request_payload_json: String,
    governance_snapshot_json: String,
    requested_at: i64,
    resolved_at: Option<i64>,
    resolved_by_session_id: Option<String>,
    executed_at: Option<i64>,
    last_error: Option<String>,
}

#[derive(Debug)]
struct RawApprovalGrantRecord {
    scope_session_id: String,
    approval_key: String,
    created_by_session_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl SessionRecord {
    fn try_from_raw(raw: RawSessionRecord) -> Result<Self, String> {
        Ok(Self {
            session_id: raw.session_id,
            kind: SessionKind::from_db(&raw.kind)?,
            parent_session_id: raw.parent_session_id,
            label: raw.label,
            state: SessionState::from_db(&raw.state)?,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            last_error: raw.last_error,
        })
    }
}

impl SessionSummaryRecord {
    fn try_from_raw(raw: RawSessionSummaryRecord) -> Result<Self, String> {
        Ok(Self {
            session_id: raw.session_id,
            kind: SessionKind::from_db(&raw.kind)?,
            parent_session_id: raw.parent_session_id,
            label: raw.label,
            state: SessionState::from_db(&raw.state)?,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            archived_at: raw.archived_at,
            turn_count: raw.turn_count.max(0) as usize,
            last_turn_at: raw.last_turn_at,
            last_error: raw.last_error,
        })
    }
}

impl SessionEventRecord {
    fn try_from_raw(raw: RawSessionEventRecord) -> Result<Self, String> {
        Ok(Self {
            id: raw.id,
            session_id: raw.session_id,
            event_kind: raw.event_kind,
            actor_session_id: raw.actor_session_id,
            payload_json: serde_json::from_str(&raw.payload_json)
                .map_err(|error| format!("decode session event payload failed: {error}"))?,
            ts: raw.ts,
        })
    }
}

impl SessionTerminalOutcomeRecord {
    fn try_from_raw(raw: RawSessionTerminalOutcomeRecord) -> Result<Self, String> {
        Ok(Self {
            session_id: raw.session_id,
            status: raw.status,
            payload_json: serde_json::from_str(&raw.payload_json).map_err(|error| {
                format!("decode session terminal outcome payload failed: {error}")
            })?,
            recorded_at: raw.recorded_at,
        })
    }
}

impl ApprovalRequestRecord {
    fn try_from_raw(raw: RawApprovalRequestRecord) -> Result<Self, String> {
        Ok(Self {
            approval_request_id: raw.approval_request_id,
            session_id: raw.session_id,
            turn_id: raw.turn_id,
            tool_call_id: raw.tool_call_id,
            tool_name: raw.tool_name,
            approval_key: raw.approval_key,
            status: ApprovalRequestStatus::from_db(&raw.status)?,
            decision: raw
                .decision
                .as_deref()
                .map(ApprovalDecision::from_db)
                .transpose()?,
            request_payload_json: serde_json::from_str(&raw.request_payload_json)
                .map_err(|error| format!("decode approval request payload failed: {error}"))?,
            governance_snapshot_json: serde_json::from_str(&raw.governance_snapshot_json)
                .map_err(|error| format!("decode approval governance snapshot failed: {error}"))?,
            requested_at: raw.requested_at,
            resolved_at: raw.resolved_at,
            resolved_by_session_id: raw.resolved_by_session_id,
            executed_at: raw.executed_at,
            last_error: raw.last_error,
        })
    }
}

impl ApprovalGrantRecord {
    fn try_from_raw(raw: RawApprovalGrantRecord) -> Result<Self, String> {
        Ok(Self {
            scope_session_id: raw.scope_session_id,
            approval_key: raw.approval_key,
            created_by_session_id: raw.created_by_session_id,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        })
    }
}

fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("session repository requires {field_name}"));
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn unix_ts_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn infer_legacy_session_kind(session_id: &str) -> SessionKind {
    if session_id.starts_with("delegate:") {
        SessionKind::DelegateChild
    } else {
        SessionKind::Root
    }
}

fn sort_session_summaries(sessions: &mut [SessionSummaryRecord]) {
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use crate::memory::append_turn_direct;
    use crate::memory::runtime_config::MemoryRuntimeConfig;

    use super::*;

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-session-repository-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    #[test]
    fn session_repository_creates_and_loads_session_rows() {
        let config = isolated_memory_config("create-load");
        let repo = SessionRepository::new(&config).expect("repository");
        let created = repo
            .create_session(NewSessionRecord {
                session_id: "root-session".to_owned(),
                kind: SessionKind::Root,
                parent_session_id: None,
                label: Some("Root".to_owned()),
                state: SessionState::Ready,
            })
            .expect("create session");

        assert_eq!(created.session_id, "root-session");
        assert_eq!(created.kind, SessionKind::Root);
        assert_eq!(created.state, SessionState::Ready);

        let loaded = repo
            .load_session("root-session")
            .expect("load session")
            .expect("session row");
        assert_eq!(loaded.session_id, "root-session");
        assert_eq!(loaded.label.as_deref(), Some("Root"));
        assert_eq!(loaded.parent_session_id, None);
    }

    #[test]
    fn session_repository_updates_state_and_last_error() {
        let config = isolated_memory_config("update-state");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create session");

        let updated = repo
            .update_session_state(
                "child-session",
                SessionState::Failed,
                Some("tool timeout".to_owned()),
            )
            .expect("update session state");
        assert_eq!(updated.state, SessionState::Failed);
        assert_eq!(updated.last_error.as_deref(), Some("tool timeout"));
    }

    #[test]
    fn session_repository_conditional_state_update_requires_expected_state() {
        let config = isolated_memory_config("update-state-if-current");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create session");

        let updated = repo
            .update_session_state_if_current(
                "child-session",
                SessionState::Ready,
                SessionState::Running,
                None,
            )
            .expect("conditional update should succeed");
        assert!(updated.is_none());

        let loaded = repo
            .load_session("child-session")
            .expect("load session")
            .expect("session row");
        assert_eq!(loaded.state, SessionState::Completed);
    }

    #[test]
    fn transition_session_with_event_if_current_writes_state_and_event_together() {
        let config = isolated_memory_config("transition-session-with-event");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");

        let transitioned = repo
            .transition_session_with_event_if_current(
                "child-session",
                TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Ready,
                    next_state: SessionState::Running,
                    last_error: None,
                    event_kind: "delegate_started".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({
                        "task": "child task",
                        "timeout_seconds": 60
                    }),
                },
            )
            .expect("transition session with event")
            .expect("transition result");

        assert_eq!(transitioned.session.state, SessionState::Running);
        assert_eq!(transitioned.session.last_error, None);
        assert_eq!(transitioned.event.event_kind, "delegate_started");
        assert_eq!(
            transitioned.event.actor_session_id.as_deref(),
            Some("root-session")
        );

        let child = repo
            .load_session("child-session")
            .expect("load child")
            .expect("child row");
        assert_eq!(child.state, SessionState::Running);

        let events = repo
            .list_recent_events("child-session", 10)
            .expect("list events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_kind, "delegate_started");
    }

    #[test]
    fn transition_session_with_event_if_current_rolls_back_state_when_event_insert_fails() {
        let config = isolated_memory_config("transition-session-with-event-rollback");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");

        let conn = repo.open_connection().expect("open connection");
        conn.execute("DROP TABLE session_events", [])
            .expect("drop session_events table");

        let error = repo
            .transition_session_with_event_if_current(
                "child-session",
                TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Ready,
                    next_state: SessionState::Running,
                    last_error: None,
                    event_kind: "delegate_started".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({
                        "task": "child task",
                        "timeout_seconds": 60
                    }),
                },
            )
            .expect_err("transition should fail when event insert fails");
        assert!(error.contains("insert session transition event failed"));

        let child = repo
            .load_session("child-session")
            .expect("load child")
            .expect("child row");
        assert_eq!(child.state, SessionState::Ready);

        let events_error = repo
            .list_recent_events("child-session", 10)
            .expect_err("list events should fail after dropping table");
        assert!(events_error.contains("prepare session event query failed"));
    }

    #[test]
    fn session_repository_ensure_session_is_idempotent() {
        let config = isolated_memory_config("ensure-session");
        let repo = SessionRepository::new(&config).expect("repository");

        let first = repo
            .ensure_session(NewSessionRecord {
                session_id: "root-session".to_owned(),
                kind: SessionKind::Root,
                parent_session_id: None,
                label: Some("Root".to_owned()),
                state: SessionState::Ready,
            })
            .expect("ensure root session");
        let second = repo
            .ensure_session(NewSessionRecord {
                session_id: "root-session".to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("other-parent".to_owned()),
                label: Some("Ignored".to_owned()),
                state: SessionState::Failed,
            })
            .expect("ensure existing session");

        assert_eq!(first.session_id, second.session_id);
        assert_eq!(second.kind, SessionKind::Root);
        assert_eq!(second.parent_session_id, None);
        assert_eq!(second.label.as_deref(), Some("Root"));
        assert_eq!(repo.list_sessions().expect("list sessions").len(), 1);
    }

    #[test]
    fn create_session_with_event_writes_session_and_event_together() {
        let config = isolated_memory_config("create-session-with-event");
        let repo = SessionRepository::new(&config).expect("repository");

        let created = repo
            .create_session_with_event(CreateSessionWithEventRequest {
                session: NewSessionRecord {
                    session_id: "child-session".to_owned(),
                    kind: SessionKind::DelegateChild,
                    parent_session_id: Some("root-session".to_owned()),
                    label: Some("Child".to_owned()),
                    state: SessionState::Ready,
                },
                event_kind: "delegate_queued".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "task": "child task",
                    "timeout_seconds": 60
                }),
            })
            .expect("create session with queued event");

        assert_eq!(created.session.state, SessionState::Ready);
        assert_eq!(
            created.session.parent_session_id.as_deref(),
            Some("root-session")
        );
        assert_eq!(created.event.event_kind, "delegate_queued");
        assert_eq!(
            created.event.actor_session_id.as_deref(),
            Some("root-session")
        );

        let sessions = repo.list_sessions().expect("list sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "child-session");

        let events = repo
            .list_recent_events("child-session", 10)
            .expect("list child events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_kind, "delegate_queued");
    }

    #[test]
    fn create_session_with_event_rolls_back_session_when_event_insert_fails() {
        let config = isolated_memory_config("create-session-with-event-rollback");
        let repo = SessionRepository::new(&config).expect("repository");
        let conn = repo.open_connection().expect("open connection");
        conn.execute(
            "CREATE TRIGGER fail_create_session_event
             BEFORE INSERT ON session_events
             BEGIN
                SELECT RAISE(FAIL, 'forced create session event failure');
             END;",
            [],
        )
        .expect("create session event failure trigger");

        let error = repo
            .create_session_with_event(CreateSessionWithEventRequest {
                session: NewSessionRecord {
                    session_id: "child-session".to_owned(),
                    kind: SessionKind::DelegateChild,
                    parent_session_id: Some("root-session".to_owned()),
                    label: Some("Child".to_owned()),
                    state: SessionState::Ready,
                },
                event_kind: "delegate_queued".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "task": "child task",
                    "timeout_seconds": 60
                }),
            })
            .expect_err("create session with event should fail when event insert fails");
        assert!(error.contains("insert session event failed"));

        assert!(
            repo.load_session("child-session")
                .expect("load child after rollback")
                .is_none()
        );
    }

    #[test]
    fn session_repository_lists_parent_child_relationships() {
        let config = isolated_memory_config("list-relationships");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({"depth": 1}),
        })
        .expect("append event");

        let sessions = repo.list_sessions().expect("list sessions");
        assert_eq!(sessions.len(), 2);
        let child = sessions
            .iter()
            .find(|session| session.session_id == "child-session")
            .expect("child session");
        assert_eq!(child.parent_session_id.as_deref(), Some("root-session"));
        assert_eq!(child.kind, SessionKind::DelegateChild);
    }

    #[test]
    fn list_visible_sessions_infers_legacy_rows_from_turn_history_without_backfill() {
        let config = isolated_memory_config("legacy-visible-sessions");
        append_turn_direct("telegram:123", "user", "hello", &config).expect("append user turn");
        append_turn_direct("telegram:123", "assistant", "world", &config)
            .expect("append assistant turn");

        let repo = SessionRepository::new(&config).expect("repository");
        let sessions = repo
            .list_visible_sessions("telegram:123")
            .expect("list visible sessions");

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.session_id, "telegram:123");
        assert_eq!(session.kind, SessionKind::Root);
        assert_eq!(session.parent_session_id, None);
        assert_eq!(session.turn_count, 2);
        assert!(session.last_turn_at.is_some());

        assert!(
            repo.load_session("telegram:123")
                .expect("load legacy session")
                .is_none()
        );
        assert!(
            repo.list_sessions()
                .expect("list concrete sessions")
                .is_empty()
        );
    }

    #[test]
    fn inferred_legacy_session_kind_uses_known_prefixes() {
        let config = isolated_memory_config("legacy-kind-prefixes");
        append_turn_direct("delegate:legacy-child", "assistant", "done", &config)
            .expect("append delegate turn");
        append_turn_direct("telegram:456", "user", "ping", &config).expect("append telegram turn");

        let repo = SessionRepository::new(&config).expect("repository");
        let delegate_session = repo
            .list_visible_sessions("delegate:legacy-child")
            .expect("list delegate legacy session")
            .into_iter()
            .find(|session| session.session_id == "delegate:legacy-child")
            .expect("delegate legacy session");
        assert_eq!(delegate_session.kind, SessionKind::DelegateChild);

        let telegram_session = repo
            .list_visible_sessions("telegram:456")
            .expect("list telegram legacy session")
            .into_iter()
            .find(|session| session.session_id == "telegram:456")
            .expect("telegram legacy session");
        assert_eq!(telegram_session.kind, SessionKind::Root);
    }

    #[test]
    fn session_lineage_depth_counts_root_child_and_grandchild() {
        let config = isolated_memory_config("lineage-depth");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "grandchild-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("child-session".to_owned()),
            label: Some("Grandchild".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create grandchild");

        assert_eq!(
            repo.session_lineage_depth("root-session")
                .expect("root depth"),
            0
        );
        assert_eq!(
            repo.session_lineage_depth("child-session")
                .expect("child depth"),
            1
        );
        assert_eq!(
            repo.session_lineage_depth("grandchild-session")
                .expect("grandchild depth"),
            2
        );
    }

    #[test]
    fn lineage_root_session_id_returns_root_for_delegate_descendants() {
        let config = isolated_memory_config("lineage-root");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "grandchild-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("child-session".to_owned()),
            label: Some("Grandchild".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create grandchild");

        assert_eq!(
            repo.lineage_root_session_id("root-session")
                .expect("root lineage root"),
            Some("root-session".to_owned())
        );
        assert_eq!(
            repo.lineage_root_session_id("grandchild-session")
                .expect("grandchild lineage root"),
            Some("root-session".to_owned())
        );
        assert_eq!(
            repo.lineage_root_session_id("missing-session")
                .expect("missing lineage root"),
            None
        );
    }

    #[test]
    fn list_visible_sessions_includes_descendant_delegate_chain() {
        let config = isolated_memory_config("descendant-visibility");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "grandchild-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("child-session".to_owned()),
            label: Some("Grandchild".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create grandchild");

        let visible = repo
            .list_visible_sessions("root-session")
            .expect("visible sessions");
        let ids: Vec<&str> = visible
            .iter()
            .map(|session| session.session_id.as_str())
            .collect();
        assert!(ids.contains(&"root-session"));
        assert!(ids.contains(&"child-session"));
        assert!(ids.contains(&"grandchild-session"));
        assert!(
            repo.is_session_visible("root-session", "grandchild-session")
                .expect("root should see grandchild")
        );
    }

    #[test]
    fn session_terminal_outcome_round_trips_payload() {
        let config = isolated_memory_config("terminal-outcome-round-trip");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create child");

        repo.upsert_terminal_outcome(
            "child-session",
            "ok",
            json!({
                "child_session_id": "child-session",
                "final_output": "done",
                "duration_ms": 12
            }),
        )
        .expect("upsert terminal outcome");

        let outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome")
            .expect("terminal outcome row");

        assert_eq!(outcome.session_id, "child-session");
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload_json["final_output"], "done");
        assert!(outcome.recorded_at > 0);
    }

    #[test]
    fn session_terminal_outcome_upsert_replaces_existing_row() {
        let config = isolated_memory_config("terminal-outcome-upsert");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Failed,
        })
        .expect("create child");

        repo.upsert_terminal_outcome(
            "child-session",
            "error",
            json!({
                "error": "first"
            }),
        )
        .expect("upsert first terminal outcome");
        repo.upsert_terminal_outcome(
            "child-session",
            "timeout",
            json!({
                "error": "delegate_timeout"
            }),
        )
        .expect("upsert second terminal outcome");

        let outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome")
            .expect("terminal outcome row");
        assert_eq!(outcome.status, "timeout");
        assert_eq!(outcome.payload_json["error"], "delegate_timeout");
    }

    #[test]
    fn finalize_session_terminal_writes_state_event_and_outcome_together() {
        let config = isolated_memory_config("finalize-session-terminal");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        let finalized = repo
            .finalize_session_terminal(
                "child-session",
                FinalizeSessionTerminalRequest {
                    state: SessionState::Completed,
                    last_error: None,
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({
                        "turn_count": 2,
                        "duration_ms": 15
                    }),
                    outcome_status: "ok".to_owned(),
                    outcome_payload_json: json!({
                        "child_session_id": "child-session",
                        "final_output": "done",
                        "turn_count": 2,
                        "duration_ms": 15
                    }),
                },
            )
            .expect("finalize session");

        assert_eq!(finalized.session.state, SessionState::Completed);
        assert_eq!(finalized.session.last_error, None);
        assert_eq!(finalized.event.event_kind, "delegate_completed");
        assert_eq!(
            finalized.event.actor_session_id.as_deref(),
            Some("root-session")
        );
        assert_eq!(finalized.terminal_outcome.status, "ok");
        assert_eq!(finalized.session.updated_at, finalized.event.ts);
        assert_eq!(finalized.event.ts, finalized.terminal_outcome.recorded_at);

        let child = repo
            .load_session("child-session")
            .expect("load child session")
            .expect("child session row");
        assert_eq!(child.state, SessionState::Completed);

        let events = repo
            .list_recent_events("child-session", 10)
            .expect("list child events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_kind, "delegate_completed");

        let outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome")
            .expect("terminal outcome row");
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload_json["final_output"], "done");
    }

    #[test]
    fn finalize_session_terminal_replaces_previous_outcome_payload() {
        let config = isolated_memory_config("finalize-session-terminal-upsert");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        repo.finalize_session_terminal(
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Failed,
                last_error: Some("first".to_owned()),
                event_kind: "delegate_failed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "error": "first"
                }),
                outcome_status: "error".to_owned(),
                outcome_payload_json: json!({
                    "error": "first"
                }),
            },
        )
        .expect("finalize first terminal state");

        let finalized = repo
            .finalize_session_terminal(
                "child-session",
                FinalizeSessionTerminalRequest {
                    state: SessionState::TimedOut,
                    last_error: Some("delegate_timeout".to_owned()),
                    event_kind: "delegate_timed_out".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({
                        "error": "delegate_timeout"
                    }),
                    outcome_status: "timeout".to_owned(),
                    outcome_payload_json: json!({
                        "error": "delegate_timeout"
                    }),
                },
            )
            .expect("finalize second terminal state");

        assert_eq!(finalized.session.state, SessionState::TimedOut);
        assert_eq!(
            finalized.session.last_error.as_deref(),
            Some("delegate_timeout")
        );
        assert_eq!(finalized.terminal_outcome.status, "timeout");
        assert_eq!(
            finalized.terminal_outcome.payload_json["error"],
            "delegate_timeout"
        );

        let outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome")
            .expect("terminal outcome row");
        assert_eq!(outcome.status, "timeout");
        assert_eq!(outcome.payload_json["error"], "delegate_timeout");
    }

    #[test]
    fn finalize_session_terminal_if_current_writes_state_event_and_outcome_when_state_matches() {
        let config = isolated_memory_config("finalize-session-terminal-if-current");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");

        let finalized = repo
            .finalize_session_terminal_if_current(
                "child-session",
                SessionState::Ready,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some("delegate_timeout".to_owned()),
                    event_kind: "delegate_recovery_applied".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({
                        "kind": "queued_async_overdue_marked_failed",
                        "reference": "queued"
                    }),
                    outcome_status: "error".to_owned(),
                    outcome_payload_json: json!({
                        "error": "delegate_timeout"
                    }),
                },
            )
            .expect("conditionally finalize session")
            .expect("conditional finalize result");

        assert_eq!(finalized.session.state, SessionState::Failed);
        assert_eq!(
            finalized.session.last_error.as_deref(),
            Some("delegate_timeout")
        );
        assert_eq!(finalized.event.event_kind, "delegate_recovery_applied");
        assert_eq!(finalized.terminal_outcome.status, "error");

        let child = repo
            .load_session("child-session")
            .expect("load child session")
            .expect("child session row");
        assert_eq!(child.state, SessionState::Failed);
        assert_eq!(child.last_error.as_deref(), Some("delegate_timeout"));

        let events = repo
            .list_recent_events("child-session", 10)
            .expect("list child events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_kind, "delegate_recovery_applied");

        let outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome")
            .expect("terminal outcome row");
        assert_eq!(outcome.status, "error");
        assert_eq!(outcome.payload_json["error"], "delegate_timeout");
    }

    #[test]
    fn finalize_session_terminal_if_current_writes_nothing_when_state_does_not_match() {
        let config = isolated_memory_config("finalize-session-terminal-if-current-noop");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        let finalized = repo
            .finalize_session_terminal_if_current(
                "child-session",
                SessionState::Ready,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some("delegate_timeout".to_owned()),
                    event_kind: "delegate_recovery_applied".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({
                        "kind": "queued_async_overdue_marked_failed",
                        "reference": "queued"
                    }),
                    outcome_status: "error".to_owned(),
                    outcome_payload_json: json!({
                        "error": "delegate_timeout"
                    }),
                },
            )
            .expect("conditionally finalize session");

        assert!(finalized.is_none());

        let child = repo
            .load_session("child-session")
            .expect("load child session")
            .expect("child session row");
        assert_eq!(child.state, SessionState::Running);
        assert!(child.last_error.is_none());

        let events = repo
            .list_recent_events("child-session", 10)
            .expect("list child events");
        assert!(events.is_empty());

        let outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome");
        assert!(outcome.is_none());
    }

    #[test]
    fn load_session_observation_drains_tail_after_cursor_through_terminal_event() {
        let config = isolated_memory_config("session-observation-tail-drain");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        for index in 0..60 {
            repo.append_event(NewSessionEvent {
                session_id: "child-session".to_owned(),
                event_kind: format!("delegate_progress_{index}"),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({
                    "step": index
                }),
            })
            .expect("append progress event");
        }
        repo.finalize_session_terminal(
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "turn_count": 1
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "child-session",
                    "final_output": "done"
                }),
            },
        )
        .expect("finalize child");

        let observation = repo
            .load_session_observation("child-session", 5, Some(0), 50)
            .expect("load session observation")
            .expect("session observation");

        assert_eq!(observation.session.state, SessionState::Completed);
        assert_eq!(
            observation
                .terminal_outcome
                .as_ref()
                .expect("terminal outcome")
                .status,
            "ok"
        );
        assert_eq!(observation.tail_events.len(), 61);
        assert_eq!(
            observation
                .tail_events
                .first()
                .expect("first tail event")
                .id,
            1
        );
        assert_eq!(
            observation
                .tail_events
                .last()
                .expect("last tail event")
                .event_kind,
            "delegate_completed"
        );
        assert_eq!(observation.recent_events.len(), 5);
        assert_eq!(
            observation
                .recent_events
                .last()
                .expect("last recent event")
                .event_kind,
            "delegate_completed"
        );
    }

    #[test]
    fn approval_request_repository_persists_pending_request() {
        let config = isolated_memory_config("approval-request-create");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let created = repo
            .ensure_approval_request(NewApprovalRequestRecord {
                approval_request_id: "apr_123".to_owned(),
                session_id: "root-session".to_owned(),
                turn_id: "turn-123".to_owned(),
                tool_call_id: "call-123".to_owned(),
                tool_name: "delegate_async".to_owned(),
                approval_key: "tool:delegate_async".to_owned(),
                request_payload_json: json!({
                    "tool_name": "delegate_async",
                    "payload": {
                        "task": "inspect child issue"
                    }
                }),
                governance_snapshot_json: json!({
                    "reason": "governed_tool_requires_approval",
                    "rule_id": "medium_balanced_delegate_async"
                }),
            })
            .expect("persist approval request");

        assert_eq!(created.approval_request_id, "apr_123");
        assert_eq!(created.session_id, "root-session");
        assert_eq!(created.tool_name, "delegate_async");
        assert_eq!(created.approval_key, "tool:delegate_async");
        assert_eq!(created.status, ApprovalRequestStatus::Pending);
        assert_eq!(created.decision, None);
        assert_eq!(
            created.request_payload_json["payload"]["task"],
            "inspect child issue"
        );
        assert_eq!(
            created.governance_snapshot_json["rule_id"],
            "medium_balanced_delegate_async"
        );
        assert!(created.resolved_at.is_none());
        assert!(created.executed_at.is_none());
        assert!(created.last_error.is_none());

        let loaded = repo
            .load_approval_request("apr_123")
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(loaded, created);
    }

    #[test]
    fn approval_request_repository_duplicate_create_returns_existing_row() {
        let config = isolated_memory_config("approval-request-idempotent");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let first = repo
            .ensure_approval_request(NewApprovalRequestRecord {
                approval_request_id: "apr_duplicate".to_owned(),
                session_id: "root-session".to_owned(),
                turn_id: "turn-1".to_owned(),
                tool_call_id: "call-1".to_owned(),
                tool_name: "delegate".to_owned(),
                approval_key: "tool:delegate".to_owned(),
                request_payload_json: json!({
                    "tool_name": "delegate",
                    "payload": {
                        "task": "original"
                    }
                }),
                governance_snapshot_json: json!({
                    "reason": "first_reason",
                    "rule_id": "first_rule"
                }),
            })
            .expect("persist first approval request");
        let second = repo
            .ensure_approval_request(NewApprovalRequestRecord {
                approval_request_id: "apr_duplicate".to_owned(),
                session_id: "root-session".to_owned(),
                turn_id: "turn-2".to_owned(),
                tool_call_id: "call-2".to_owned(),
                tool_name: "delegate_async".to_owned(),
                approval_key: "tool:delegate_async".to_owned(),
                request_payload_json: json!({
                    "tool_name": "delegate_async",
                    "payload": {
                        "task": "should_be_ignored"
                    }
                }),
                governance_snapshot_json: json!({
                    "reason": "second_reason",
                    "rule_id": "second_rule"
                }),
            })
            .expect("persist second approval request");

        assert_eq!(second.approval_request_id, first.approval_request_id);
        assert_eq!(second.turn_id, first.turn_id);
        assert_eq!(second.tool_call_id, first.tool_call_id);
        assert_eq!(second.tool_name, first.tool_name);
        assert_eq!(second.approval_key, first.approval_key);
        assert_eq!(second.request_payload_json, first.request_payload_json);
        assert_eq!(
            second.governance_snapshot_json,
            first.governance_snapshot_json
        );
    }

    #[test]
    fn approval_request_repository_transitions_status_if_current() {
        let config = isolated_memory_config("approval-request-transition");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-transition".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate"
            }),
            governance_snapshot_json: json!({
                "reason": "requires_review",
                "rule_id": "delegate_review"
            }),
        })
        .expect("persist approval request");

        let approved = repo
            .transition_approval_request_if_current(
                "apr-transition",
                TransitionApprovalRequestIfCurrentRequest {
                    expected_status: ApprovalRequestStatus::Pending,
                    next_status: ApprovalRequestStatus::Approved,
                    decision: Some(ApprovalDecision::ApproveOnce),
                    resolved_by_session_id: Some("root-session".to_owned()),
                    executed_at: None,
                    last_error: None,
                },
            )
            .expect("transition approval request")
            .expect("transition result");
        assert_eq!(approved.status, ApprovalRequestStatus::Approved);
        assert_eq!(approved.decision, Some(ApprovalDecision::ApproveOnce));
        assert_eq!(
            approved.resolved_by_session_id.as_deref(),
            Some("root-session")
        );
        assert!(approved.resolved_at.is_some());
        assert!(approved.executed_at.is_none());
        assert!(approved.last_error.is_none());

        let noop = repo
            .transition_approval_request_if_current(
                "apr-transition",
                TransitionApprovalRequestIfCurrentRequest {
                    expected_status: ApprovalRequestStatus::Pending,
                    next_status: ApprovalRequestStatus::Denied,
                    decision: Some(ApprovalDecision::Deny),
                    resolved_by_session_id: Some("root-session".to_owned()),
                    executed_at: None,
                    last_error: Some("should not apply".to_owned()),
                },
            )
            .expect("stale transition should not error");
        assert!(noop.is_none());
    }

    #[test]
    fn approval_request_repository_persists_session_scoped_runtime_grant() {
        let config = isolated_memory_config("approval-grant-upsert");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let created = repo
            .upsert_approval_grant(NewApprovalGrantRecord {
                scope_session_id: "root-session".to_owned(),
                approval_key: "tool:delegate_async".to_owned(),
                created_by_session_id: Some("operator-session".to_owned()),
            })
            .expect("upsert approval grant");
        assert_eq!(created.scope_session_id, "root-session");
        assert_eq!(created.approval_key, "tool:delegate_async");
        assert_eq!(
            created.created_by_session_id.as_deref(),
            Some("operator-session")
        );

        let loaded = repo
            .load_approval_grant("root-session", "tool:delegate_async")
            .expect("load approval grant")
            .expect("approval grant row");
        assert_eq!(loaded, created);
    }

    #[test]
    fn approval_request_repository_lists_requests_for_session_and_status() {
        let config = isolated_memory_config("approval-request-list");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child session");

        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-root-pending".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-root-pending".to_owned(),
            tool_call_id: "call-root-pending".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate"
            }),
            governance_snapshot_json: json!({
                "rule_id": "root_pending"
            }),
        })
        .expect("persist root pending request");
        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-root-approved".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-root-approved".to_owned(),
            tool_call_id: "call-root-approved".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate_async"
            }),
            governance_snapshot_json: json!({
                "rule_id": "root_approved"
            }),
        })
        .expect("persist root approved request");
        repo.transition_approval_request_if_current(
            "apr-root-approved",
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Approved,
                decision: Some(ApprovalDecision::ApproveAlways),
                resolved_by_session_id: Some("root-session".to_owned()),
                executed_at: None,
                last_error: None,
            },
        )
        .expect("transition root approved request")
        .expect("approved root request");
        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-child-pending".to_owned(),
            session_id: "child-session".to_owned(),
            turn_id: "turn-child-pending".to_owned(),
            tool_call_id: "call-child-pending".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate"
            }),
            governance_snapshot_json: json!({
                "rule_id": "child_pending"
            }),
        })
        .expect("persist child pending request");

        let all_root_requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list root approval requests");
        assert_eq!(all_root_requests.len(), 2);
        let root_ids = all_root_requests
            .iter()
            .map(|record| record.approval_request_id.as_str())
            .collect::<Vec<_>>();
        assert!(root_ids.contains(&"apr-root-pending"));
        assert!(root_ids.contains(&"apr-root-approved"));

        let pending_root_requests = repo
            .list_approval_requests_for_session(
                "root-session",
                Some(ApprovalRequestStatus::Pending),
            )
            .expect("list pending root approval requests");
        assert_eq!(pending_root_requests.len(), 1);
        assert_eq!(
            pending_root_requests[0].approval_request_id,
            "apr-root-pending"
        );
    }
}
