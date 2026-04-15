use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::PROTOCOL_VERSION;

pub const CONTROL_PLANE_PROTOCOL_VERSION: u32 = PROTOCOL_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneRole {
    Operator,
    Node,
}

impl ControlPlaneRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Node => "node",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ControlPlaneScope {
    OperatorRead,
    OperatorWrite,
    OperatorAdmin,
    OperatorApprovals,
    OperatorPairing,
    OperatorAcp,
}

impl ControlPlaneScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OperatorRead => "operator.read",
            Self::OperatorWrite => "operator.write",
            Self::OperatorAdmin => "operator.admin",
            Self::OperatorApprovals => "operator.approvals",
            Self::OperatorPairing => "operator.pairing",
            Self::OperatorAcp => "operator.acp",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase().replace('_', ".");
        match normalized.as_str() {
            "operator.read" => Some(Self::OperatorRead),
            "operator.write" => Some(Self::OperatorWrite),
            "operator.admin" => Some(Self::OperatorAdmin),
            "operator.approvals" => Some(Self::OperatorApprovals),
            "operator.pairing" => Some(Self::OperatorPairing),
            "operator.acp" => Some(Self::OperatorAcp),
            _ => None,
        }
    }
}

impl Serialize for ControlPlaneScope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ControlPlaneScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        ControlPlaneScope::parse(raw.as_str())
            .ok_or_else(|| serde::de::Error::custom(format!("unknown control-plane scope `{raw}`")))
    }
}

struct RedactedSecretDebug;

