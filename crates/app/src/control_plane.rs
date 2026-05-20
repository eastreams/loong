use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, broadcast};
use tokio::time::{Duration, Instant, timeout};

#[cfg(feature = "memory-sqlite")]
use crate::acp::{
    AcpSessionMetadata, AcpSessionStatus, AcpSessionStore, AcpSqliteSessionStore,
    acquire_shared_acp_session_manager,
};
#[cfg(feature = "memory-sqlite")]
use crate::config::LoongConfig;
#[cfg(feature = "memory-sqlite")]
use crate::config::ToolConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalRequestRecord, ApprovalRequestStatus, ControlPlaneDeviceTokenRecord,
    ControlPlanePairingRequestRecord as PersistedControlPlanePairingRequestRecord,
    ControlPlanePairingRequestStatus as PersistedControlPlanePairingRequestStatus,
    NewControlPlaneDeviceTokenRecord, NewControlPlanePairingRequestRecord, SessionEventRecord,
    SessionObservationRecord, SessionRepository, SessionSummaryRecord,
    SessionTerminalOutcomeRecord, TransitionControlPlanePairingRequestIfCurrentRequest,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::store::{self, SessionStoreConfig};
#[cfg(feature = "memory-sqlite")]
use crate::tools::session::{
    SessionRuntimeSelfContinuityRecord, SessionWorkflowBindingRecord, SessionWorkflowRecord,
    build_session_tool_policy_status_payload, load_session_workflow_record,
    session_delegate_lifecycle_at,
};

