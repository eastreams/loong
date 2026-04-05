use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{
    Connection, OptionalExtension, Transaction, TransactionBehavior, params, params_from_iter,
};
use serde_json::Value;

use crate::config::ToolConsentMode;
use crate::memory;
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::tools::runtime_config::ToolRuntimeNarrowing;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlanePairingRequestStatus {
    Pending,
    Approved,
    Rejected,
}

impl ControlPlanePairingRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            _ => Err(format!("unknown control-plane pairing status `{value}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlanePairingRequestRecord {
    pub pairing_request_id: String,
    pub device_id: String,
    pub client_id: String,
    pub public_key: String,
    pub role: String,
    pub requested_scopes: BTreeSet<String>,
    pub status: ControlPlanePairingRequestStatus,
    pub requested_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
    pub issued_token_id: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewControlPlanePairingRequestRecord {
    pub pairing_request_id: String,
    pub device_id: String,
    pub client_id: String,
    pub public_key: String,
    pub role: String,
    pub requested_scopes: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionControlPlanePairingRequestIfCurrentRequest {
    pub expected_status: ControlPlanePairingRequestStatus,
    pub next_status: ControlPlanePairingRequestStatus,
    pub issued_token_id: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneDeviceTokenRecord {
    pub token_id: String,
    pub device_id: String,
    pub public_key: String,
    pub role: String,
    pub approved_scopes: BTreeSet<String>,
    pub token_hash: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
    pub last_used_at_ms: Option<i64>,
    pub pairing_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewControlPlaneDeviceTokenRecord {
    pub token_id: String,
    pub device_id: String,
    pub public_key: String,
    pub role: String,
    pub approved_scopes: BTreeSet<String>,
    pub token_hash: String,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
    pub last_used_at_ms: Option<i64>,
    pub pairing_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToolConsentRecord {
    pub scope_session_id: String,
    pub mode: ToolConsentMode,
    pub updated_by_session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionToolConsentRecord {
    pub scope_session_id: String,
    pub mode: ToolConsentMode,
    pub updated_by_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToolPolicyRecord {
    pub session_id: String,
    pub requested_tool_ids: Vec<String>,
    pub runtime_narrowing: ToolRuntimeNarrowing,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSessionToolPolicyRecord {
    pub session_id: String,
    pub requested_tool_ids: Vec<String>,
    pub runtime_narrowing: ToolRuntimeNarrowing,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSearchSourceKind {
    Turn,
    Event,
}

impl SessionSearchSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::Event => "event",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSearchRecord {
    pub session_id: String,
    pub source_kind: SessionSearchSourceKind,
    pub source_id: i64,
    pub role: Option<String>,
    pub event_kind: Option<String>,
    pub content_text: String,
    pub ts: i64,
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

    pub fn count_active_direct_children(&self, parent_session_id: &str) -> Result<usize, String> {
        let parent_session_id = normalize_required_text(parent_session_id, "parent_session_id")?;
        let conn = self.open_connection()?;
        Self::count_active_direct_children_with_conn(&conn, &parent_session_id)
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
        let normalized_query = normalize_required_text(query, "query")?.to_ascii_lowercase();
        let conn = self.open_connection()?;
        Self::search_session_content_with_conn(&conn, &session_id, &normalized_query, limit)
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

    pub fn upsert_session_tool_consent(
        &self,
        record: NewSessionToolConsentRecord,
    ) -> Result<SessionToolConsentRecord, String> {
        let requested_scope_session_id =
            normalize_required_text(&record.scope_session_id, "scope_session_id")?;
        let scope_session_id = self
            .lineage_root_session_id(&requested_scope_session_id)?
            .ok_or_else(|| format!("session `{requested_scope_session_id}` not found"))?;
        let session_exists = self
            .load_session_summary_with_legacy_fallback(&scope_session_id)?
            .is_some();
        if !session_exists {
            return Err(format!("session `{scope_session_id}` not found"));
        }
        let updated_by_session_id = normalize_optional_text(record.updated_by_session_id);
        let ts = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO session_tool_consent(
                scope_session_id,
                mode,
                updated_by_session_id,
                created_at,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(scope_session_id) DO UPDATE SET
                mode = excluded.mode,
                updated_by_session_id = excluded.updated_by_session_id,
                updated_at = excluded.updated_at",
            params![
                scope_session_id,
                record.mode.as_str(),
                updated_by_session_id,
                ts,
                ts
            ],
        )
        .map_err(|error| format!("upsert session tool consent failed: {error}"))?;

        self.load_session_tool_consent(&scope_session_id)?
            .ok_or_else(|| {
                format!("session tool consent `{scope_session_id}` disappeared after upsert")
            })
    }

    pub fn load_session_tool_consent(
        &self,
        scope_session_id: &str,
    ) -> Result<Option<SessionToolConsentRecord>, String> {
        let requested_scope_session_id =
            normalize_required_text(scope_session_id, "scope_session_id")?;
        let scope_session_id = match self.lineage_root_session_id(&requested_scope_session_id)? {
            Some(root_scope_session_id) => root_scope_session_id,
            None => return Ok(None),
        };
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT scope_session_id, mode, updated_by_session_id, created_at, updated_at
                 FROM session_tool_consent
                 WHERE scope_session_id = ?1",
                params![scope_session_id],
                |row| {
                    Ok(RawSessionToolConsentRecord {
                        scope_session_id: row.get(0)?,
                        mode: row.get(1)?,
                        updated_by_session_id: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session tool consent failed: {error}"))?;
        raw.map(SessionToolConsentRecord::try_from_raw).transpose()
    }

    pub fn upsert_session_tool_policy(
        &self,
        record: NewSessionToolPolicyRecord,
    ) -> Result<SessionToolPolicyRecord, String> {
        let session_id = normalize_required_text(&record.session_id, "session_id")?;
        let session_exists = self
            .load_session_summary_with_legacy_fallback(&session_id)?
            .is_some();
        if !session_exists {
            return Err(format!("session `{session_id}` not found"));
        }

        let requested_tool_ids = normalize_tool_id_list(record.requested_tool_ids);
        let encoded_requested_tool_ids = serde_json::to_string(&requested_tool_ids)
            .map_err(|error| format!("encode session tool policy tool ids failed: {error}"))?;
        let encoded_runtime_narrowing = serde_json::to_string(&record.runtime_narrowing)
            .map_err(|error| format!("encode session tool policy narrowing failed: {error}"))?;
        let updated_at = unix_ts_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO session_tool_policies(
                session_id,
                requested_tool_ids_json,
                runtime_narrowing_json,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                requested_tool_ids_json = excluded.requested_tool_ids_json,
                runtime_narrowing_json = excluded.runtime_narrowing_json,
                updated_at = excluded.updated_at",
            params![
                session_id,
                encoded_requested_tool_ids,
                encoded_runtime_narrowing,
                updated_at,
            ],
        )
        .map_err(|error| format!("upsert session tool policy failed: {error}"))?;

        self.load_session_tool_policy(&record.session_id)?
            .ok_or_else(|| {
                format!(
                    "session tool policy `{}` disappeared after upsert",
                    record.session_id
                )
            })
    }

    pub fn load_session_tool_policy(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionToolPolicyRecord>, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    session_id,
                    requested_tool_ids_json,
                    runtime_narrowing_json,
                    updated_at
                 FROM session_tool_policies
                 WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(RawSessionToolPolicyRecord {
                        session_id: row.get(0)?,
                        requested_tool_ids_json: row.get(1)?,
                        runtime_narrowing_json: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load session tool policy failed: {error}"))?;
        raw.map(SessionToolPolicyRecord::try_from_raw).transpose()
    }

    pub fn delete_session_tool_policy(&self, session_id: &str) -> Result<bool, String> {
        let session_id = normalize_required_text(session_id, "session_id")?;
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "DELETE FROM session_tool_policies
                 WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|error| format!("delete session tool policy failed: {error}"))?;
        Ok(affected > 0)
    }

    pub fn ensure_control_plane_pairing_request(
        &self,
        record: NewControlPlanePairingRequestRecord,
    ) -> Result<ControlPlanePairingRequestRecord, String> {
        let pairing_request_id =
            normalize_required_text(&record.pairing_request_id, "pairing_request_id")?;
        let device_id = normalize_required_text(&record.device_id, "device_id")?;
        let client_id = normalize_required_text(&record.client_id, "client_id")?;
        let public_key = normalize_required_text(&record.public_key, "public_key")?;
        let role = normalize_required_text(&record.role, "role")?;
        let requested_scopes_json = encode_string_set_json(&record.requested_scopes)?;
        let requested_at_ms = unix_time_ms_now();
        let conn = self.open_connection()?;
        match conn.execute(
            "INSERT INTO control_plane_pairing_requests(
                pairing_request_id,
                device_id,
                client_id,
                public_key,
                role,
                requested_scopes_json,
                status,
                requested_at_ms,
                resolved_at_ms,
                issued_token_id,
                last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL)",
            params![
                &pairing_request_id,
                device_id,
                client_id,
                public_key,
                role,
                requested_scopes_json,
                ControlPlanePairingRequestStatus::Pending.as_str(),
                requested_at_ms,
            ],
        ) {
            Ok(_) => {}
            Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
                return self
                    .load_control_plane_pairing_request(&pairing_request_id)?
                    .ok_or_else(|| {
                        format!(
                            "control-plane pairing request `{pairing_request_id}` missing after concurrent insert"
                        )
                    });
            }
            Err(error) => {
                return Err(format!(
                    "insert control-plane pairing request row failed: {error}"
                ));
            }
        }

        self.load_control_plane_pairing_request(&pairing_request_id)?
            .ok_or_else(|| {
                format!(
                    "control-plane pairing request `{pairing_request_id}` disappeared after insert"
                )
            })
    }

    pub fn load_control_plane_pairing_request(
        &self,
        pairing_request_id: &str,
    ) -> Result<Option<ControlPlanePairingRequestRecord>, String> {
        let pairing_request_id = normalize_required_text(pairing_request_id, "pairing_request_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    pairing_request_id,
                    device_id,
                    client_id,
                    public_key,
                    role,
                    requested_scopes_json,
                    status,
                    requested_at_ms,
                    resolved_at_ms,
                    issued_token_id,
                    last_error
                 FROM control_plane_pairing_requests
                 WHERE pairing_request_id = ?1",
                params![pairing_request_id],
                |row| {
                    Ok(RawControlPlanePairingRequestRecord {
                        pairing_request_id: row.get(0)?,
                        device_id: row.get(1)?,
                        client_id: row.get(2)?,
                        public_key: row.get(3)?,
                        role: row.get(4)?,
                        requested_scopes_json: row.get(5)?,
                        status: row.get(6)?,
                        requested_at_ms: row.get(7)?,
                        resolved_at_ms: row.get(8)?,
                        issued_token_id: row.get(9)?,
                        last_error: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load control-plane pairing request row failed: {error}"))?;
        raw.map(ControlPlanePairingRequestRecord::try_from_raw)
            .transpose()
    }

    pub fn list_control_plane_pairing_requests(
        &self,
        status: Option<ControlPlanePairingRequestStatus>,
    ) -> Result<Vec<ControlPlanePairingRequestRecord>, String> {
        let conn = self.open_connection()?;
        let mut requests = Vec::new();
        match status {
            Some(status) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            pairing_request_id,
                            device_id,
                            client_id,
                            public_key,
                            role,
                            requested_scopes_json,
                            status,
                            requested_at_ms,
                            resolved_at_ms,
                            issued_token_id,
                            last_error
                         FROM control_plane_pairing_requests
                         WHERE status = ?1
                         ORDER BY requested_at_ms DESC, pairing_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare control-plane pairing request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map(params![status.as_str()], |row| {
                        Ok(RawControlPlanePairingRequestRecord {
                            pairing_request_id: row.get(0)?,
                            device_id: row.get(1)?,
                            client_id: row.get(2)?,
                            public_key: row.get(3)?,
                            role: row.get(4)?,
                            requested_scopes_json: row.get(5)?,
                            status: row.get(6)?,
                            requested_at_ms: row.get(7)?,
                            resolved_at_ms: row.get(8)?,
                            issued_token_id: row.get(9)?,
                            last_error: row.get(10)?,
                        })
                    })
                    .map_err(|error| {
                        format!("query control-plane pairing request list failed: {error}")
                    })?;
                for row in rows {
                    let raw = row.map_err(|error| {
                        format!("decode control-plane pairing request row failed: {error}")
                    })?;
                    let request = ControlPlanePairingRequestRecord::try_from_raw(raw)?;
                    requests.push(request);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            pairing_request_id,
                            device_id,
                            client_id,
                            public_key,
                            role,
                            requested_scopes_json,
                            status,
                            requested_at_ms,
                            resolved_at_ms,
                            issued_token_id,
                            last_error
                         FROM control_plane_pairing_requests
                         ORDER BY requested_at_ms DESC, pairing_request_id ASC",
                    )
                    .map_err(|error| {
                        format!("prepare control-plane pairing request list query failed: {error}")
                    })?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(RawControlPlanePairingRequestRecord {
                            pairing_request_id: row.get(0)?,
                            device_id: row.get(1)?,
                            client_id: row.get(2)?,
                            public_key: row.get(3)?,
                            role: row.get(4)?,
                            requested_scopes_json: row.get(5)?,
                            status: row.get(6)?,
                            requested_at_ms: row.get(7)?,
                            resolved_at_ms: row.get(8)?,
                            issued_token_id: row.get(9)?,
                            last_error: row.get(10)?,
                        })
                    })
                    .map_err(|error| {
                        format!("query control-plane pairing request list failed: {error}")
                    })?;
                for row in rows {
                    let raw = row.map_err(|error| {
                        format!("decode control-plane pairing request row failed: {error}")
                    })?;
                    let request = ControlPlanePairingRequestRecord::try_from_raw(raw)?;
                    requests.push(request);
                }
            }
        }
        Ok(requests)
    }

    pub fn transition_control_plane_pairing_request_if_current(
        &self,
        pairing_request_id: &str,
        request: TransitionControlPlanePairingRequestIfCurrentRequest,
    ) -> Result<Option<ControlPlanePairingRequestRecord>, String> {
        let pairing_request_id = normalize_required_text(pairing_request_id, "pairing_request_id")?;
        let last_error = normalize_optional_text(request.last_error);
        let resolution_ts = matches!(
            request.next_status,
            ControlPlanePairingRequestStatus::Approved | ControlPlanePairingRequestStatus::Rejected
        )
        .then(unix_time_ms_now);
        let conn = self.open_connection()?;
        let affected = conn
            .execute(
                "UPDATE control_plane_pairing_requests
                 SET status = ?3,
                     resolved_at_ms = CASE WHEN ?4 IS NULL THEN resolved_at_ms ELSE ?4 END,
                     issued_token_id = CASE WHEN ?5 IS NULL THEN issued_token_id ELSE ?5 END,
                     last_error = ?6
                 WHERE pairing_request_id = ?1 AND status = ?2",
                params![
                    &pairing_request_id,
                    request.expected_status.as_str(),
                    request.next_status.as_str(),
                    resolution_ts,
                    request.issued_token_id,
                    last_error,
                ],
            )
            .map_err(|error| {
                format!("conditionally update control-plane pairing request failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }

        self.load_control_plane_pairing_request(&pairing_request_id)?
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "control-plane pairing request `{pairing_request_id}` missing after conditional update"
                )
            })
    }

    pub fn approve_control_plane_pairing_request(
        &self,
        request: &ControlPlanePairingRequestRecord,
        token: NewControlPlaneDeviceTokenRecord,
    ) -> Result<Option<ControlPlanePairingRequestRecord>, String> {
        if request.status != ControlPlanePairingRequestStatus::Approved {
            return Err(
                "control-plane pairing approval persistence requires approved status".to_owned(),
            );
        }

        let pairing_request_id =
            normalize_required_text(&request.pairing_request_id, "pairing_request_id")?;
        let resolved_at_ms = request.resolved_at_ms.ok_or_else(|| {
            "approved control-plane pairing request requires resolved_at_ms".to_owned()
        })?;
        let issued_token_id = request.issued_token_id.clone().ok_or_else(|| {
            "approved control-plane pairing request requires issued_token_id".to_owned()
        })?;
        let token_id = normalize_required_text(&token.token_id, "token_id")?;
        let device_id = normalize_required_text(&token.device_id, "device_id")?;
        let public_key = normalize_required_text(&token.public_key, "public_key")?;
        let role = normalize_required_text(&token.role, "role")?;
        let token_hash = normalize_required_text(&token.token_hash, "token_hash")?;
        let approved_scopes_json = encode_string_set_json(&token.approved_scopes)?;
        let last_used_at_ms = token.last_used_at_ms;
        let expires_at_ms = token.expires_at_ms;
        let revoked_at_ms = token.revoked_at_ms;
        let pairing_request_binding = token.pairing_request_id;
        let mut conn = self.open_connection()?;
        let tx = conn.transaction().map_err(|error| {
            format!("open control-plane pairing approval transaction failed: {error}")
        })?;
        let affected = tx
            .execute(
                "UPDATE control_plane_pairing_requests
                 SET status = ?3,
                     resolved_at_ms = ?4,
                     issued_token_id = ?5,
                     last_error = NULL
                 WHERE pairing_request_id = ?1 AND status = ?2",
                params![
                    &pairing_request_id,
                    ControlPlanePairingRequestStatus::Pending.as_str(),
                    ControlPlanePairingRequestStatus::Approved.as_str(),
                    resolved_at_ms,
                    issued_token_id,
                ],
            )
            .map_err(|error| {
                format!("approve control-plane pairing request transaction update failed: {error}")
            })?;
        if affected == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO control_plane_device_tokens(
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                issued_at_ms,
                expires_at_ms,
                revoked_at_ms,
                last_used_at_ms,
                pairing_request_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(device_id) DO UPDATE SET
                token_id = excluded.token_id,
                public_key = excluded.public_key,
                role = excluded.role,
                approved_scopes_json = excluded.approved_scopes_json,
                token_hash = excluded.token_hash,
                issued_at_ms = excluded.issued_at_ms,
                expires_at_ms = excluded.expires_at_ms,
                revoked_at_ms = excluded.revoked_at_ms,
                last_used_at_ms = excluded.last_used_at_ms,
                pairing_request_id = excluded.pairing_request_id",
            params![
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                resolved_at_ms,
                expires_at_ms,
                revoked_at_ms,
                last_used_at_ms,
                pairing_request_binding,
            ],
        )
        .map_err(|error| {
            format!("approve control-plane pairing request token upsert failed: {error}")
        })?;
        tx.commit().map_err(|error| {
            format!("commit control-plane pairing approval transaction failed: {error}")
        })?;

        self.load_control_plane_pairing_request(&pairing_request_id)?
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "control-plane pairing request `{pairing_request_id}` missing after approval commit"
                )
            })
    }

    pub fn upsert_control_plane_device_token(
        &self,
        record: NewControlPlaneDeviceTokenRecord,
    ) -> Result<ControlPlaneDeviceTokenRecord, String> {
        let token_id = normalize_required_text(&record.token_id, "token_id")?;
        let device_id = normalize_required_text(&record.device_id, "device_id")?;
        let public_key = normalize_required_text(&record.public_key, "public_key")?;
        let role = normalize_required_text(&record.role, "role")?;
        let token_hash = normalize_required_text(&record.token_hash, "token_hash")?;
        let approved_scopes_json = encode_string_set_json(&record.approved_scopes)?;
        let issued_at_ms = unix_time_ms_now();
        let conn = self.open_connection()?;
        conn.execute(
            "INSERT INTO control_plane_device_tokens(
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                issued_at_ms,
                expires_at_ms,
                revoked_at_ms,
                last_used_at_ms,
                pairing_request_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(device_id) DO UPDATE SET
                token_id = excluded.token_id,
                public_key = excluded.public_key,
                role = excluded.role,
                approved_scopes_json = excluded.approved_scopes_json,
                token_hash = excluded.token_hash,
                issued_at_ms = excluded.issued_at_ms,
                expires_at_ms = excluded.expires_at_ms,
                revoked_at_ms = excluded.revoked_at_ms,
                last_used_at_ms = excluded.last_used_at_ms,
                pairing_request_id = excluded.pairing_request_id",
            params![
                token_id,
                device_id,
                public_key,
                role,
                approved_scopes_json,
                token_hash,
                issued_at_ms,
                record.expires_at_ms,
                record.revoked_at_ms,
                record.last_used_at_ms,
                record.pairing_request_id,
            ],
        )
        .map_err(|error| format!("upsert control-plane device token failed: {error}"))?;

        self.load_control_plane_device_token_by_device_id(&device_id)?
            .ok_or_else(|| {
                format!("control-plane device token for `{device_id}` disappeared after upsert")
            })
    }

    pub fn load_control_plane_device_token_by_device_id(
        &self,
        device_id: &str,
    ) -> Result<Option<ControlPlaneDeviceTokenRecord>, String> {
        let device_id = normalize_required_text(device_id, "device_id")?;
        let conn = self.open_connection()?;
        let raw = conn
            .query_row(
                "SELECT
                    token_id,
                    device_id,
                    public_key,
                    role,
                    approved_scopes_json,
                    token_hash,
                    issued_at_ms,
                    expires_at_ms,
                    revoked_at_ms,
                    last_used_at_ms,
                    pairing_request_id
                 FROM control_plane_device_tokens
                 WHERE device_id = ?1",
                params![device_id],
                |row| {
                    Ok(RawControlPlaneDeviceTokenRecord {
                        token_id: row.get(0)?,
                        device_id: row.get(1)?,
                        public_key: row.get(2)?,
                        role: row.get(3)?,
                        approved_scopes_json: row.get(4)?,
                        token_hash: row.get(5)?,
                        issued_at_ms: row.get(6)?,
                        expires_at_ms: row.get(7)?,
                        revoked_at_ms: row.get(8)?,
                        last_used_at_ms: row.get(9)?,
                        pairing_request_id: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load control-plane device token failed: {error}"))?;
        raw.map(ControlPlaneDeviceTokenRecord::try_from_raw)
            .transpose()
    }

    pub fn list_control_plane_device_tokens(
        &self,
    ) -> Result<Vec<ControlPlaneDeviceTokenRecord>, String> {
        let conn = self.open_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT
                    token_id,
                    device_id,
                    public_key,
                    role,
                    approved_scopes_json,
                    token_hash,
                    issued_at_ms,
                    expires_at_ms,
                    revoked_at_ms,
                    last_used_at_ms,
                    pairing_request_id
                 FROM control_plane_device_tokens
                 ORDER BY issued_at_ms DESC, token_id ASC",
            )
            .map_err(|error| {
                format!("prepare control-plane device token list query failed: {error}")
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RawControlPlaneDeviceTokenRecord {
                    token_id: row.get(0)?,
                    device_id: row.get(1)?,
                    public_key: row.get(2)?,
                    role: row.get(3)?,
                    approved_scopes_json: row.get(4)?,
                    token_hash: row.get(5)?,
                    issued_at_ms: row.get(6)?,
                    expires_at_ms: row.get(7)?,
                    revoked_at_ms: row.get(8)?,
                    last_used_at_ms: row.get(9)?,
                    pairing_request_id: row.get(10)?,
                })
            })
            .map_err(|error| format!("query control-plane device token list failed: {error}"))?;
        let mut tokens = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| {
                format!("decode control-plane device token row failed: {error}")
            })?;
            let token = ControlPlaneDeviceTokenRecord::try_from_raw(raw)?;
            tokens.push(token);
        }
        Ok(tokens)
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

    fn create_session_with_event_in_tx(
        tx: &Transaction<'_>,
        request: CreateSessionWithEventRequest,
    ) -> Result<SessionEventRecord, String> {
        let session_id = normalize_required_text(&request.session.session_id, "session_id")?;
        let parent_session_id = normalize_optional_text(request.session.parent_session_id);
        let label = normalize_optional_text(request.session.label);
        let event_kind = normalize_required_text(&request.event_kind, "event_kind")?;
        let actor_session_id = normalize_optional_text(request.actor_session_id);
        let event_payload_json = request.event_payload_json;
        let encoded_event_payload = serde_json::to_string(&event_payload_json)
            .map_err(|error| format!("encode session event payload failed: {error}"))?;
        let ts = unix_ts_now();

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

        Ok(SessionEventRecord {
            id: tx.last_insert_rowid(),
            session_id,
            event_kind,
            actor_session_id,
            payload_json: event_payload_json,
            ts,
        })
    }

    fn count_active_direct_children_with_conn(
        conn: &Connection,
        parent_session_id: &str,
    ) -> Result<usize, String> {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM sessions
                 WHERE parent_session_id = ?1
                   AND state IN ('ready', 'running')",
                params![parent_session_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("count active direct child sessions failed: {error}"))?;
        usize::try_from(count)
            .map_err(|error| format!("active direct child count overflowed usize: {error}"))
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

    fn search_session_content_with_conn(
        conn: &Connection,
        session_id: &str,
        normalized_query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchRecord>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let patterns = build_search_like_patterns(normalized_query);
        if patterns.is_empty() {
            return Ok(Vec::new());
        }

        let mut hits = Vec::new();
        hits.extend(Self::search_session_turns_with_conn(
            conn,
            session_id,
            patterns.as_slice(),
            limit,
        )?);
        hits.extend(Self::search_session_events_with_conn(
            conn,
            session_id,
            patterns.as_slice(),
            limit,
        )?);
        Ok(hits)
    }

    fn search_session_turns_with_conn(
        conn: &Connection,
        session_id: &str,
        patterns: &[String],
        limit: usize,
    ) -> Result<Vec<SessionSearchRecord>, String> {
        let where_clause = build_search_where_clause("lower(content)", patterns.len(), 2);
        let sql = format!(
            "SELECT id, session_id, role, content, ts
             FROM turns
             WHERE session_id = ?1
               AND ({where_clause})
             ORDER BY id DESC
             LIMIT {limit}"
        );

        let mut stmt = conn
            .prepare(sql.as_str())
            .map_err(|error| format!("prepare session search turns query failed: {error}"))?;
        let mut bindings = Vec::with_capacity(patterns.len().saturating_add(1));
        bindings.push(session_id.to_owned());
        bindings.extend(patterns.iter().cloned());

        let rows = stmt
            .query_map(params_from_iter(bindings.iter()), |row| {
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
        patterns: &[String],
        limit: usize,
    ) -> Result<Vec<SessionSearchRecord>, String> {
        let where_clause = build_search_where_clause(
            "lower(event_kind || ' ' || payload_json)",
            patterns.len(),
            2,
        );
        let sql = format!(
            "SELECT id, session_id, event_kind, payload_json, ts
             FROM session_events
             WHERE session_id = ?1
               AND ({where_clause})
             ORDER BY id DESC
             LIMIT {limit}"
        );

        let mut stmt = conn
            .prepare(sql.as_str())
            .map_err(|error| format!("prepare session search events query failed: {error}"))?;
        let mut bindings = Vec::with_capacity(patterns.len().saturating_add(1));
        bindings.push(session_id.to_owned());
        bindings.extend(patterns.iter().cloned());

        let rows = stmt
            .query_map(params_from_iter(bindings.iter()), |row| {
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
struct RawSessionSearchTurnRecord {
    id: i64,
    session_id: String,
    role: String,
    content: String,
    ts: i64,
}

#[derive(Debug)]
struct RawSessionSearchEventRecord {
    id: i64,
    session_id: String,
    event_kind: String,
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

#[derive(Debug)]
struct RawControlPlanePairingRequestRecord {
    pairing_request_id: String,
    device_id: String,
    client_id: String,
    public_key: String,
    role: String,
    requested_scopes_json: String,
    status: String,
    requested_at_ms: i64,
    resolved_at_ms: Option<i64>,
    issued_token_id: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug)]
struct RawControlPlaneDeviceTokenRecord {
    token_id: String,
    device_id: String,
    public_key: String,
    role: String,
    approved_scopes_json: String,
    token_hash: String,
    issued_at_ms: i64,
    expires_at_ms: Option<i64>,
    revoked_at_ms: Option<i64>,
    last_used_at_ms: Option<i64>,
    pairing_request_id: Option<String>,
}

#[derive(Debug)]
struct RawSessionToolConsentRecord {
    scope_session_id: String,
    mode: String,
    updated_by_session_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug)]
struct RawSessionToolPolicyRecord {
    session_id: String,
    requested_tool_ids_json: String,
    runtime_narrowing_json: String,
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

impl SessionToolConsentRecord {
    fn try_from_raw(raw: RawSessionToolConsentRecord) -> Result<Self, String> {
        let mode = match raw.mode.as_str() {
            "prompt" => ToolConsentMode::Prompt,
            "auto" => ToolConsentMode::Auto,
            "full" => ToolConsentMode::Full,
            value => return Err(format!("unknown session tool consent mode `{value}`")),
        };
        Ok(Self {
            scope_session_id: raw.scope_session_id,
            mode,
            updated_by_session_id: raw.updated_by_session_id,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        })
    }
}

impl SessionToolPolicyRecord {
    fn try_from_raw(raw: RawSessionToolPolicyRecord) -> Result<Self, String> {
        let requested_tool_ids: Vec<String> = serde_json::from_str(&raw.requested_tool_ids_json)
            .map_err(|error| format!("decode session tool policy tool ids failed: {error}"))?;
        let runtime_narrowing: ToolRuntimeNarrowing =
            serde_json::from_str(&raw.runtime_narrowing_json)
                .map_err(|error| format!("decode session tool policy narrowing failed: {error}"))?;
        Ok(Self {
            session_id: raw.session_id,
            requested_tool_ids: normalize_tool_id_list(requested_tool_ids),
            runtime_narrowing,
            updated_at: raw.updated_at,
        })
    }
}

impl ControlPlanePairingRequestRecord {
    fn try_from_raw(raw: RawControlPlanePairingRequestRecord) -> Result<Self, String> {
        let requested_scopes = decode_string_set_json(&raw.requested_scopes_json)?;
        Ok(Self {
            pairing_request_id: raw.pairing_request_id,
            device_id: raw.device_id,
            client_id: raw.client_id,
            public_key: raw.public_key,
            role: raw.role,
            requested_scopes,
            status: ControlPlanePairingRequestStatus::from_db(&raw.status)?,
            requested_at_ms: raw.requested_at_ms,
            resolved_at_ms: raw.resolved_at_ms,
            issued_token_id: raw.issued_token_id,
            last_error: raw.last_error,
        })
    }
}

impl ControlPlaneDeviceTokenRecord {
    fn try_from_raw(raw: RawControlPlaneDeviceTokenRecord) -> Result<Self, String> {
        let approved_scopes = decode_string_set_json(&raw.approved_scopes_json)?;
        Ok(Self {
            token_id: raw.token_id,
            device_id: raw.device_id,
            public_key: raw.public_key,
            role: raw.role,
            approved_scopes,
            token_hash: raw.token_hash,
            issued_at_ms: raw.issued_at_ms,
            expires_at_ms: raw.expires_at_ms,
            revoked_at_ms: raw.revoked_at_ms,
            last_used_at_ms: raw.last_used_at_ms,
            pairing_request_id: raw.pairing_request_id,
        })
    }
}

fn encode_string_set_json(values: &BTreeSet<String>) -> Result<String, String> {
    let normalized = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    serde_json::to_string(&normalized)
        .map_err(|error| format!("encode control-plane scope set failed: {error}"))
}

fn decode_string_set_json(encoded: &str) -> Result<BTreeSet<String>, String> {
    let decoded = serde_json::from_str::<BTreeSet<String>>(encoded)
        .map_err(|error| format!("decode control-plane scope set failed: {error}"))?;
    Ok(decoded
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>())
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

fn normalize_tool_id_list(tool_ids: Vec<String>) -> Vec<String> {
    let mut normalized = BTreeSet::new();
    for tool_id in tool_ids {
        let trimmed = tool_id.trim();
        if trimmed.is_empty() {
            continue;
        }
        normalized.insert(trimmed.to_owned());
    }
    normalized.into_iter().collect()
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

fn is_resumable_root_session_summary(summary: &SessionSummaryRecord) -> bool {
    if summary.kind != SessionKind::Root {
        return false;
    }
    if summary.archived_at.is_some() {
        return false;
    }
    if summary.turn_count == 0 {
        return false;
    }
    true
}

fn sort_session_summaries(sessions: &mut [SessionSummaryRecord]) {
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
}

fn build_search_where_clause(
    expression: &str,
    pattern_count: usize,
    first_placeholder_index: usize,
) -> String {
    let mut clauses = Vec::with_capacity(pattern_count);
    for offset in 0..pattern_count {
        let placeholder = first_placeholder_index.saturating_add(offset);
        clauses.push(format!("{expression} LIKE ?{placeholder} ESCAPE '\\'"));
    }
    clauses.join(" OR ")
}

fn build_search_like_patterns(normalized_query: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    patterns.push(like_pattern(normalized_query));

    for token in tokenize_search_query(normalized_query) {
        let pattern = like_pattern(token.as_str());
        if patterns.iter().any(|existing| existing == &pattern) {
            continue;
        }
        patterns.push(pattern);
    }

    patterns
}

fn tokenize_search_query(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn like_pattern(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len().saturating_add(4));
    escaped.push('%');
    for ch in raw.chars() {
        match ch {
            '%' | '_' | '\\' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped.push('%');
    escaped
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use serde_json::json;

    use crate::memory::append_turn_direct;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::tools::runtime_config::ToolRuntimeNarrowing;

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

    fn create_root_session(repo: &SessionRepository, session_id: &str) {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
    }

    fn create_delegate_child_session(
        repo: &SessionRepository,
        session_id: &str,
        parent_session_id: &str,
    ) {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some(parent_session_id.to_owned()),
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        })
        .expect("create delegate child session");
    }

    fn append_session_turn(
        config: &MemoryRuntimeConfig,
        session_id: &str,
        role: &str,
        content: &str,
    ) {
        append_turn_direct(session_id, role, content, config).expect("append session turn");
    }

    fn set_session_updated_at(repo: &SessionRepository, session_id: &str, updated_at: i64) {
        let conn = repo.open_connection().expect("open connection");
        conn.execute(
            "UPDATE sessions
             SET updated_at = ?2
             WHERE session_id = ?1",
            params![session_id, updated_at],
        )
        .expect("set session updated_at");
    }

    fn set_turn_timestamps(repo: &SessionRepository, session_id: &str, ts: i64) {
        let conn = repo.open_connection().expect("open connection");
        conn.execute(
            "UPDATE turns
             SET ts = ?2
             WHERE session_id = ?1",
            params![session_id, ts],
        )
        .expect("set turn timestamps");
    }

    fn archive_session(repo: &SessionRepository, session_id: &str, archived_at: i64) {
        let conn = repo.open_connection().expect("open connection");
        conn.execute(
            "INSERT INTO session_events(
                session_id,
                event_kind,
                actor_session_id,
                payload_json,
                ts
             ) VALUES (?1, ?2, NULL, ?3, ?4)",
            params![session_id, "session_archived", "{}", archived_at],
        )
        .expect("insert archive event");
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
    fn create_delegate_child_session_with_event_if_within_limit_serializes_capacity() {
        let config = isolated_memory_config("delegate-child-limit-serialized");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let config = Arc::new(config);
        let handles = ["child-session-a", "child-session-b"]
            .into_iter()
            .map(|child_session_id| {
                let config = Arc::clone(&config);
                thread::spawn(move || {
                    let repo = SessionRepository::new(&config).expect("repository");
                    repo.create_delegate_child_session_with_event_if_within_limit(
                        "root-session",
                        1,
                        |active_children| {
                            thread::park_timeout(Duration::from_millis(100));
                            Ok((
                                CreateSessionWithEventRequest {
                                    session: NewSessionRecord {
                                        session_id: child_session_id.to_owned(),
                                        kind: SessionKind::DelegateChild,
                                        parent_session_id: Some("root-session".to_owned()),
                                        label: Some(child_session_id.to_owned()),
                                        state: SessionState::Ready,
                                    },
                                    event_kind: "delegate_queued".to_owned(),
                                    actor_session_id: Some("root-session".to_owned()),
                                    event_payload_json: json!({
                                        "task": child_session_id,
                                        "active_children": active_children
                                    }),
                                },
                                active_children,
                            ))
                        },
                    )
                })
            })
            .collect::<Vec<_>>();

        let mut active_children_values = Vec::new();
        let mut limit_errors = Vec::new();
        for handle in handles {
            match handle.join().expect("thread join") {
                Ok((created, active_children)) => {
                    active_children_values.push(active_children);
                    assert_eq!(
                        created.session.parent_session_id.as_deref(),
                        Some("root-session")
                    );
                }
                Err(error) => limit_errors.push(error),
            }
        }

        assert_eq!(
            active_children_values,
            vec![0],
            "only one child should be admitted before capacity is exhausted"
        );
        assert_eq!(
            limit_errors.len(),
            1,
            "one concurrent admission should be rejected"
        );
        assert!(
            limit_errors[0].contains("delegate_active_children_exceeded"),
            "unexpected error: {}",
            limit_errors[0]
        );
        assert_eq!(
            repo.count_active_direct_children("root-session")
                .expect("count active direct children after concurrent admissions"),
            1
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
    fn latest_resumable_root_session_prefers_newest_eligible_root() {
        let config = isolated_memory_config("latest-resumable-root");
        let repo = SessionRepository::new(&config).expect("repository");

        create_root_session(&repo, "root-old");
        append_session_turn(&config, "root-old", "user", "old");
        set_session_updated_at(&repo, "root-old", 100);
        set_turn_timestamps(&repo, "root-old", 100);

        create_root_session(&repo, "root-new");
        append_session_turn(&config, "root-new", "user", "new");
        set_session_updated_at(&repo, "root-new", 200);
        set_turn_timestamps(&repo, "root-new", 200);

        create_delegate_child_session(&repo, "delegate-child", "root-new");
        append_session_turn(&config, "delegate-child", "assistant", "child");
        set_session_updated_at(&repo, "delegate-child", 400);
        set_turn_timestamps(&repo, "delegate-child", 400);

        create_root_session(&repo, "root-archived");
        append_session_turn(&config, "root-archived", "assistant", "archived");
        set_session_updated_at(&repo, "root-archived", 500);
        set_turn_timestamps(&repo, "root-archived", 500);
        archive_session(&repo, "root-archived", 600);

        create_root_session(&repo, "root-empty");
        set_session_updated_at(&repo, "root-empty", 700);

        let latest = repo
            .latest_resumable_root_session_summary()
            .expect("load latest resumable root session")
            .expect("eligible root session");

        assert_eq!(latest.session_id, "root-new");
        assert_eq!(latest.kind, SessionKind::Root);
        assert_eq!(latest.archived_at, None);
        assert_eq!(latest.turn_count, 1);
    }

    #[test]
    fn latest_resumable_root_session_includes_legacy_root_when_newest() {
        let config = isolated_memory_config("latest-legacy-root");
        let repo = SessionRepository::new(&config).expect("repository");

        create_root_session(&repo, "root-session");
        append_session_turn(&config, "root-session", "user", "root");
        set_session_updated_at(&repo, "root-session", 100);
        set_turn_timestamps(&repo, "root-session", 100);

        append_session_turn(&config, "telegram:latest", "assistant", "legacy");
        set_turn_timestamps(&repo, "telegram:latest", 200);

        let latest = repo
            .latest_resumable_root_session_summary()
            .expect("load latest resumable root session")
            .expect("latest session");

        assert_eq!(latest.session_id, "telegram:latest");
        assert_eq!(latest.kind, SessionKind::Root);
        assert_eq!(latest.turn_count, 1);
        assert_eq!(latest.last_turn_at, Some(200));
        assert!(
            repo.load_session("telegram:latest")
                .expect("load legacy session")
                .is_none()
        );
    }

    #[test]
    fn latest_resumable_root_session_returns_none_when_no_root_is_resumable() {
        let config = isolated_memory_config("latest-no-resumable-root");
        let repo = SessionRepository::new(&config).expect("repository");

        create_root_session(&repo, "root-empty");
        set_session_updated_at(&repo, "root-empty", 300);

        create_root_session(&repo, "root-archived");
        append_session_turn(&config, "root-archived", "assistant", "archived");
        set_session_updated_at(&repo, "root-archived", 400);
        set_turn_timestamps(&repo, "root-archived", 400);
        archive_session(&repo, "root-archived", 500);

        create_delegate_child_session(&repo, "delegate-child", "root-archived");
        append_session_turn(&config, "delegate-child", "assistant", "delegate");
        set_session_updated_at(&repo, "delegate-child", 600);
        set_turn_timestamps(&repo, "delegate-child", 600);

        append_session_turn(
            &config,
            "delegate:legacy-child",
            "assistant",
            "legacy delegate",
        );
        set_turn_timestamps(&repo, "delegate:legacy-child", 700);

        let latest = repo
            .latest_resumable_root_session_summary()
            .expect("load latest resumable root session");

        assert!(latest.is_none());
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
    fn session_tool_consent_repository_round_trips_root_mode() {
        let config = isolated_memory_config("session-tool-consent");
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
            .upsert_session_tool_consent(NewSessionToolConsentRecord {
                scope_session_id: "root-session".to_owned(),
                mode: ToolConsentMode::Full,
                updated_by_session_id: Some("root-session".to_owned()),
            })
            .expect("upsert session tool consent");
        assert_eq!(created.scope_session_id, "root-session");
        assert_eq!(created.mode, ToolConsentMode::Full);
        assert_eq!(
            created.updated_by_session_id.as_deref(),
            Some("root-session")
        );

        let loaded = repo
            .load_session_tool_consent("root-session")
            .expect("load session tool consent")
            .expect("session tool consent row");
        assert_eq!(loaded, created);
    }

    #[test]
    fn session_tool_consent_repository_normalizes_delegate_scope_to_root() {
        let config = isolated_memory_config("session-tool-consent-delegate-root");
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

        let created = repo
            .upsert_session_tool_consent(NewSessionToolConsentRecord {
                scope_session_id: "child-session".to_owned(),
                mode: ToolConsentMode::Auto,
                updated_by_session_id: Some("child-session".to_owned()),
            })
            .expect("upsert session tool consent");

        assert_eq!(created.scope_session_id, "root-session");
        assert_eq!(created.mode, ToolConsentMode::Auto);

        let loaded = repo
            .load_session_tool_consent("root-session")
            .expect("load session tool consent")
            .expect("session tool consent row");
        assert_eq!(loaded, created);

        let loaded_via_child = repo
            .load_session_tool_consent("child-session")
            .expect("load child session tool consent")
            .expect("child session tool consent row");
        assert_eq!(loaded_via_child, created);
    }

    #[test]
    fn session_tool_policy_repository_round_trips_and_deletes_policy() {
        let config = isolated_memory_config("session-tool-policy");
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
            .upsert_session_tool_policy(NewSessionToolPolicyRecord {
                session_id: "root-session".to_owned(),
                requested_tool_ids: vec![
                    "tool.search".to_owned(),
                    "file.read".to_owned(),
                    "tool.search".to_owned(),
                ],
                runtime_narrowing: ToolRuntimeNarrowing {
                    browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
                        max_sessions: Some(1),
                        ..crate::tools::runtime_config::BrowserRuntimeNarrowing::default()
                    },
                    web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
                        allow_private_hosts: Some(false),
                        enforce_allowed_domains: false,
                        allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                        blocked_domains: BTreeSet::from(["deny.example.com".to_owned()]),
                        timeout_seconds: Some(5),
                        max_bytes: Some(4_096),
                        max_redirects: Some(2),
                    },
                },
            })
            .expect("upsert session tool policy");

        assert_eq!(created.session_id, "root-session");
        assert_eq!(
            created.requested_tool_ids,
            vec!["file.read".to_owned(), "tool.search".to_owned()]
        );
        assert_eq!(created.runtime_narrowing.browser.max_sessions, Some(1));
        assert_eq!(
            created.runtime_narrowing.web_fetch.allowed_domains,
            BTreeSet::from(["docs.example.com".to_owned()])
        );

        let loaded = repo
            .load_session_tool_policy("root-session")
            .expect("load session tool policy")
            .expect("session tool policy");
        assert_eq!(loaded, created);

        let deleted = repo
            .delete_session_tool_policy("root-session")
            .expect("delete session tool policy");
        assert!(deleted);
        assert!(
            repo.load_session_tool_policy("root-session")
                .expect("load session tool policy after delete")
                .is_none()
        );
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