impl fmt::Debug for RedactedSecretDebug {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

struct RedactedSecretOptionDebug(bool);

impl fmt::Debug for RedactedSecretOptionDebug {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 {
            formatter.write_str("Some(<redacted>)")
        } else {
            formatter.write_str("None")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneClientIdentity {
    pub id: String,
    pub version: String,
    pub mode: String,
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneDeviceIdentity {
    pub device_id: String,
    pub public_key: String,
    pub signature: String,
    pub signed_at_ms: u64,
    pub nonce: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ControlPlaneAuthClaims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl fmt::Debug for ControlPlaneAuthClaims {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let token = RedactedSecretOptionDebug(self.token.is_some());
        let device_token = RedactedSecretOptionDebug(self.device_token.is_some());
        let bootstrap_token = RedactedSecretOptionDebug(self.bootstrap_token.is_some());
        let password = RedactedSecretOptionDebug(self.password.is_some());
        formatter
            .debug_struct("ControlPlaneAuthClaims")
            .field("token", &token)
            .field("device_token", &device_token)
            .field("bootstrap_token", &bootstrap_token)
            .field("password", &password)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ControlPlaneStateVersion {
    pub presence: u64,
    pub health: u64,
    pub sessions: u64,
    pub approvals: u64,
    pub acp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSnapshot {
    pub state_version: ControlPlaneStateVersion,
    pub presence_count: usize,
    pub session_count: usize,
    pub pending_approval_count: usize,
    pub acp_session_count: usize,
    pub runtime_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlanePolicy {
    pub max_payload_bytes: usize,
    pub max_buffered_bytes: usize,
    pub tick_interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlanePrincipal {
    pub connection_id: String,
    pub client_id: String,
    pub role: ControlPlaneRole,
    pub scopes: BTreeSet<ControlPlaneScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneConnectRequest {
    pub min_protocol: u32,
    pub max_protocol: u32,
    pub client: ControlPlaneClientIdentity,
    pub role: ControlPlaneRole,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub scopes: BTreeSet<ControlPlaneScope>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub caps: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub commands: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<String, bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ControlPlaneAuthClaims>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<ControlPlaneDeviceIdentity>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneConnectResponse {
    pub protocol: u32,
    pub principal: ControlPlanePrincipal,
    pub connection_token: String,
    pub connection_token_expires_at_ms: u64,
    pub snapshot: ControlPlaneSnapshot,
    pub policy: ControlPlanePolicy,
}

impl fmt::Debug for ControlPlaneConnectResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlPlaneConnectResponse")
            .field("protocol", &self.protocol)
            .field("principal", &self.principal)
            .field("connection_token", &RedactedSecretDebug)
            .field(
                "connection_token_expires_at_ms",
                &self.connection_token_expires_at_ms,
            )
            .field("snapshot", &self.snapshot)
            .field("policy", &self.policy)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSnapshotResponse {
    pub snapshot: ControlPlaneSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneChallengeResponse {
    pub nonce: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlanePairingRequestSummary {
    pub pairing_request_id: String,
    pub device_id: String,
    pub client_id: String,
    pub public_key: String,
    pub role: ControlPlaneRole,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub requested_scopes: BTreeSet<ControlPlaneScope>,
    pub status: ControlPlanePairingStatus,
    pub requested_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlanePairingListResponse {
    pub matched_count: usize,
    pub returned_count: usize,
    pub requests: Vec<ControlPlanePairingRequestSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlanePairingResolveRequest {
    pub pairing_request_id: String,
    pub approve: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlanePairingResolveResponse {
    pub request: ControlPlanePairingRequestSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
}

impl fmt::Debug for ControlPlanePairingResolveResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let device_token = RedactedSecretOptionDebug(self.device_token.is_some());
        formatter
            .debug_struct("ControlPlanePairingResolveResponse")
            .field("request", &self.request)
            .field("device_token", &device_token)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneConnectErrorCode {
    ProtocolMismatch,
    ChallengeRequired,
    ChallengeExpired,
    SharedTokenRequired,
    SharedTokenInvalid,
    DeviceSignatureInvalid,
    PairingRequired,
    DeviceTokenRequired,
    DeviceTokenInvalid,
}

impl ControlPlaneConnectErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::ChallengeRequired => "challenge_required",
            Self::ChallengeExpired => "challenge_expired",
            Self::SharedTokenRequired => "shared_token_required",
            Self::SharedTokenInvalid => "shared_token_invalid",
            Self::DeviceSignatureInvalid => "device_signature_invalid",
            Self::PairingRequired => "pairing_required",
            Self::DeviceTokenRequired => "device_token_required",
            Self::DeviceTokenInvalid => "device_token_invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneConnectErrorResponse {
    pub code: ControlPlaneConnectErrorCode,
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneSessionKind {
    Root,
    DelegateChild,
}

impl ControlPlaneSessionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::DelegateChild => "delegate_child",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneSessionState {
    Ready,
    Running,
    Completed,
    Failed,
    TimedOut,
}

impl ControlPlaneSessionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSessionSummary {
    pub session_id: String,
    pub kind: ControlPlaneSessionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub state: ControlPlaneSessionState,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<i64>,
    pub turn_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSessionEvent {
    pub id: i64,
    pub session_id: String,
    pub event_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_session_id: Option<String>,
    pub payload: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSessionTerminalOutcome {
    pub session_id: String,
    pub status: String,
    pub payload: Value,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSessionObservation {
    pub session: ControlPlaneSessionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_outcome: Option<ControlPlaneSessionTerminalOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_events: Vec<ControlPlaneSessionEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tail_events: Vec<ControlPlaneSessionEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSessionListResponse {
    pub current_session_id: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub sessions: Vec<ControlPlaneSessionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneSessionReadResponse {
    pub current_session_id: String,
    pub observation: ControlPlaneSessionObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneApprovalRequestStatus {
    Pending,
    Approved,
    Executing,
    Executed,
    Denied,
    Expired,
    Cancelled,
}

impl ControlPlaneApprovalRequestStatus {
    pub const fn as_str(self) -> &'static str {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneApprovalDecision {
    ApproveOnce,
    ApproveAlways,
    Deny,
}

impl ControlPlaneApprovalDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::ApproveAlways => "approve_always",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneApprovalSummary {
    pub approval_request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub approval_key: String,
    pub status: ControlPlaneApprovalRequestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ControlPlaneApprovalDecision>,
    pub requested_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneApprovalListResponse {
    pub current_session_id: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub approvals: Vec<ControlPlaneApprovalSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneAcpSessionState {
    Initializing,
    Ready,
    Busy,
    Cancelling,
    Error,
    Closed,
}

impl ControlPlaneAcpSessionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Busy => "busy",
            Self::Cancelling => "cancelling",
            Self::Error => "error",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneAcpSessionMode {
    Interactive,
    Background,
    Review,
}

impl ControlPlaneAcpSessionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Background => "background",
            Self::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneAcpRoutingOrigin {
    ExplicitRequest,
    AutomaticAgentPrefixed,
    AutomaticDispatch,
}

impl ControlPlaneAcpRoutingOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitRequest => "explicit_request",
            Self::AutomaticAgentPrefixed => "automatic_agent_prefixed",
            Self::AutomaticDispatch => "automatic_dispatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneAcpBindingScope {
    pub route_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneAcpSessionMetadata {
    pub session_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<ControlPlaneAcpBindingScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_origin: Option<ControlPlaneAcpRoutingOrigin>,
    pub backend_id: String,
    pub runtime_session_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ControlPlaneAcpSessionMode>,
    pub state: ControlPlaneAcpSessionState,
    pub last_activity_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneAcpSessionStatus {
    pub session_key: String,
    pub backend_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<ControlPlaneAcpBindingScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_origin: Option<ControlPlaneAcpRoutingOrigin>,
    pub state: ControlPlaneAcpSessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ControlPlaneAcpSessionMode>,
    pub pending_turns: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn_id: Option<String>,
    pub last_activity_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneAcpSessionListResponse {
    pub current_session_id: String,
    pub matched_count: usize,
    pub returned_count: usize,
    pub sessions: Vec<ControlPlaneAcpSessionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneAcpSessionReadResponse {
    pub current_session_id: String,
    pub metadata: ControlPlaneAcpSessionMetadata,
    pub status: ControlPlaneAcpSessionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneTurnSubmitRequest {
    pub session_id: String,
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneTurnSummary {
    pub turn_id: String,
    pub session_id: String,
    pub status: ControlPlaneTurnStatus,
    pub submitted_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneTurnSubmitResponse {
    pub turn: ControlPlaneTurnSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlPlaneTurnResultResponse {
    pub turn: ControlPlaneTurnSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlPlaneTurnEventEnvelope {
    pub turn_id: String,
    pub session_id: String,
    pub seq: u64,
    pub terminal: bool,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneEventName {
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

impl ControlPlaneEventName {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneEventEnvelope {
    pub event: ControlPlaneEventName,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_version: Option<ControlPlaneStateVersion>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPlaneRecentEventsResponse {
    pub events: Vec<ControlPlaneEventEnvelope>,
}
