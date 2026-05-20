use super::*;

pub(super) fn session_root_node_id(session_id: &str) -> String {
    format!("session-root:{session_id}")
}

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

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionNodeKind {
    Root,
    UserTurn,
    AssistantTurn,
    Artifact,
}

impl SessionNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::UserTurn => "user_turn",
            Self::AssistantTurn => "assistant_turn",
            Self::Artifact => "artifact",
        }
    }

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "root" => Ok(Self::Root),
            "user_turn" => Ok(Self::UserTurn),
            "assistant_turn" => Ok(Self::AssistantTurn),
            "artifact" => Ok(Self::Artifact),
            _ => Err(format!("unknown session node kind `{value}`")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHeadMode {
    Live,
    Pinned,
}

impl SessionHeadMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Pinned => "pinned",
        }
    }

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "live" => Ok(Self::Live),
            "pinned" => Ok(Self::Pinned),
            _ => Err(format!("unknown session head mode `{value}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionNodeRecord {
    pub node_id: String,
    pub session_id: String,
    pub parent_node_id: Option<String>,
    pub kind: SessionNodeKind,
    pub role: Option<String>,
    pub content: Option<String>,
    pub session_turn_index: Option<i64>,
    pub metadata_json: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHeadRecord {
    pub session_id: String,
    pub head_name: String,
    pub node_id: String,
    pub mode: SessionHeadMode,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRouteBindingRecord {
    pub route_session_id: String,
    pub active_session_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionArtifactKind {
    Checkpoint,
    BranchSummary,
    CompactionSummary,
    Handoff,
    Note,
}

impl SessionArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checkpoint => "checkpoint",
            Self::BranchSummary => "branch_summary",
            Self::CompactionSummary => "compaction_summary",
            Self::Handoff => "handoff",
            Self::Note => "note",
        }
    }

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "checkpoint" => Ok(Self::Checkpoint),
            "branch_summary" => Ok(Self::BranchSummary),
            "compaction_summary" => Ok(Self::CompactionSummary),
            "handoff" => Ok(Self::Handoff),
            "note" => Ok(Self::Note),
            _ => Err(format!("unknown session artifact kind `{value}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionArtifactRecord {
    pub artifact_id: String,
    pub session_id: String,
    pub kind: SessionArtifactKind,
    pub head_name: Option<String>,
    pub anchor_node_id: Option<String>,
    pub source_start_node_id: Option<String>,
    pub source_end_node_id: Option<String>,
    pub payload_json: Value,
    pub summary_text: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSessionArtifactRecord {
    pub artifact_id: String,
    pub session_id: String,
    pub kind: SessionArtifactKind,
    pub head_name: Option<String>,
    pub anchor_node_id: Option<String>,
    pub source_start_node_id: Option<String>,
    pub source_end_node_id: Option<String>,
    pub payload_json: Value,
    pub summary_text: Option<String>,
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

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
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
    pub frozen_result: Option<FrozenResult>,
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

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
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

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
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

    pub(super) fn from_db(value: &str) -> Result<Self, String> {
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

#[derive(Debug, Clone, PartialEq)]
pub struct SessionTrajectoryReadSnapshot {
    pub summary: SessionSummaryRecord,
    pub lineage_root_session_id: Option<String>,
    pub lineage_depth: usize,
    pub turns: Vec<SessionTranscriptTurn>,
    pub events: Vec<SessionEventRecord>,
    pub approval_requests: Vec<ApprovalRequestRecord>,
    pub terminal_outcome: Option<SessionTerminalOutcomeRecord>,
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
    pub frozen_result: Option<FrozenResult>,
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
pub(super) struct RawSessionRecord {
    pub(super) session_id: String,
    pub(super) kind: String,
    pub(super) parent_session_id: Option<String>,
    pub(super) label: Option<String>,
    pub(super) state: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) last_error: Option<String>,
}

#[derive(Debug)]
pub(super) struct RawSessionSummaryRecord {
    pub(super) session_id: String,
    pub(super) kind: String,
    pub(super) parent_session_id: Option<String>,
    pub(super) label: Option<String>,
    pub(super) state: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) last_error: Option<String>,
    pub(super) archived_at: Option<i64>,
    pub(super) turn_count: i64,
    pub(super) last_turn_at: Option<i64>,
}

#[derive(Debug)]
pub(super) struct RawSessionEventRecord {
    pub(super) id: i64,
    pub(super) session_id: String,
    pub(super) event_kind: String,
    pub(super) actor_session_id: Option<String>,
    pub(super) payload_json: String,
    pub(super) ts: i64,
}

#[derive(Debug)]
pub(super) struct RawSessionNodeRecord {
    pub(super) node_id: String,
    pub(super) session_id: String,
    pub(super) parent_node_id: Option<String>,
    pub(super) node_kind: String,
    pub(super) role: Option<String>,
    pub(super) content: Option<String>,
    pub(super) session_turn_index: Option<i64>,
    pub(super) metadata_json: String,
    pub(super) created_at: i64,
}

#[derive(Debug)]
pub(super) struct RawSessionHeadRecord {
    pub(super) session_id: String,
    pub(super) head_name: String,
    pub(super) node_id: String,
    pub(super) head_mode: String,
    pub(super) updated_at: i64,
}

#[derive(Debug)]
pub(super) struct RawSessionArtifactRecord {
    pub(super) artifact_id: String,
    pub(super) session_id: String,
    pub(super) artifact_type: String,
    pub(super) head_name: Option<String>,
    pub(super) anchor_node_id: Option<String>,
    pub(super) source_start_node_id: Option<String>,
    pub(super) source_end_node_id: Option<String>,
    pub(super) payload_json: String,
    pub(super) summary_text: Option<String>,
    pub(super) created_at: i64,
}

#[derive(Debug)]
pub(super) struct RawSessionSearchTurnRecord {
    pub(super) id: i64,
    pub(super) session_id: String,
    pub(super) role: String,
    pub(super) content: String,
    pub(super) ts: i64,
}

#[derive(Debug)]
pub(super) struct RawSessionSearchEventRecord {
    pub(super) id: i64,
    pub(super) session_id: String,
    pub(super) event_kind: String,
    pub(super) payload_json: String,
    pub(super) ts: i64,
}

#[derive(Debug)]
pub(super) struct RawSessionTerminalOutcomeRecord {
    pub(super) session_id: String,
    pub(super) status: String,
    pub(super) payload_json: String,
    pub(super) frozen_result_json: Option<String>,
    pub(super) recorded_at: i64,
}

#[derive(Debug)]
pub(super) struct RawApprovalRequestRecord {
    pub(super) approval_request_id: String,
    pub(super) session_id: String,
    pub(super) turn_id: String,
    pub(super) tool_call_id: String,
    pub(super) tool_name: String,
    pub(super) approval_key: String,
    pub(super) status: String,
    pub(super) decision: Option<String>,
    pub(super) request_payload_json: String,
    pub(super) governance_snapshot_json: String,
    pub(super) requested_at: i64,
    pub(super) resolved_at: Option<i64>,
    pub(super) resolved_by_session_id: Option<String>,
    pub(super) executed_at: Option<i64>,
    pub(super) last_error: Option<String>,
}

#[derive(Debug)]
pub(super) struct RawApprovalGrantRecord {
    pub(super) scope_session_id: String,
    pub(super) approval_key: String,
    pub(super) created_by_session_id: Option<String>,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(Debug)]
pub(super) struct RawControlPlanePairingRequestRecord {
    pub(super) pairing_request_id: String,
    pub(super) device_id: String,
    pub(super) client_id: String,
    pub(super) public_key: String,
    pub(super) role: String,
    pub(super) requested_scopes_json: String,
    pub(super) status: String,
    pub(super) requested_at_ms: i64,
    pub(super) resolved_at_ms: Option<i64>,
    pub(super) issued_token_id: Option<String>,
    pub(super) last_error: Option<String>,
}

#[derive(Debug)]
pub(super) struct RawControlPlaneDeviceTokenRecord {
    pub(super) token_id: String,
    pub(super) device_id: String,
    pub(super) public_key: String,
    pub(super) role: String,
    pub(super) approved_scopes_json: String,
    pub(super) token_hash: String,
    pub(super) issued_at_ms: i64,
    pub(super) expires_at_ms: Option<i64>,
    pub(super) revoked_at_ms: Option<i64>,
    pub(super) last_used_at_ms: Option<i64>,
    pub(super) pairing_request_id: Option<String>,
}

#[derive(Debug)]
pub(super) struct RawSessionToolConsentRecord {
    pub(super) scope_session_id: String,
    pub(super) mode: String,
    pub(super) updated_by_session_id: Option<String>,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

#[derive(Debug)]
pub(super) struct RawSessionToolPolicyRecord {
    pub(super) session_id: String,
    pub(super) requested_tool_ids_json: String,
    pub(super) runtime_narrowing_json: String,
    pub(super) updated_at: i64,
}

impl SessionRecord {
    pub(super) fn try_from_raw(raw: RawSessionRecord) -> Result<Self, String> {
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
    pub(super) fn try_from_raw(raw: RawSessionSummaryRecord) -> Result<Self, String> {
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
    pub(super) fn try_from_raw(raw: RawSessionEventRecord) -> Result<Self, String> {
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

impl SessionNodeRecord {
    pub(super) fn try_from_raw(raw: RawSessionNodeRecord) -> Result<Self, String> {
        Ok(Self {
            node_id: raw.node_id,
            session_id: raw.session_id,
            parent_node_id: raw.parent_node_id,
            kind: SessionNodeKind::from_db(&raw.node_kind)?,
            role: raw.role,
            content: raw.content,
            session_turn_index: raw.session_turn_index,
            metadata_json: serde_json::from_str(&raw.metadata_json)
                .map_err(|error| format!("decode session node metadata failed: {error}"))?,
            created_at: raw.created_at,
        })
    }
}

impl SessionHeadRecord {
    pub(super) fn from_raw(raw: RawSessionHeadRecord) -> Result<Self, String> {
        Ok(Self {
            session_id: raw.session_id,
            head_name: raw.head_name,
            node_id: raw.node_id,
            mode: SessionHeadMode::from_db(&raw.head_mode)?,
            updated_at: raw.updated_at,
        })
    }
}

impl SessionArtifactRecord {
    pub(super) fn try_from_raw(raw: RawSessionArtifactRecord) -> Result<Self, String> {
        Ok(Self {
            artifact_id: raw.artifact_id,
            session_id: raw.session_id,
            kind: SessionArtifactKind::from_db(&raw.artifact_type)?,
            head_name: raw.head_name,
            anchor_node_id: raw.anchor_node_id,
            source_start_node_id: raw.source_start_node_id,
            source_end_node_id: raw.source_end_node_id,
            payload_json: serde_json::from_str(&raw.payload_json)
                .map_err(|error| format!("decode session artifact payload failed: {error}"))?,
            summary_text: raw.summary_text,
            created_at: raw.created_at,
        })
    }
}

impl SessionTerminalOutcomeRecord {
    pub(super) fn try_from_raw(raw: RawSessionTerminalOutcomeRecord) -> Result<Self, String> {
        let payload_json = serde_json::from_str(&raw.payload_json)
            .map_err(|error| format!("decode session terminal outcome payload failed: {error}"))?;
        let frozen_result = decode_optional_frozen_result(raw.frozen_result_json)?;

        Ok(Self {
            session_id: raw.session_id,
            status: raw.status,
            payload_json,
            frozen_result,
            recorded_at: raw.recorded_at,
        })
    }
}

pub(super) fn encode_optional_frozen_result(
    frozen_result: &Option<FrozenResult>,
) -> Result<Option<String>, String> {
    let Some(frozen_result) = frozen_result else {
        return Ok(None);
    };

    let encoded_frozen_result = serde_json::to_string(frozen_result)
        .map_err(|error| format!("encode frozen session result failed: {error}"))?;

    Ok(Some(encoded_frozen_result))
}

pub(super) fn decode_optional_frozen_result(
    raw_frozen_result: Option<String>,
) -> Result<Option<FrozenResult>, String> {
    let Some(raw_frozen_result) = raw_frozen_result else {
        return Ok(None);
    };

    let frozen_result = serde_json::from_str(&raw_frozen_result)
        .map_err(|error| format!("decode frozen session result failed: {error}"))?;

    Ok(Some(frozen_result))
}

impl ApprovalRequestRecord {
    pub(super) fn try_from_raw(raw: RawApprovalRequestRecord) -> Result<Self, String> {
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
    pub(super) fn try_from_raw(raw: RawApprovalGrantRecord) -> Result<Self, String> {
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
    pub(super) fn try_from_raw(raw: RawSessionToolConsentRecord) -> Result<Self, String> {
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
    pub(super) fn try_from_raw(raw: RawSessionToolPolicyRecord) -> Result<Self, String> {
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
    pub(super) fn try_from_raw(raw: RawControlPlanePairingRequestRecord) -> Result<Self, String> {
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
    pub(super) fn try_from_raw(raw: RawControlPlaneDeviceTokenRecord) -> Result<Self, String> {
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

pub(super) fn encode_string_set_json(values: &BTreeSet<String>) -> Result<String, String> {
    let normalized = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    serde_json::to_string(&normalized)
        .map_err(|error| format!("encode control-plane scope set failed: {error}"))
}

pub(super) fn decode_string_set_json(encoded: &str) -> Result<BTreeSet<String>, String> {
    let decoded = serde_json::from_str::<BTreeSet<String>>(encoded)
        .map_err(|error| format!("decode control-plane scope set failed: {error}"))?;
    Ok(decoded
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>())
}

pub(super) fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("session repository requires {field_name}"));
    }
    Ok(trimmed.to_owned())
}

pub(super) fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

pub(super) fn seed_session_tree_for_new_session(
    conn: &Connection,
    session_id: &str,
    ts: i64,
) -> Result<(), String> {
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
        params![root_node_id, session_id, ts],
    )
    .map_err(|error| format!("seed session root node failed: {error}"))?;
    upsert_session_head_with_conn(
        conn,
        session_id,
        ACTIVE_SESSION_HEAD_NAME,
        &root_node_id,
        SessionHeadMode::Live,
        ts,
    )?;
    Ok(())
}

pub(super) fn upsert_session_head_with_conn(
    conn: &Connection,
    session_id: &str,
    head_name: &str,
    node_id: &str,
    head_mode: SessionHeadMode,
    updated_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_heads(session_id, head_name, node_id, head_mode, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(session_id, head_name) DO UPDATE SET
            node_id = excluded.node_id,
            head_mode = excluded.head_mode,
            updated_at = excluded.updated_at",
        params![
            session_id,
            head_name,
            node_id,
            head_mode.as_str(),
            updated_at
        ],
    )
    .map(|_| ())
    .map_err(|error| format!("upsert session head failed: {error}"))
}

pub(super) fn normalize_tool_id_list(tool_ids: Vec<String>) -> Vec<String> {
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

pub(super) fn unix_ts_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(super) fn unix_time_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

pub(super) fn infer_legacy_session_kind(session_id: &str) -> SessionKind {
    if session_id.starts_with("delegate:") {
        SessionKind::DelegateChild
    } else {
        SessionKind::Root
    }
}

pub(super) fn is_resumable_root_session_summary(summary: &SessionSummaryRecord) -> bool {
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

pub(super) fn sort_session_summaries(sessions: &mut [SessionSummaryRecord]) {
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
}

pub(super) fn build_search_fts_query(normalized_query: &str) -> Option<String> {
    crate::search_text::build_search_fts_query(normalized_query, 6)
}

pub(super) fn session_event_search_text(event_kind: &str, payload_json: &str) -> String {
    build_search_index_text(&[event_kind, payload_json])
}