const DEFAULT_RECENT_EVENT_LIMIT: usize = 256;
const CONTROL_PLANE_CONNECTION_TTL_MS: u64 = 15 * 60 * 1000;
const CONTROL_PLANE_CHALLENGE_TTL_MS: u64 = 60 * 1000;
const CONTROL_PLANE_MAX_WAIT_TIMEOUT_MS: u64 = 30_000;
const CONTROL_PLANE_EVENT_CHANNEL_CAPACITY: usize = 256;
const CONTROL_PLANE_TURN_EVENT_CHANNEL_CAPACITY: usize = 256;
const CONTROL_PLANE_TURN_RECENT_EVENT_LIMIT: usize = 256;
const CONTROL_PLANE_TURN_TERMINAL_RETENTION_LIMIT: usize = 256;
#[cfg(feature = "memory-sqlite")]
const DEFAULT_CONTROL_PLANE_SESSION_ID: &str = "default";
#[cfg(feature = "memory-sqlite")]
const CONTROL_PLANE_MAX_LIST_LIMIT: usize = 256;
#[cfg(feature = "memory-sqlite")]
const CONTROL_PLANE_MAX_RECENT_EVENT_LIMIT: usize = 100;
#[cfg(feature = "memory-sqlite")]
const CONTROL_PLANE_MAX_TAIL_EVENT_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneStateLane {
    Presence,
    Health,
    Sessions,
    Approvals,
    Acp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneEventKind {
    PresenceChanged,
    HealthChanged,
    SessionChanged,
    SessionMessage,
    ApprovalRequested,
    ApprovalResolved,
    PairingRequested,
    PairingResolved,
    AcpSessionChanged,
    AcpTurnEvent,
}

impl ControlPlaneEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PresenceChanged => "presence.changed",
            Self::HealthChanged => "health.changed",
            Self::SessionChanged => "session.changed",
            Self::SessionMessage => "session.message",
            Self::ApprovalRequested => "approval.requested",
            Self::ApprovalResolved => "approval.resolved",
            Self::PairingRequested => "pairing.requested",
            Self::PairingResolved => "pairing.resolved",
            Self::AcpSessionChanged => "acp.session.changed",
            Self::AcpTurnEvent => "acp.turn.event",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControlPlaneStateVersion {
    pub presence: u64,
    pub health: u64,
    pub sessions: u64,
    pub approvals: u64,
    pub acp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneSnapshotSummary {
    pub state_version: ControlPlaneStateVersion,
    pub presence_count: usize,
    pub session_count: usize,
    pub pending_approval_count: usize,
    pub acp_session_count: usize,
    pub runtime_ready: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneEventRecord {
    pub kind: ControlPlaneEventKind,
    pub event_name: &'static str,
    pub seq: u64,
    pub state_version: ControlPlaneStateVersion,
    pub payload: Value,
    pub targeted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneTurnStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ControlPlaneTurnStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneTurnEventRecord {
    pub turn_id: String,
    pub session_id: String,
    pub seq: u64,
    pub terminal: bool,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneTurnSnapshot {
    pub turn_id: String,
    pub session_id: String,
    pub status: ControlPlaneTurnStatus,
    pub submitted_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub event_count: usize,
    pub output_text: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ControlPlaneTurnStateRecord {
    snapshot: ControlPlaneTurnSnapshot,
    recent_events: VecDeque<ControlPlaneTurnEventRecord>,
    next_seq: u64,
}

#[derive(Debug)]
pub struct ControlPlaneTurnRegistry {
    nonce: AtomicU64,
    turns: RwLock<BTreeMap<String, ControlPlaneTurnStateRecord>>,
    sender: broadcast::Sender<ControlPlaneTurnEventRecord>,
}

impl Default for ControlPlaneTurnRegistry {
    fn default() -> Self {
        let channel = broadcast::channel(CONTROL_PLANE_TURN_EVENT_CHANNEL_CAPACITY);
        let sender = channel.0;
        Self {
            nonce: AtomicU64::new(0),
            turns: RwLock::new(BTreeMap::new()),
            sender,
        }
    }
}

impl ControlPlaneTurnRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue_turn(&self, session_id: &str) -> ControlPlaneTurnSnapshot {
        let issued_at_ms = current_time_ms();
        let sequence = self.nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let random_component = rand::random::<u64>();
        let turn_id = format!("cpt-turn-{sequence:016x}-{random_component:016x}");
        let snapshot = ControlPlaneTurnSnapshot {
            turn_id: turn_id.clone(),
            session_id: session_id.to_owned(),
            status: ControlPlaneTurnStatus::Running,
            submitted_at_ms: issued_at_ms,
            completed_at_ms: None,
            event_count: 0,
            output_text: None,
            stop_reason: None,
            usage: None,
            error: None,
        };
        let record = ControlPlaneTurnStateRecord {
            snapshot: snapshot.clone(),
            recent_events: VecDeque::new(),
            next_seq: 1,
        };
        let mut turns = self
            .turns
            .write()
            .unwrap_or_else(|error| error.into_inner());
        turns.insert(turn_id, record);
        snapshot
    }

    pub fn read_turn(&self, turn_id: &str) -> Result<Option<ControlPlaneTurnSnapshot>, String> {
        let turns = self.turns.read().unwrap_or_else(|error| error.into_inner());
        let snapshot = turns.get(turn_id).map(|record| record.snapshot.clone());
        Ok(snapshot)
    }

    pub fn recent_events_after(
        &self,
        turn_id: &str,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<ControlPlaneTurnEventRecord>, String> {
        let bounded_limit = limit.clamp(1, CONTROL_PLANE_TURN_RECENT_EVENT_LIMIT);
        let turns = self.turns.read().unwrap_or_else(|error| error.into_inner());
        let Some(record) = turns.get(turn_id) else {
            return Err(format!("control_plane_turn_not_found: `{turn_id}`"));
        };
        let events = record
            .recent_events
            .iter()
            .filter(|event| event.seq > after_seq)
            .take(bounded_limit)
            .cloned()
            .collect::<Vec<_>>();
        Ok(events)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ControlPlaneTurnEventRecord> {
        self.sender.subscribe()
    }

    pub fn record_runtime_event(
        &self,
        turn_id: &str,
        payload: Value,
    ) -> Result<ControlPlaneTurnEventRecord, String> {
        self.push_event(turn_id, false, payload)
    }

    pub fn complete_success(
        &self,
        turn_id: &str,
        output_text: &str,
        stop_reason: Option<&str>,
        usage: Option<Value>,
    ) -> Result<ControlPlaneTurnEventRecord, String> {
        let completed_at_ms = current_time_ms();
        let terminal_status = match stop_reason {
            Some("cancelled") => ControlPlaneTurnStatus::Cancelled,
            _ => ControlPlaneTurnStatus::Completed,
        };
        let usage_payload = usage.clone();
        let payload = json!({
            "event_type": "turn.completed",
            "output_text": output_text,
            "stop_reason": stop_reason,
            "usage": usage_payload,
        });
        let event = {
            let mut turns = self
                .turns
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let Some(record) = turns.get_mut(turn_id) else {
                return Err(format!("control_plane_turn_not_found: `{turn_id}`"));
            };
            Self::ensure_turn_mutable(record, turn_id)?;
            record.snapshot.status = terminal_status;
            record.snapshot.completed_at_ms = Some(completed_at_ms);
            record.snapshot.output_text = Some(output_text.to_owned());
            record.snapshot.stop_reason = stop_reason.map(ToOwned::to_owned);
            record.snapshot.usage = usage;
            record.snapshot.error = None;
            let event = Self::push_event_locked(record, true, payload);
            Self::prune_terminal_turns_locked(&mut turns);
            event
        };
        let send_result = self.sender.send(event.clone());
        let _ = send_result;
        Ok(event)
    }

    pub fn complete_failure(
        &self,
        turn_id: &str,
        error: &str,
    ) -> Result<ControlPlaneTurnEventRecord, String> {
        let completed_at_ms = current_time_ms();
        let payload = json!({
            "event_type": "turn.failed",
            "error": error,
        });
        let event = {
            let mut turns = self
                .turns
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let Some(record) = turns.get_mut(turn_id) else {
                return Err(format!("control_plane_turn_not_found: `{turn_id}`"));
            };
            Self::ensure_turn_mutable(record, turn_id)?;
            record.snapshot.status = ControlPlaneTurnStatus::Failed;
            record.snapshot.completed_at_ms = Some(completed_at_ms);
            record.snapshot.error = Some(error.to_owned());
            record.snapshot.output_text = None;
            record.snapshot.stop_reason = None;
            record.snapshot.usage = None;
            let event = Self::push_event_locked(record, true, payload);
            Self::prune_terminal_turns_locked(&mut turns);
            event
        };
        let send_result = self.sender.send(event.clone());
        let _ = send_result;
        Ok(event)
    }

    fn push_event(
        &self,
        turn_id: &str,
        terminal: bool,
        payload: Value,
    ) -> Result<ControlPlaneTurnEventRecord, String> {
        let event = {
            let mut turns = self
                .turns
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let Some(record) = turns.get_mut(turn_id) else {
                return Err(format!("control_plane_turn_not_found: `{turn_id}`"));
            };
            Self::ensure_turn_mutable(record, turn_id)?;
            Self::push_event_locked(record, terminal, payload)
        };
        let send_result = self.sender.send(event.clone());
        let _ = send_result;
        Ok(event)
    }

    fn push_event_locked(
        record: &mut ControlPlaneTurnStateRecord,
        terminal: bool,
        payload: Value,
    ) -> ControlPlaneTurnEventRecord {
        let event = ControlPlaneTurnEventRecord {
            turn_id: record.snapshot.turn_id.clone(),
            session_id: record.snapshot.session_id.clone(),
            seq: record.next_seq,
            terminal,
            payload,
        };
        record.next_seq += 1;
        record.snapshot.event_count += 1;
        record.recent_events.push_back(event.clone());
        while record.recent_events.len() > CONTROL_PLANE_TURN_RECENT_EVENT_LIMIT {
            record.recent_events.pop_front();
        }
        event
    }

    fn prune_terminal_turns_locked(turns: &mut BTreeMap<String, ControlPlaneTurnStateRecord>) {
        let terminal_count = turns
            .values()
            .filter(|record| record.snapshot.status.is_terminal())
            .count();
        if terminal_count <= CONTROL_PLANE_TURN_TERMINAL_RETENTION_LIMIT {
            return;
        }
        let overflow_count = terminal_count - CONTROL_PLANE_TURN_TERMINAL_RETENTION_LIMIT;
        let mut removal_candidates = turns
            .iter()
            .filter(|(_, record)| record.snapshot.status.is_terminal())
            .map(|(turn_id, record)| {
                let completed_at_ms = record
                    .snapshot
                    .completed_at_ms
                    .unwrap_or(record.snapshot.submitted_at_ms);
                let submitted_at_ms = record.snapshot.submitted_at_ms;
                (completed_at_ms, submitted_at_ms, turn_id.clone())
            })
            .collect::<Vec<_>>();
        removal_candidates.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        for (_, _, turn_id) in removal_candidates.into_iter().take(overflow_count) {
            turns.remove(turn_id.as_str());
        }
    }

    fn ensure_turn_mutable(
        record: &ControlPlaneTurnStateRecord,
        turn_id: &str,
    ) -> Result<(), String> {
        if !record.snapshot.status.is_terminal() {
            return Ok(());
        }
        Err(format!("control_plane_turn_already_terminal: `{turn_id}`"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ControlPlaneSnapshotState {
    presence_count: usize,
    session_count: usize,
    pending_approval_count: usize,
    acp_session_count: usize,
}

#[derive(Debug, Clone)]
struct ControlPlaneRetentionState {
    recent_events: VecDeque<ControlPlaneEventRecord>,
}

impl Default for ControlPlaneRetentionState {
    fn default() -> Self {
        Self {
            recent_events: VecDeque::with_capacity(DEFAULT_RECENT_EVENT_LIMIT),
        }
    }
}

pub struct ControlPlaneManager {
    seq: AtomicU64,
    presence_version: AtomicU64,
    health_version: AtomicU64,
    sessions_version: AtomicU64,
    approvals_version: AtomicU64,
    acp_version: AtomicU64,
    runtime_ready: AtomicBool,
    snapshot_state: RwLock<ControlPlaneSnapshotState>,
    retention_state: RwLock<ControlPlaneRetentionState>,
    event_notify: Notify,
    event_sender: broadcast::Sender<ControlPlaneEventRecord>,
}

impl Default for ControlPlaneManager {
    fn default() -> Self {
        let channel = broadcast::channel(CONTROL_PLANE_EVENT_CHANNEL_CAPACITY);
        let event_sender = channel.0;
        let seq = AtomicU64::new(0);
        let presence_version = AtomicU64::new(0);
        let health_version = AtomicU64::new(0);
        let sessions_version = AtomicU64::new(0);
        let approvals_version = AtomicU64::new(0);
        let acp_version = AtomicU64::new(0);
        let runtime_ready = AtomicBool::new(false);
        let snapshot_state = RwLock::new(ControlPlaneSnapshotState::default());
        let retention_state = RwLock::new(ControlPlaneRetentionState::default());
        let event_notify = Notify::new();
        Self {
            seq,
            presence_version,
            health_version,
            sessions_version,
            approvals_version,
            acp_version,
            runtime_ready,
            snapshot_state,
            retention_state,
            event_notify,
            event_sender,
        }
    }
}

impl ControlPlaneManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> ControlPlaneSnapshotSummary {
        let snapshot_state = self.snapshot_state();
        ControlPlaneSnapshotSummary {
            state_version: self.state_version(),
            presence_count: snapshot_state.presence_count,
            session_count: snapshot_state.session_count,
            pending_approval_count: snapshot_state.pending_approval_count,
            acp_session_count: snapshot_state.acp_session_count,
            runtime_ready: self.runtime_ready.load(Ordering::Relaxed),
        }
    }

    pub fn recent_events(
        &self,
        limit: usize,
        include_targeted: bool,
    ) -> Vec<ControlPlaneEventRecord> {
        let retention = self.retention_state();
        let bounded_limit = limit.clamp(1, DEFAULT_RECENT_EVENT_LIMIT);
        let mut events = retention
            .recent_events
            .iter()
            .filter(|event| include_targeted || !event.targeted)
            .cloned()
            .collect::<Vec<_>>();
        let start = events.len().saturating_sub(bounded_limit);
        if start > 0 {
            events.drain(0..start);
        }
        events
    }

    pub fn recent_events_after(
        &self,
        after_seq: u64,
        limit: usize,
        include_targeted: bool,
    ) -> Vec<ControlPlaneEventRecord> {
        let retention = self.retention_state();
        let bounded_limit = limit.clamp(1, DEFAULT_RECENT_EVENT_LIMIT);
        retention
            .recent_events
            .iter()
            .filter(|event| include_targeted || !event.targeted)
            .filter(|event| event.seq > after_seq)
            .take(bounded_limit)
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ControlPlaneEventRecord> {
        self.event_sender.subscribe()
    }

    pub async fn wait_for_recent_events(
        &self,
        after_seq: u64,
        limit: usize,
        include_targeted: bool,
        timeout_ms: u64,
    ) -> Vec<ControlPlaneEventRecord> {
        let clamped_timeout_ms = timeout_ms.clamp(1, CONTROL_PLANE_MAX_WAIT_TIMEOUT_MS);
        let deadline = Instant::now() + Duration::from_millis(clamped_timeout_ms);
        loop {
            let notified = self.event_notify.notified();
            let events = self.recent_events_after(after_seq, limit, include_targeted);
            if !events.is_empty() {
                return events;
            }

            let now = Instant::now();
            if now >= deadline {
                return Vec::new();
            }

            let remaining = deadline.saturating_duration_since(now);
            let wait_result = timeout(remaining, notified).await;
            if wait_result.is_err() {
                return Vec::new();
            }
        }
    }

    pub fn set_runtime_ready(&self, runtime_ready: bool) {
        self.runtime_ready.store(runtime_ready, Ordering::Relaxed);
    }

    pub fn set_presence_count(&self, count: usize) {
        self.with_snapshot_state(|snapshot| {
            snapshot.presence_count = count;
        });
    }

    pub fn set_session_count(&self, count: usize) {
        self.with_snapshot_state(|snapshot| {
            snapshot.session_count = count;
        });
    }

    pub fn set_pending_approval_count(&self, count: usize) {
        self.with_snapshot_state(|snapshot| {
            snapshot.pending_approval_count = count;
        });
    }

    pub fn set_acp_session_count(&self, count: usize) {
        self.with_snapshot_state(|snapshot| {
            snapshot.acp_session_count = count;
        });
    }

    pub fn record_presence_changed(&self, count: usize, payload: Value) -> ControlPlaneEventRecord {
        self.with_snapshot_state(|snapshot| {
            snapshot.presence_count = count;
        });
        self.record_event(
            ControlPlaneStateLane::Presence,
            ControlPlaneEventKind::PresenceChanged,
            payload,
            false,
        )
    }

    pub fn record_health_changed(
        &self,
        runtime_ready: bool,
        payload: Value,
    ) -> ControlPlaneEventRecord {
        self.set_runtime_ready(runtime_ready);
        self.record_event(
            ControlPlaneStateLane::Health,
            ControlPlaneEventKind::HealthChanged,
            payload,
            false,
        )
    }

    pub fn record_sessions_changed(&self, count: usize, payload: Value) -> ControlPlaneEventRecord {
        self.with_snapshot_state(|snapshot| {
            snapshot.session_count = count;
        });
        self.record_event(
            ControlPlaneStateLane::Sessions,
            ControlPlaneEventKind::SessionChanged,
            payload,
            false,
        )
    }

    pub fn record_session_message(
        &self,
        payload: Value,
        targeted: bool,
    ) -> ControlPlaneEventRecord {
        self.record_event(
            ControlPlaneStateLane::Sessions,
            ControlPlaneEventKind::SessionMessage,
            payload,
            targeted,
        )
    }

    pub fn record_approval_requested(
        &self,
        pending_count: usize,
        payload: Value,
    ) -> ControlPlaneEventRecord {
        self.with_snapshot_state(|snapshot| {
            snapshot.pending_approval_count = pending_count;
        });
        self.record_event(
            ControlPlaneStateLane::Approvals,
            ControlPlaneEventKind::ApprovalRequested,
            payload,
            false,
        )
    }

    pub fn record_approval_resolved(
        &self,
        pending_count: usize,
        payload: Value,
        targeted: bool,
    ) -> ControlPlaneEventRecord {
        self.with_snapshot_state(|snapshot| {
            snapshot.pending_approval_count = pending_count;
        });
        self.record_event(
            ControlPlaneStateLane::Approvals,
            ControlPlaneEventKind::ApprovalResolved,
            payload,
            targeted,
        )
    }

    pub fn record_pairing_requested(&self, payload: Value) -> ControlPlaneEventRecord {
        self.record_event(
            ControlPlaneStateLane::Approvals,
            ControlPlaneEventKind::PairingRequested,
            payload,
            false,
        )
    }

    pub fn record_pairing_resolved(
        &self,
        payload: Value,
        targeted: bool,
    ) -> ControlPlaneEventRecord {
        self.record_event(
            ControlPlaneStateLane::Approvals,
            ControlPlaneEventKind::PairingResolved,
            payload,
            targeted,
        )
    }

    pub fn record_acp_session_changed(
        &self,
        count: usize,
        payload: Value,
    ) -> ControlPlaneEventRecord {
        self.with_snapshot_state(|snapshot| {
            snapshot.acp_session_count = count;
        });
        self.record_event(
            ControlPlaneStateLane::Acp,
            ControlPlaneEventKind::AcpSessionChanged,
            payload,
            false,
        )
    }

    pub fn record_acp_turn_event(&self, payload: Value, targeted: bool) -> ControlPlaneEventRecord {
        self.record_event(
            ControlPlaneStateLane::Acp,
            ControlPlaneEventKind::AcpTurnEvent,
            payload,
            targeted,
        )
    }

    pub fn state_version(&self) -> ControlPlaneStateVersion {
        ControlPlaneStateVersion {
            presence: self.presence_version.load(Ordering::Relaxed),
            health: self.health_version.load(Ordering::Relaxed),
            sessions: self.sessions_version.load(Ordering::Relaxed),
            approvals: self.approvals_version.load(Ordering::Relaxed),
            acp: self.acp_version.load(Ordering::Relaxed),
        }
    }

    fn record_event(
        &self,
        lane: ControlPlaneStateLane,
        kind: ControlPlaneEventKind,
        payload: Value,
        targeted: bool,
    ) -> ControlPlaneEventRecord {
        let _ = self.bump_version(lane);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let event = ControlPlaneEventRecord {
            kind,
            event_name: kind.as_str(),
            seq,
            state_version: self.state_version(),
            payload,
            targeted,
        };
        self.push_recent_event(event.clone());
        event
    }

    fn bump_version(&self, lane: ControlPlaneStateLane) -> u64 {
        match lane {
            ControlPlaneStateLane::Presence => {
                self.presence_version.fetch_add(1, Ordering::Relaxed) + 1
            }
            ControlPlaneStateLane::Health => {
                self.health_version.fetch_add(1, Ordering::Relaxed) + 1
            }
            ControlPlaneStateLane::Sessions => {
                self.sessions_version.fetch_add(1, Ordering::Relaxed) + 1
            }
            ControlPlaneStateLane::Approvals => {
                self.approvals_version.fetch_add(1, Ordering::Relaxed) + 1
            }
            ControlPlaneStateLane::Acp => self.acp_version.fetch_add(1, Ordering::Relaxed) + 1,
        }
    }

    fn snapshot_state(&self) -> ControlPlaneSnapshotState {
        self.snapshot_state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn retention_state(&self) -> ControlPlaneRetentionState {
        self.retention_state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn with_snapshot_state(&self, mutate: impl FnOnce(&mut ControlPlaneSnapshotState)) {
        let mut snapshot = self
            .snapshot_state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        mutate(&mut snapshot);
    }

    fn push_recent_event(&self, event: ControlPlaneEventRecord) {
        let mut retention = self
            .retention_state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        retention.recent_events.push_back(event.clone());
        while retention.recent_events.len() > DEFAULT_RECENT_EVENT_LIMIT {
            retention.recent_events.pop_front();
        }
        let send_result = self.event_sender.send(event);
        let _ = send_result;
        self.event_notify.notify_waiters();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneConnectionPrincipal {
    pub connection_id: String,
    pub client_id: String,
    pub role: String,
    pub scopes: BTreeSet<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneConnectionLease {
    pub token: String,
    pub principal: ControlPlaneConnectionPrincipal,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub acknowledged_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlPlaneConnectionRecord {
    principal: ControlPlaneConnectionPrincipal,
    issued_at_ms: u64,
    expires_at_ms: u64,
    acknowledged_seq: Option<u64>,
}

#[derive(Debug, Default)]
pub struct ControlPlaneConnectionRegistry {
    nonce: AtomicU64,
    connections: RwLock<BTreeMap<String, ControlPlaneConnectionRecord>>,
}

impl ControlPlaneConnectionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(&self, principal: ControlPlaneConnectionPrincipal) -> ControlPlaneConnectionLease {
        self.issue_with_ttl_ms(principal, CONTROL_PLANE_CONNECTION_TTL_MS)
    }

    pub fn resolve(&self, token: &str) -> Result<Option<ControlPlaneConnectionLease>, String> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }
        let now_ms = current_time_ms();
        let mut connections = self
            .connections
            .write()
            .unwrap_or_else(|error| error.into_inner());
        connections.retain(|_, record| record.expires_at_ms > now_ms);
        Ok(connections
            .get(token)
            .map(|record| ControlPlaneConnectionLease {
                token: token.to_owned(),
                principal: record.principal.clone(),
                issued_at_ms: record.issued_at_ms,
                expires_at_ms: record.expires_at_ms,
                acknowledged_seq: record.acknowledged_seq,
            }))
    }

    pub fn acknowledge_seq(
        &self,
        token: &str,
        seq: u64,
    ) -> Result<Option<ControlPlaneConnectionLease>, String> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }
        let now_ms = current_time_ms();
        let mut connections = self
            .connections
            .write()
            .unwrap_or_else(|error| error.into_inner());
        connections.retain(|_, record| record.expires_at_ms > now_ms);
        let Some(record) = connections.get_mut(token) else {
            return Ok(None);
        };
        let next_acknowledged_seq = match record.acknowledged_seq {
            Some(existing) => existing.max(seq),
            None => seq,
        };
        record.acknowledged_seq = Some(next_acknowledged_seq);
        Ok(Some(ControlPlaneConnectionLease {
            token: token.to_owned(),
            principal: record.principal.clone(),
            issued_at_ms: record.issued_at_ms,
            expires_at_ms: record.expires_at_ms,
            acknowledged_seq: record.acknowledged_seq,
        }))
    }

    pub fn revoke(&self, token: &str) -> bool {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        self.connections
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(token)
            .is_some()
    }

    pub fn snapshot_leases(&self) -> Vec<ControlPlaneConnectionLease> {
        let now_ms = current_time_ms();
        let mut connections = self
            .connections
            .write()
            .unwrap_or_else(|error| error.into_inner());
        connections.retain(|_, record| record.expires_at_ms > now_ms);
        let mut leases = connections
            .iter()
            .map(|(token, record)| ControlPlaneConnectionLease {
                token: token.clone(),
                principal: record.principal.clone(),
                issued_at_ms: record.issued_at_ms,
                expires_at_ms: record.expires_at_ms,
                acknowledged_seq: record.acknowledged_seq,
            })
            .collect::<Vec<_>>();
        leases.sort_by(|left, right| left.token.cmp(&right.token));
        leases
    }

    pub fn restore_leases(&self, leases: &[ControlPlaneConnectionLease]) -> Result<usize, String> {
        let now_ms = current_time_ms();
        let mut connections = self
            .connections
            .write()
            .unwrap_or_else(|error| error.into_inner());
        connections.clear();
        let mut restored = 0usize;
        for lease in leases {
            if lease.expires_at_ms <= now_ms {
                continue;
            }
            connections.insert(
                lease.token.clone(),
                ControlPlaneConnectionRecord {
                    principal: lease.principal.clone(),
                    issued_at_ms: lease.issued_at_ms,
                    expires_at_ms: lease.expires_at_ms,
                    acknowledged_seq: lease.acknowledged_seq,
                },
            );
            restored = restored.saturating_add(1);
        }
        Ok(restored)
    }

    fn issue_with_ttl_ms(
        &self,
        principal: ControlPlaneConnectionPrincipal,
        ttl_ms: u64,
    ) -> ControlPlaneConnectionLease {
        let issued_at_ms = current_time_ms();
        let expires_at_ms = issued_at_ms.saturating_add(ttl_ms.max(1));
        let sequence = self.nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let random_component = rand::random::<u64>();
        let token = format!("cpt-{sequence:016x}-{random_component:016x}");
        let record = ControlPlaneConnectionRecord {
            principal: principal.clone(),
            issued_at_ms,
            expires_at_ms,
            acknowledged_seq: None,
        };
        self.connections
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(token.clone(), record);
        ControlPlaneConnectionLease {
            token,
            principal,
            issued_at_ms,
            expires_at_ms,
            acknowledged_seq: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneChallenge {
    pub nonce: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Default)]
pub struct ControlPlaneChallengeRegistry {
    nonce: AtomicU64,
    challenges: RwLock<BTreeMap<String, ControlPlaneChallenge>>,
}

impl ControlPlaneChallengeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(&self) -> ControlPlaneChallenge {
        self.issue_with_ttl_ms(CONTROL_PLANE_CHALLENGE_TTL_MS)
    }

    pub fn consume(&self, nonce: &str) -> Result<Option<ControlPlaneChallenge>, String> {
        let nonce = nonce.trim();
        if nonce.is_empty() {
            return Ok(None);
        }
        let now_ms = current_time_ms();
        let mut challenges = self
            .challenges
            .write()
            .unwrap_or_else(|error| error.into_inner());
        challenges.retain(|_, challenge| challenge.expires_at_ms > now_ms);
        Ok(challenges.remove(nonce))
    }

    fn issue_with_ttl_ms(&self, ttl_ms: u64) -> ControlPlaneChallenge {
        let issued_at_ms = current_time_ms();
        let expires_at_ms = issued_at_ms.saturating_add(ttl_ms.max(1));
        let sequence = self.nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let random_component = rand::random::<u64>();
        let challenge = ControlPlaneChallenge {
            nonce: format!("cpc-{sequence:016x}-{random_component:016x}"),
            issued_at_ms,
            expires_at_ms,
        };
        self.challenges
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(challenge.nonce.clone(), challenge.clone());
        challenge
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlanePairingStatus {
    Pending,
    Approved,
    Rejected,
}

impl ControlPlanePairingStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
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
    pub status: ControlPlanePairingStatus,
    pub requested_at_ms: u64,
    pub resolved_at_ms: Option<u64>,
    pub issued_token_id: Option<String>,
    pub device_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneApprovedDeviceSummary {
    pub device_id: String,
    pub public_key: String,
    pub role: String,
    pub approved_scopes: BTreeSet<String>,
    pub issued_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlPlanePairingConnectDecision {
    Authorized,
    PairingRequired {
        request: Box<ControlPlanePairingRequestRecord>,
        created: bool,
    },
    DeviceTokenRequired,
    DeviceTokenInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlPlaneApprovedDeviceRecord {
    device_id: String,
    public_key: String,
    role: String,
    approved_scopes: BTreeSet<String>,
    token_id: String,
    token_hash: String,
    approved_at_ms: u64,
}

pub struct ControlPlanePairingRegistry {
    nonce: AtomicU64,
    requests: RwLock<BTreeMap<String, ControlPlanePairingRequestRecord>>,
    approved_devices: RwLock<BTreeMap<String, ControlPlaneApprovedDeviceRecord>>,
    #[cfg(feature = "memory-sqlite")]
    memory_config: Option<SessionStoreConfig>,
}

impl Default for ControlPlanePairingRegistry {
    fn default() -> Self {
        Self {
            nonce: AtomicU64::new(0),
            requests: RwLock::new(BTreeMap::new()),
            approved_devices: RwLock::new(BTreeMap::new()),
            #[cfg(feature = "memory-sqlite")]
            memory_config: None,
        }
    }
}

impl ControlPlanePairingRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "memory-sqlite")]
    pub fn with_memory_config(memory_config: SessionStoreConfig) -> Result<Self, String> {
        let repo = SessionRepository::new(&memory_config)?;
        let persisted_requests = repo.list_control_plane_pairing_requests(None)?;
        let persisted_devices = repo.list_control_plane_device_tokens()?;

        let requests = persisted_requests
            .into_iter()
            .map(Self::request_from_persisted)
            .map(|request| (request.pairing_request_id.clone(), request))
            .collect::<BTreeMap<_, _>>();
        let approved_devices = persisted_devices
            .into_iter()
            .map(Self::approved_device_from_persisted)
            .map(|device| (device.device_id.clone(), device))
            .collect::<BTreeMap<_, _>>();

        Ok(Self {
            nonce: AtomicU64::new(0),
            requests: RwLock::new(requests),
            approved_devices: RwLock::new(approved_devices),
            memory_config: Some(memory_config),
        })
    }

    pub fn evaluate_connect(
        &self,
        device_id: &str,
        client_id: &str,
        public_key: &str,
        role: &str,
        requested_scopes: &BTreeSet<String>,
        device_token: Option<&str>,
    ) -> Result<ControlPlanePairingConnectDecision, String> {
        let device_id = normalize_required_text(device_id, "device_id")?;
        let client_id = normalize_required_text(client_id, "client_id")?;
        let public_key = normalize_required_text(public_key, "public_key")?;
        let role = normalize_required_text(role, "role")?;
        let requested_scopes = requested_scopes
            .iter()
            .map(|scope| scope.trim())
            .filter(|scope| !scope.is_empty())
            .map(ToOwned::to_owned)
            .collect::<BTreeSet<_>>();
        let device_token = device_token
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(approved) = self
            .approved_devices
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(&device_id)
            .cloned()
            && approved.public_key == public_key
        {
            let requires_repairing =
                approved_device_requires_pairing(&approved, role.as_str(), &requested_scopes);
            if !requires_repairing {
                let hashed_token = device_token.map(hash_control_plane_device_token);
                return match device_token {
                    Some(_)
                        if hashed_token
                            .as_deref()
                            .is_some_and(|token_hash| token_hash == approved.token_hash) =>
                    {
                        Ok(ControlPlanePairingConnectDecision::Authorized)
                    }
                    Some(_) => Ok(ControlPlanePairingConnectDecision::DeviceTokenInvalid),
                    None => Ok(ControlPlanePairingConnectDecision::DeviceTokenRequired),
                };
            }
        }

        let mut requests = self
            .requests
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(existing) = requests
            .values()
            .find(|record| {
                record.status == ControlPlanePairingStatus::Pending
                    && record.device_id == device_id
                    && record.public_key == public_key
                    && record.role == role
                    && record.requested_scopes == requested_scopes
            })
            .cloned()
        {
            return Ok(ControlPlanePairingConnectDecision::PairingRequired {
                request: Box::new(existing),
                created: false,
            });
        }

        let request_id = self.next_pairing_request_id();
        let request = ControlPlanePairingRequestRecord {
            pairing_request_id: request_id.clone(),
            device_id,
            client_id,
            public_key,
            role,
            requested_scopes,
            status: ControlPlanePairingStatus::Pending,
            requested_at_ms: current_time_ms(),
            resolved_at_ms: None,
            issued_token_id: None,
            device_token: None,
        };
        #[cfg(feature = "memory-sqlite")]
        self.persist_request(&request)?;
        requests.insert(request_id, request.clone());
        Ok(ControlPlanePairingConnectDecision::PairingRequired {
            request: Box::new(request),
            created: true,
        })
    }

    pub fn list_requests(
        &self,
        status: Option<ControlPlanePairingStatus>,
        limit: usize,
    ) -> Vec<ControlPlanePairingRequestRecord> {
        let mut requests = self
            .requests
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|record| status.is_none_or(|status| record.status == status))
            .cloned()
            .collect::<Vec<_>>();
        requests.sort_by(|left, right| {
            right
                .requested_at_ms
                .cmp(&left.requested_at_ms)
                .then_with(|| left.pairing_request_id.cmp(&right.pairing_request_id))
        });
        requests.truncate(limit.max(1));
        requests
    }

    pub fn pending_request_count(&self) -> usize {
        self.requests
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|record| record.status == ControlPlanePairingStatus::Pending)
            .count()
    }

    pub fn approved_device_count(&self) -> usize {
        self.approved_devices
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    pub fn list_approved_devices(&self, limit: usize) -> Vec<ControlPlaneApprovedDeviceSummary> {
        let mut approved_devices = self
            .approved_devices
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        approved_devices.sort_by(|left, right| {
            right
                .approved_at_ms
                .cmp(&left.approved_at_ms)
                .then_with(|| left.device_id.cmp(&right.device_id))
        });
        approved_devices.truncate(limit.max(1));

        approved_devices
            .iter()
            .map(approved_device_summary_from_record)
            .collect()
    }

    pub fn last_activity_ms(&self) -> Option<u64> {
        let request_activity = self
            .requests
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .flat_map(|record| [Some(record.requested_at_ms), record.resolved_at_ms])
            .flatten()
            .max();
        let device_activity = self
            .approved_devices
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .map(|device| device.approved_at_ms)
            .max();

        match (request_activity, device_activity) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        }
    }

    pub fn resolve_request(
        &self,
        pairing_request_id: &str,
        approve: bool,
    ) -> Result<Option<ControlPlanePairingRequestRecord>, String> {
        let pairing_request_id = normalize_required_text(pairing_request_id, "pairing_request_id")?;
        let mut requests = self
            .requests
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let Some(record) = requests.get_mut(&pairing_request_id) else {
            return Ok(None);
        };
        if record.status != ControlPlanePairingStatus::Pending {
            return Ok(Some(record.clone()));
        }
        let resolved_at_ms = current_time_ms();
        if approve {
            let token_id = self.next_device_token_id();
            let device_token = self.next_device_token();
            let token_hash = hash_control_plane_device_token(device_token.as_str());
            let approved_device = ControlPlaneApprovedDeviceRecord {
                device_id: record.device_id.clone(),
                public_key: record.public_key.clone(),
                role: record.role.clone(),
                approved_scopes: record.requested_scopes.clone(),
                token_id: token_id.clone(),
                token_hash,
                approved_at_ms: resolved_at_ms,
            };
            let mut updated_record = record.clone();
            updated_record.status = ControlPlanePairingStatus::Approved;
            updated_record.resolved_at_ms = Some(resolved_at_ms);
            updated_record.issued_token_id = Some(token_id.clone());
            updated_record.device_token = Some(device_token);
            #[cfg(feature = "memory-sqlite")]
            self.persist_approved_device(&updated_record, resolved_at_ms, token_id)?;
            self.approved_devices
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .insert(updated_record.device_id.clone(), approved_device);
            *record = updated_record;
        } else {
            let mut updated_record = record.clone();
            updated_record.status = ControlPlanePairingStatus::Rejected;
            updated_record.resolved_at_ms = Some(resolved_at_ms);
            updated_record.issued_token_id = None;
            updated_record.device_token = None;
            #[cfg(feature = "memory-sqlite")]
            self.persist_request(&updated_record)?;
            *record = updated_record;
        }
        Ok(Some(record.clone()))
    }

    fn next_pairing_request_id(&self) -> String {
        let sequence = self.nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let random_component = rand::random::<u64>();
        format!("pair-{sequence:016x}-{random_component:016x}")
    }

    fn next_device_token_id(&self) -> String {
        let sequence = self.nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let random_component = rand::random::<u64>();
        format!("cpdt-{sequence:016x}-{random_component:016x}")
    }

    fn next_device_token(&self) -> String {
        let sequence = self.nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let random_component = rand::random::<u64>();
        format!("cpd-{sequence:016x}-{random_component:016x}")
    }

    #[cfg(feature = "memory-sqlite")]
    fn persist_request(&self, request: &ControlPlanePairingRequestRecord) -> Result<(), String> {
        let Some(memory_config) = self.memory_config.as_ref() else {
            return Ok(());
        };
        let repo = SessionRepository::new(memory_config)?;
        let new_request = NewControlPlanePairingRequestRecord {
            pairing_request_id: request.pairing_request_id.clone(),
            device_id: request.device_id.clone(),
            client_id: request.client_id.clone(),
            public_key: request.public_key.clone(),
            role: request.role.clone(),
            requested_scopes: request.requested_scopes.clone(),
        };
        let _ = repo.ensure_control_plane_pairing_request(new_request)?;
        let next_status = Self::persisted_pairing_status(request.status);
        if next_status != PersistedControlPlanePairingRequestStatus::Pending {
            let transition = TransitionControlPlanePairingRequestIfCurrentRequest {
                expected_status: PersistedControlPlanePairingRequestStatus::Pending,
                next_status,
                issued_token_id: request.issued_token_id.clone(),
                last_error: None,
            };
            let _ = repo.transition_control_plane_pairing_request_if_current(
                &request.pairing_request_id,
                transition,
            )?;
        }
        Ok(())
    }

    #[cfg(feature = "memory-sqlite")]
    fn persist_approved_device(
        &self,
        request: &ControlPlanePairingRequestRecord,
        resolved_at_ms: u64,
        token_id: String,
    ) -> Result<(), String> {
        let Some(memory_config) = self.memory_config.as_ref() else {
            return Ok(());
        };
        let Some(device_token) = request.device_token.as_deref() else {
            return Err("control-plane pairing approval requires device_token".to_owned());
        };
        let repo = SessionRepository::new(memory_config)?;
        let new_token = NewControlPlaneDeviceTokenRecord {
            token_id,
            device_id: request.device_id.clone(),
            public_key: request.public_key.clone(),
            role: request.role.clone(),
            approved_scopes: request.requested_scopes.clone(),
            token_hash: hash_control_plane_device_token(device_token),
            expires_at_ms: None,
            revoked_at_ms: None,
            last_used_at_ms: Some(resolved_at_ms as i64),
            pairing_request_id: Some(request.pairing_request_id.clone()),
        };
        let persisted_request = Self::request_to_persisted(request);
        let persisted =
            repo.approve_control_plane_pairing_request(&persisted_request, new_token)?;
        if persisted.is_none() {
            return Err(format!(
                "control-plane pairing request `{}` changed before approval persistence completed",
                request.pairing_request_id
            ));
        }
        Ok(())
    }

    #[cfg(feature = "memory-sqlite")]
    fn request_from_persisted(
        persisted: PersistedControlPlanePairingRequestRecord,
    ) -> ControlPlanePairingRequestRecord {
        ControlPlanePairingRequestRecord {
            pairing_request_id: persisted.pairing_request_id,
            device_id: persisted.device_id,
            client_id: persisted.client_id,
            public_key: persisted.public_key,
            role: persisted.role,
            requested_scopes: persisted.requested_scopes,
            status: Self::pairing_status_from_persisted(persisted.status),
            requested_at_ms: persisted.requested_at_ms as u64,
            resolved_at_ms: persisted.resolved_at_ms.map(|value| value as u64),
            issued_token_id: persisted.issued_token_id,
            device_token: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn request_to_persisted(
        request: &ControlPlanePairingRequestRecord,
    ) -> PersistedControlPlanePairingRequestRecord {
        let requested_at_ms = request.requested_at_ms.try_into().unwrap_or(i64::MAX);
        let resolved_at_ms = request
            .resolved_at_ms
            .map(|value| value.try_into().unwrap_or(i64::MAX));
        PersistedControlPlanePairingRequestRecord {
            pairing_request_id: request.pairing_request_id.clone(),
            device_id: request.device_id.clone(),
            client_id: request.client_id.clone(),
            public_key: request.public_key.clone(),
            role: request.role.clone(),
            requested_scopes: request.requested_scopes.clone(),
            status: Self::persisted_pairing_status(request.status),
            requested_at_ms,
            resolved_at_ms,
            issued_token_id: request.issued_token_id.clone(),
            last_error: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn approved_device_from_persisted(
        persisted: ControlPlaneDeviceTokenRecord,
    ) -> ControlPlaneApprovedDeviceRecord {
        ControlPlaneApprovedDeviceRecord {
            device_id: persisted.device_id,
            public_key: persisted.public_key,
            role: persisted.role,
            approved_scopes: persisted.approved_scopes,
            token_id: persisted.token_id,
            token_hash: persisted.token_hash,
            approved_at_ms: persisted.issued_at_ms as u64,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn persisted_pairing_status(
        status: ControlPlanePairingStatus,
    ) -> PersistedControlPlanePairingRequestStatus {
        match status {
            ControlPlanePairingStatus::Pending => {
                PersistedControlPlanePairingRequestStatus::Pending
            }
            ControlPlanePairingStatus::Approved => {
                PersistedControlPlanePairingRequestStatus::Approved
            }
            ControlPlanePairingStatus::Rejected => {
                PersistedControlPlanePairingRequestStatus::Rejected
            }
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn pairing_status_from_persisted(
        status: PersistedControlPlanePairingRequestStatus,
    ) -> ControlPlanePairingStatus {
        match status {
            PersistedControlPlanePairingRequestStatus::Pending => {
                ControlPlanePairingStatus::Pending
            }
            PersistedControlPlanePairingRequestStatus::Approved => {
                ControlPlanePairingStatus::Approved
            }
            PersistedControlPlanePairingRequestStatus::Rejected => {
                ControlPlanePairingStatus::Rejected
            }
        }
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{field_name} is required"))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn hash_control_plane_device_token(token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(token.as_bytes());
    let bytes = digest.finalize();
    hex::encode(bytes)
}

fn approved_device_requires_pairing(
    approved: &ControlPlaneApprovedDeviceRecord,
    requested_role: &str,
    requested_scopes: &BTreeSet<String>,
) -> bool {
    let same_role = approved.role == requested_role;
    if !same_role {
        return true;
    }
    let scopes_within_approved = requested_scopes.is_subset(&approved.approved_scopes);
    !scopes_within_approved
}

fn approved_device_summary_from_record(
    approved: &ControlPlaneApprovedDeviceRecord,
) -> ControlPlaneApprovedDeviceSummary {
    ControlPlaneApprovedDeviceSummary {
        device_id: approved.device_id.clone(),
        public_key: approved.public_key.clone(),
        role: approved.role.clone(),
        approved_scopes: approved.approved_scopes.clone(),
        issued_at_ms: approved.approved_at_ms,
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneRepositorySnapshotSummary {
    pub current_session_id: String,
    pub session_count: usize,
    pub pending_approval_count: usize,
    pub acp_session_count: usize,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneSessionListView {
    pub current_session_id: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub sessions: Vec<ControlPlaneSessionSummaryView>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneSessionSummaryView {
    pub session: SessionSummaryRecord,
    pub workflow: ControlPlaneSessionWorkflowView,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneSessionWorkflowContinuityView {
    pub present: bool,
    pub resolved_identity_present: bool,
    pub session_profile_projection_present: bool,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneSessionWorkflowBindingWorktreeView {
    pub worktree_id: String,
    pub workspace_root: String,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneSessionWorkflowBindingView {
    pub session_id: String,
    pub task_id: String,
    pub task_session_id: String,
    pub mode: String,
    pub execution_surface: String,
    pub worktree: Option<ControlPlaneSessionWorkflowBindingWorktreeView>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneSessionWorkflowView {
    pub workflow_id: String,
    pub task: Option<String>,
    pub phase: Option<String>,
    pub operation_kind: Option<String>,
    pub operation_scope: Option<String>,
    pub task_session_id: Option<String>,
    pub lineage_root_session_id: Option<String>,
    pub lineage_depth: Option<usize>,
    pub runtime_self_continuity: Option<ControlPlaneSessionWorkflowContinuityView>,
    pub binding: Option<ControlPlaneSessionWorkflowBindingView>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneSessionObservationView {
    pub session: ControlPlaneSessionSummaryView,
    pub terminal_outcome: Option<SessionTerminalOutcomeRecord>,
    pub recent_events: Vec<SessionEventRecord>,
    pub tail_events: Vec<SessionEventRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneTaskSummaryView {
    pub task_id: String,
    pub task_session_id: String,
    pub owner_session_id: String,
    pub session_id: String,
    pub scope_session_id: String,
    pub label: Option<String>,
    pub session_state: String,
    pub delegate_phase: Option<String>,
    pub delegate_mode: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub workflow: ControlPlaneSessionWorkflowView,
    pub approval_request_count: usize,
    pub approval_attention_count: usize,
    pub requested_tool_ids: Vec<String>,
    pub visible_requested_tool_ids: Vec<String>,
    pub effective_tool_ids: Vec<String>,
    pub visible_effective_tool_ids: Vec<String>,
    pub effective_runtime_narrowing: Value,
    pub last_error: Option<String>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneTaskListView {
    pub current_session_id: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub tasks: Vec<ControlPlaneTaskSummaryView>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
struct BackgroundTaskCandidate {
    task_id: String,
    owner_session_id: String,
    updated_at: i64,
    view: ControlPlaneTaskSummaryView,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub struct ControlPlaneApprovalListView {
    pub current_session_id: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub approvals: Vec<ApprovalRequestRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneAcpSessionListView {
    pub current_session_id: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub sessions: Vec<AcpSessionMetadata>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneAcpSessionReadView {
    pub current_session_id: String,
    pub metadata: AcpSessionMetadata,
    pub status: AcpSessionStatus,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
pub struct ControlPlaneRepositoryView {
    memory_config: SessionStoreConfig,
    tool_config: ToolConfig,
    current_session_id: String,
}

#[cfg(feature = "memory-sqlite")]
impl ControlPlaneRepositoryView {
    pub fn new(
        memory_config: SessionStoreConfig,
        tool_config: ToolConfig,
        current_session_id: impl Into<String>,
    ) -> Self {
        Self {
            memory_config,
            tool_config,
            current_session_id: normalize_control_plane_session_id(&current_session_id.into()),
        }
    }

    pub fn current_session_id(&self) -> &str {
        &self.current_session_id
    }

    pub fn snapshot_summary(&self) -> Result<ControlPlaneRepositorySnapshotSummary, String> {
        let repo = self.open_repo()?;
        let visible_sessions = self.visible_sessions(&repo)?;
        let pending_approval_count = self.count_approvals(
            &repo,
            visible_sessions.as_slice(),
            Some(ApprovalRequestStatus::Pending),
        )?;
        Ok(ControlPlaneRepositorySnapshotSummary {
            current_session_id: self.current_session_id.clone(),
            session_count: visible_sessions.len(),
            pending_approval_count,
            acp_session_count: 0,
        })
    }

    pub fn list_sessions(
        &self,
        include_archived: bool,
        limit: usize,
    ) -> Result<ControlPlaneSessionListView, String> {
        let repo = self.open_repo()?;
        let mut visible_sessions = self.visible_sessions(&repo)?;
        if !include_archived {
            visible_sessions.retain(|session| session.archived_at.is_none());
        }
        let matched_count = visible_sessions.len();
        visible_sessions.truncate(limit.clamp(1, CONTROL_PLANE_MAX_LIST_LIMIT));
        let session_views = self.load_session_summary_views(&repo, visible_sessions.as_slice())?;
        let returned_count = session_views.len();
        Ok(ControlPlaneSessionListView {
            current_session_id: self.current_session_id.clone(),
            matched_count,
            returned_count,
            sessions: session_views,
        })
    }

    pub fn list_background_tasks(
        &self,
        include_archived: bool,
        limit: usize,
    ) -> Result<ControlPlaneTaskListView, String> {
        let repo = self.open_repo()?;
        let mut visible_sessions = self.visible_sessions(&repo)?;
        if !include_archived {
            visible_sessions.retain(|session| session.archived_at.is_none());
        }

        let mut task_candidates =
            self.load_background_task_candidates(&repo, visible_sessions.as_slice())?;
        let matched_count = task_candidates.len();
        let bounded_limit = limit.clamp(1, CONTROL_PLANE_MAX_LIST_LIMIT);
        task_candidates.truncate(bounded_limit);
        let returned_count = task_candidates.len();
        let tasks = task_candidates
            .into_iter()
            .map(|candidate| candidate.view)
            .collect::<Vec<_>>();

        Ok(ControlPlaneTaskListView {
            current_session_id: self.current_session_id.clone(),
            matched_count,
            returned_count,
            tasks,
        })
    }

    pub fn read_session(
        &self,
        target_session_id: &str,
        recent_event_limit: usize,
        tail_after_id: Option<i64>,
        tail_page_limit: usize,
    ) -> Result<Option<ControlPlaneSessionObservationView>, String> {
        let target_session_id = target_session_id.trim();
        if target_session_id.is_empty() {
            return Err("control_plane_session_id_missing".to_owned());
        }
        let repo = self.open_repo()?;
        self.ensure_visible_session(&repo, target_session_id)?;
        let observation = repo.load_session_observation(
            target_session_id,
            recent_event_limit.clamp(1, CONTROL_PLANE_MAX_RECENT_EVENT_LIMIT),
            tail_after_id,
            tail_page_limit.clamp(1, CONTROL_PLANE_MAX_TAIL_EVENT_LIMIT),
        )?;
        self.build_session_observation_view(&repo, observation)
    }

    pub fn read_background_task(
        &self,
        target_task_id: &str,
    ) -> Result<Option<ControlPlaneTaskSummaryView>, String> {
        let trimmed_task_id = target_task_id.trim();
        if trimmed_task_id.is_empty() {
            return Err("control_plane_session_id_missing".to_owned());
        }

        let repo = self.open_repo()?;
        let visible_sessions = self.visible_sessions(&repo)?;
        let task_candidates =
            self.load_background_task_candidates(&repo, visible_sessions.as_slice())?;
        let matching_candidate = task_candidates
            .into_iter()
            .find(|candidate| candidate.task_id == trimmed_task_id);
        if let Some(candidate) = matching_candidate {
            return Ok(Some(candidate.view));
        }

        let session = repo.load_session_summary_with_legacy_fallback(trimmed_task_id)?;
        let Some(session) = session else {
            return Ok(None);
        };
        self.ensure_visible_session(&repo, &session.session_id)?;
        self.build_background_task_view(&repo, &session)
    }

    pub fn ensure_visible_session_id(&self, target_session_id: &str) -> Result<(), String> {
        let target_session_id = target_session_id.trim();
        if target_session_id.is_empty() {
            return Err("control_plane_session_id_missing".to_owned());
        }
        let repo = self.open_repo()?;
        self.ensure_visible_session(&repo, target_session_id)
    }

    pub fn list_approvals(
        &self,
        session_id: Option<&str>,
        status: Option<ApprovalRequestStatus>,
        limit: usize,
    ) -> Result<ControlPlaneApprovalListView, String> {
        let repo = self.open_repo()?;
        let target_session_ids = match session_id {
            Some(session_id) => {
                let session_id = session_id.trim();
                if session_id.is_empty() {
                    return Err("control_plane_session_id_missing".to_owned());
                }
                self.ensure_visible_session(&repo, session_id)?;
                vec![session_id.to_owned()]
            }
            None => self
                .visible_sessions(&repo)?
                .into_iter()
                .map(|session| session.session_id)
                .collect::<Vec<_>>(),
        };

        let mut approvals = Vec::new();
        for session_id in &target_session_ids {
            approvals.extend(repo.list_approval_requests_for_session(session_id, status)?);
        }
        approvals.sort_by(|left, right| {
            right
                .requested_at
                .cmp(&left.requested_at)
                .then_with(|| left.approval_request_id.cmp(&right.approval_request_id))
        });

        let matched_count = approvals.len();
        approvals.truncate(limit.clamp(1, CONTROL_PLANE_MAX_LIST_LIMIT));
        let returned_count = approvals.len();
        Ok(ControlPlaneApprovalListView {
            current_session_id: self.current_session_id.clone(),
            matched_count,
            returned_count,
            approvals,
        })
    }

    fn open_repo(&self) -> Result<SessionRepository, String> {
        SessionRepository::new(&self.memory_config)
    }

    fn visible_sessions(
        &self,
        repo: &SessionRepository,
    ) -> Result<Vec<SessionSummaryRecord>, String> {
        repo.list_visible_sessions(&self.current_session_id)
    }

    fn ensure_visible_session(
        &self,
        repo: &SessionRepository,
        target_session_id: &str,
    ) -> Result<(), String> {
        if self
            .visible_sessions(repo)?
            .iter()
            .any(|session| session.session_id == target_session_id)
        {
            return Ok(());
        }
        Err(format!(
            "visibility_denied: session `{target_session_id}` is not visible from `{}`",
            self.current_session_id
        ))
    }

    fn load_session_summary_views(
        &self,
        repo: &SessionRepository,
        sessions: &[SessionSummaryRecord],
    ) -> Result<Vec<ControlPlaneSessionSummaryView>, String> {
        let mut session_views = Vec::new();
        for session in sessions {
            let session_clone = session.clone();
            let workflow_record = load_session_workflow_record(repo, session, None)?;
            let workflow = control_plane_session_workflow_view(workflow_record);
            let session_view = ControlPlaneSessionSummaryView {
                session: session_clone,
                workflow,
            };
            session_views.push(session_view);
        }
        Ok(session_views)
    }

    fn build_session_observation_view(
        &self,
        repo: &SessionRepository,
        observation: Option<SessionObservationRecord>,
    ) -> Result<Option<ControlPlaneSessionObservationView>, String> {
        let Some(observation) = observation else {
            return Ok(None);
        };

        let workflow_record = load_session_workflow_record(repo, &observation.session, None)?;
        let workflow = control_plane_session_workflow_view(workflow_record);
        let session_view = ControlPlaneSessionSummaryView {
            session: observation.session,
            workflow,
        };
        let observation_view = ControlPlaneSessionObservationView {
            session: session_view,
            terminal_outcome: observation.terminal_outcome,
            recent_events: observation.recent_events,
            tail_events: observation.tail_events,
        };
        Ok(Some(observation_view))
    }

    fn build_background_task_view(
        &self,
        repo: &SessionRepository,
        session: &SessionSummaryRecord,
    ) -> Result<Option<ControlPlaneTaskSummaryView>, String> {
        let task_candidate = self.build_background_task_candidate(repo, session)?;
        Ok(task_candidate.map(|candidate| candidate.view))
    }

    fn load_background_task_candidates(
        &self,
        repo: &SessionRepository,
        sessions: &[SessionSummaryRecord],
    ) -> Result<Vec<BackgroundTaskCandidate>, String> {
        let mut candidates_by_id = BTreeMap::<String, BackgroundTaskCandidate>::new();

        for session in sessions {
            let task_candidate = self.build_background_task_candidate(repo, session)?;
            let Some(task_candidate) = task_candidate else {
                continue;
            };

            let replace_existing = candidates_by_id
                .get(task_candidate.task_id.as_str())
                .map(|existing| background_task_candidate_is_newer(&task_candidate, existing))
                .unwrap_or(true);
            if replace_existing {
                candidates_by_id.insert(task_candidate.task_id.clone(), task_candidate);
            }
        }

        let mut candidates = candidates_by_id.into_values().collect::<Vec<_>>();
        candidates.sort_by(background_task_candidate_cmp_desc);
        Ok(candidates)
    }

    fn build_background_task_candidate(
        &self,
        repo: &SessionRepository,
        session: &SessionSummaryRecord,
    ) -> Result<Option<BackgroundTaskCandidate>, String> {
        if session.kind != crate::session::repository::SessionKind::DelegateChild {
            return Ok(None);
        }

        let delegate_events = repo.list_delegate_lifecycle_events(&session.session_id)?;
        let current_timestamp = current_control_plane_unix_timestamp();
        let delegate_lifecycle =
            session_delegate_lifecycle_at(session, delegate_events.as_slice(), current_timestamp);
        let Some(delegate_lifecycle) = delegate_lifecycle else {
            return Ok(None);
        };

        let is_async_mode = delegate_lifecycle.mode == "async";
        if !is_async_mode {
            return Ok(None);
        }

        let workflow_record =
            load_session_workflow_record(repo, session, Some(delegate_events.as_slice()))?;
        let task_id = control_plane_task_id_for_workflow(&workflow_record, session);
        let updated_at = workflow_record
            .task_progress
            .as_ref()
            .map(|record| record.updated_at)
            .unwrap_or(session.updated_at);
        let workflow = control_plane_session_workflow_view(workflow_record);
        let tool_policy_payload =
            build_session_tool_policy_status_payload(repo, &session.session_id, &self.tool_config)?;
        let requested_tool_ids = control_plane_requested_tool_ids(&tool_policy_payload);
        let visible_requested_tool_ids =
            control_plane_visible_requested_tool_ids(&tool_policy_payload);
        let effective_tool_ids = control_plane_effective_tool_ids(&tool_policy_payload);
        let visible_effective_tool_ids =
            control_plane_visible_effective_tool_ids(&tool_policy_payload);
        let effective_runtime_narrowing =
            control_plane_effective_runtime_narrowing(&tool_policy_payload);
        let approval_requests =
            repo.list_approval_requests_for_session(&session.session_id, None)?;
        let pending_approval_requests = repo.list_approval_requests_for_session(
            &session.session_id,
            Some(ApprovalRequestStatus::Pending),
        )?;
        let delegate_phase = delegate_lifecycle.phase.to_owned();
        let delegate_mode = delegate_lifecycle.mode.to_owned();

        let task_view = ControlPlaneTaskSummaryView {
            task_id: task_id.clone(),
            task_session_id: session.session_id.clone(),
            owner_session_id: session.session_id.clone(),
            session_id: session.session_id.clone(),
            scope_session_id: self.current_session_id.clone(),
            label: session.label.clone(),
            session_state: session.state.as_str().to_owned(),
            delegate_phase: Some(delegate_phase),
            delegate_mode: Some(delegate_mode),
            timeout_seconds: delegate_lifecycle.timeout_seconds,
            workflow,
            approval_request_count: approval_requests.len(),
            approval_attention_count: pending_approval_requests.len(),
            requested_tool_ids,
            visible_requested_tool_ids,
            effective_tool_ids,
            visible_effective_tool_ids,
            effective_runtime_narrowing,
            last_error: session.last_error.clone(),
        };

        let task_candidate = BackgroundTaskCandidate {
            task_id,
            owner_session_id: session.session_id.clone(),
            updated_at,
            view: task_view,
        };
        Ok(Some(task_candidate))
    }

    fn count_approvals(
        &self,
        repo: &SessionRepository,
        sessions: &[SessionSummaryRecord],
        status: Option<ApprovalRequestStatus>,
    ) -> Result<usize, String> {
        let mut count = 0usize;
        for session in sessions {
            count += repo
                .list_approval_requests_for_session(&session.session_id, status)?
                .len();
        }
        Ok(count)
    }
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_session_workflow_continuity_view(
    continuity: SessionRuntimeSelfContinuityRecord,
) -> ControlPlaneSessionWorkflowContinuityView {
    ControlPlaneSessionWorkflowContinuityView {
        present: continuity.present,
        resolved_identity_present: continuity.resolved_identity_present,
        session_profile_projection_present: continuity.session_profile_projection_present,
    }
}

#[cfg(feature = "memory-sqlite")]
fn background_task_candidate_is_newer(
    candidate: &BackgroundTaskCandidate,
    existing: &BackgroundTaskCandidate,
) -> bool {
    background_task_candidate_cmp_desc(candidate, existing).is_lt()
}

#[cfg(feature = "memory-sqlite")]
fn background_task_candidate_cmp_desc(
    left: &BackgroundTaskCandidate,
    right: &BackgroundTaskCandidate,
) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| left.task_id.cmp(&right.task_id))
        .then_with(|| left.owner_session_id.cmp(&right.owner_session_id))
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_task_id_for_workflow(
    workflow: &SessionWorkflowRecord,
    session: &SessionSummaryRecord,
) -> String {
    let task_progress_task_id = workflow
        .task_progress
        .as_ref()
        .map(|task_progress| task_progress.task_id.trim())
        .filter(|task_id| !task_id.is_empty())
        .map(ToOwned::to_owned);
    if let Some(task_id) = task_progress_task_id {
        return task_id;
    }

    let binding_task_id = workflow
        .binding
        .as_ref()
        .map(|binding| binding.task_id.trim())
        .filter(|task_id| !task_id.is_empty())
        .map(ToOwned::to_owned);
    if let Some(task_id) = binding_task_id {
        return task_id;
    }

    session.session_id.clone()
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_session_workflow_view(
    workflow: SessionWorkflowRecord,
) -> ControlPlaneSessionWorkflowView {
    let phase = workflow
        .phase
        .map(|value: loong_contracts::GovernedWorkflowPhase| value.as_str().to_owned());
    let operation_kind = workflow
        .operation_kind
        .map(|value: loong_contracts::WorkflowOperationKind| value.as_str().to_owned());
    let operation_scope = workflow
        .operation_scope
        .map(|value: loong_contracts::WorkflowOperationScope| value.as_str().to_owned());
    let runtime_self_continuity = workflow
        .runtime_self_continuity
        .map(control_plane_session_workflow_continuity_view);
    let binding = workflow
        .binding
        .map(control_plane_session_workflow_binding_view);

    ControlPlaneSessionWorkflowView {
        workflow_id: workflow.workflow_id,
        task: workflow.task,
        phase,
        operation_kind,
        operation_scope,
        task_session_id: workflow.task_session_id,
        lineage_root_session_id: workflow.lineage_root_session_id,
        lineage_depth: workflow.lineage_depth,
        runtime_self_continuity,
        binding,
    }
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_session_workflow_binding_view(
    binding: SessionWorkflowBindingRecord,
) -> ControlPlaneSessionWorkflowBindingView {
    let worktree =
        binding
            .worktree
            .map(|worktree| ControlPlaneSessionWorkflowBindingWorktreeView {
                worktree_id: worktree.worktree_id,
                workspace_root: worktree.workspace_root,
            });

    ControlPlaneSessionWorkflowBindingView {
        session_id: binding.session_id,
        task_id: binding.task_id,
        task_session_id: binding.task_session_id,
        mode: binding.mode.as_str().to_owned(),
        execution_surface: binding.execution_surface,
        worktree,
    }
}

#[cfg(feature = "memory-sqlite")]
fn current_control_plane_unix_timestamp() -> i64 {
    let now = SystemTime::now();
    let duration_since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds_since_epoch = duration_since_epoch.as_secs();
    let bounded_seconds = seconds_since_epoch.min(i64::MAX as u64);
    bounded_seconds as i64
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_requested_tool_ids(tool_policy_payload: &Value) -> Vec<String> {
    control_plane_tool_ids(tool_policy_payload, "requested_tool_ids")
        .into_iter()
        .map(|tool_id| crate::tools::model_visible_tool_name(tool_id.as_str()))
        .collect()
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_visible_requested_tool_ids(tool_policy_payload: &Value) -> Vec<String> {
    let visible_tool_ids =
        control_plane_tool_ids(tool_policy_payload, "visible_requested_tool_ids");
    if !visible_tool_ids.is_empty() {
        return visible_tool_ids;
    }

    let requested_tool_ids = control_plane_requested_tool_ids(tool_policy_payload);
    requested_tool_ids
        .iter()
        .map(|tool_id| crate::tools::model_visible_tool_name(tool_id.as_str()))
        .collect()
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_effective_tool_ids(tool_policy_payload: &Value) -> Vec<String> {
    control_plane_tool_ids(tool_policy_payload, "effective_tool_ids")
        .into_iter()
        .map(|tool_id| crate::tools::model_visible_tool_name(tool_id.as_str()))
        .collect()
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_visible_effective_tool_ids(tool_policy_payload: &Value) -> Vec<String> {
    let visible_tool_ids =
        control_plane_tool_ids(tool_policy_payload, "visible_effective_tool_ids");
    if !visible_tool_ids.is_empty() {
        return visible_tool_ids;
    }

    let effective_tool_ids = control_plane_effective_tool_ids(tool_policy_payload);
    effective_tool_ids
        .iter()
        .map(|tool_id| crate::tools::model_visible_tool_name(tool_id.as_str()))
        .collect()
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_tool_ids(tool_policy_payload: &Value, field_name: &str) -> Vec<String> {
    let tool_id_values = tool_policy_payload
        .get(field_name)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut tool_ids = Vec::new();
    for tool_id_value in tool_id_values {
        let tool_id = tool_id_value.as_str();
        let Some(tool_id) = tool_id else {
            continue;
        };
        let owned_tool_id = tool_id.to_owned();
        tool_ids.push(owned_tool_id);
    }
    tool_ids
}

#[cfg(feature = "memory-sqlite")]
fn control_plane_effective_runtime_narrowing(tool_policy_payload: &Value) -> Value {
    tool_policy_payload
        .get("effective_runtime_narrowing")
        .cloned()
        .unwrap_or(Value::Null)
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
pub struct ControlPlaneAcpView {
    config: LoongConfig,
    current_session_id: String,
}

#[cfg(feature = "memory-sqlite")]
impl ControlPlaneAcpView {
    pub fn new(config: LoongConfig, current_session_id: impl Into<String>) -> Self {
        Self {
            config,
            current_session_id: normalize_control_plane_session_id(&current_session_id.into()),
        }
    }

    pub fn current_session_id(&self) -> &str {
        &self.current_session_id
    }

    pub async fn visible_session_count(&self) -> Result<usize, String> {
        Ok(self.visible_acp_sessions()?.len())
    }

    pub fn list_sessions(&self, limit: usize) -> Result<ControlPlaneAcpSessionListView, String> {
        let mut sessions = self.visible_acp_sessions()?;
        sessions.sort_by(|left, right| {
            right
                .last_activity_ms
                .cmp(&left.last_activity_ms)
                .then_with(|| left.session_key.cmp(&right.session_key))
        });
        let matched_count = sessions.len();
        sessions.truncate(limit.clamp(1, CONTROL_PLANE_MAX_LIST_LIMIT));
        let returned_count = sessions.len();
        Ok(ControlPlaneAcpSessionListView {
            current_session_id: self.current_session_id.clone(),
            matched_count,
            returned_count,
            sessions,
        })
    }

    pub async fn read_session(
        &self,
        session_key: &str,
    ) -> Result<Option<ControlPlaneAcpSessionReadView>, String> {
        let session_key = session_key.trim();
        if session_key.is_empty() {
            return Err("control_plane_acp_session_key_missing".to_owned());
        }

        let store = self.open_store();
        let Some(metadata) = store.get(session_key)? else {
            return Ok(None);
        };

        let repo = self.open_visibility_repo()?;
        if !self.is_visible_acp_session(repo.as_ref(), &metadata)? {
            return Err(format!(
                "visibility_denied: ACP session `{session_key}` is not visible from `{}`",
                self.current_session_id
            ));
        }

        let manager = acquire_shared_acp_session_manager(&self.config)?;
        let status = match manager.get_status(&self.config, session_key).await {
            Ok(status) => status,
            Err(error) => {
                fallback_acp_session_status(&metadata, Some(format!("status_unavailable: {error}")))
            }
        };
        Ok(Some(ControlPlaneAcpSessionReadView {
            current_session_id: self.current_session_id.clone(),
            metadata,
            status,
        }))
    }

    fn open_store(&self) -> AcpSqliteSessionStore {
        AcpSqliteSessionStore::new(Some(self.config.memory.resolved_sqlite_path()))
    }

    fn visible_acp_sessions(&self) -> Result<Vec<AcpSessionMetadata>, String> {
        let store = self.open_store();
        let repo = self.open_visibility_repo()?;
        let mut sessions = Vec::new();
        for metadata in store.list()? {
            if self.is_visible_acp_session(repo.as_ref(), &metadata)? {
                sessions.push(metadata);
            }
        }
        Ok(sessions)
    }

    fn open_visibility_repo(&self) -> Result<Option<SessionRepository>, String> {
        if self.current_session_id == DEFAULT_CONTROL_PLANE_SESSION_ID {
            return Ok(None);
        }
        let memory_config = store::session_store_config_from_memory_config_without_env_overrides(
            &self.config.memory,
        );
        SessionRepository::new(&memory_config).map(Some)
    }

    fn is_visible_acp_session(
        &self,
        repo: Option<&SessionRepository>,
        metadata: &AcpSessionMetadata,
    ) -> Result<bool, String> {
        if self.current_session_id == DEFAULT_CONTROL_PLANE_SESSION_ID {
            return Ok(true);
        }
        let Some(repo) = repo else {
            return Ok(false);
        };
        let Some(binding) = metadata.binding.as_ref() else {
            return Ok(false);
        };
        if binding.route_session_id == self.current_session_id {
            return Ok(true);
        }
        repo.is_session_visible(&self.current_session_id, &binding.route_session_id)
    }
}

#[cfg(feature = "memory-sqlite")]
fn fallback_acp_session_status(
    metadata: &AcpSessionMetadata,
    status_error: Option<String>,
) -> AcpSessionStatus {
    AcpSessionStatus {
        session_key: metadata.session_key.clone(),
        backend_id: metadata.backend_id.clone(),
        conversation_id: metadata.conversation_id.clone(),
        binding: metadata.binding.clone(),
        activation_origin: metadata.activation_origin,
        state: metadata.state,
        mode: metadata.mode,
        pending_turns: 0,
        active_turn_id: None,
        last_activity_ms: metadata.last_activity_ms,
        last_error: status_error.or_else(|| metadata.last_error.clone()),
    }
}

#[cfg(feature = "memory-sqlite")]
fn normalize_control_plane_session_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        DEFAULT_CONTROL_PLANE_SESSION_ID.to_owned()
    } else {
        trimmed.to_owned()
    }
}


#[cfg(test)]
#[path = "control_plane_tests.rs"]
mod tests;
