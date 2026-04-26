use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::Router;
use axum::extract::Query;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::stream::{self, Stream};
use kernel::{
    Capability, CapabilityToken, ExecutionPlane, InMemoryAuditSink, LoongKernel, PlaneTier,
    StaticPolicyEngine, VerticalPackManifest,
};
use loong_protocol::{
    CONTROL_PLANE_PROTOCOL_VERSION, ControlPlaneAcpBindingScope, ControlPlaneAcpRoutingOrigin,
    ControlPlaneAcpSessionListResponse, ControlPlaneAcpSessionMetadata, ControlPlaneAcpSessionMode,
    ControlPlaneAcpSessionReadResponse, ControlPlaneAcpSessionState, ControlPlaneAcpSessionStatus,
    ControlPlaneApprovalDecision, ControlPlaneApprovalListResponse,
    ControlPlaneApprovalRequestStatus, ControlPlaneApprovalSummary, ControlPlaneChallengeResponse,
    ControlPlaneConnectErrorCode, ControlPlaneConnectErrorResponse, ControlPlaneConnectRequest,
    ControlPlaneConnectResponse, ControlPlaneEventEnvelope, ControlPlaneEventName,
    ControlPlanePairingListResponse, ControlPlanePairingRequestSummary,
    ControlPlanePairingResolveRequest, ControlPlanePairingResolveResponse,
    ControlPlanePairingStatus, ControlPlanePolicy, ControlPlanePrincipal,
    ControlPlaneRecentEventsResponse, ControlPlaneScope, ControlPlaneSessionEvent,
    ControlPlaneSessionKind, ControlPlaneSessionListResponse, ControlPlaneSessionObservation,
    ControlPlaneSessionReadResponse, ControlPlaneSessionState, ControlPlaneSessionSummary,
    ControlPlaneSessionTerminalOutcome, ControlPlaneSessionWorkflow,
    ControlPlaneSessionWorkflowBinding, ControlPlaneSessionWorkflowBindingWorktree,
    ControlPlaneSessionWorkflowContinuity, ControlPlaneSnapshot, ControlPlaneSnapshotResponse,
    ControlPlaneStateVersion, ControlPlaneTaskListResponse, ControlPlaneTaskReadResponse,
    ControlPlaneTaskSummary, ControlPlaneTurnSubmitRequest, ControlPlaneTurnSubmitResponse,
    ProtocolRouter,
};
use serde::Deserialize;

use crate::control_plane_turn_runtime::{
    ControlPlaneTurnRuntime, map_turn_result, map_turn_summary,
};
use crate::{CliResult, mvp};

#[cfg(test)]
use crate::control_plane_turn_runtime::{initial_turn_stream_state, next_turn_sse_item};
#[cfg(test)]
use axum::body::{Body, to_bytes};
#[cfg(test)]
use axum::http::Request;
#[cfg(test)]
use ed25519_dalek::{Signer, SigningKey};
#[cfg(test)]
use loong_protocol::{
    ControlPlaneClientIdentity, ControlPlaneRole, ControlPlaneTurnResultResponse,
    ControlPlaneTurnStatus,
};
#[cfg(test)]
use tower::ServiceExt;

const CONTROL_PLANE_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const CONTROL_PLANE_MAX_BUFFERED_BYTES: usize = 256 * 1024;
const CONTROL_PLANE_TICK_INTERVAL_MS: u64 = 15_000;
pub(crate) const CONTROL_PLANE_DEFAULT_EVENT_LIMIT: usize = 50;
const CONTROL_PLANE_DEFAULT_LIST_LIMIT: usize = 50;
const CONTROL_PLANE_DEFAULT_SESSION_RECENT_LIMIT: usize = 20;
const CONTROL_PLANE_DEFAULT_SESSION_TAIL_LIMIT: usize = 50;
const CONTROL_PLANE_CHALLENGE_MAX_FUTURE_SKEW_MS: u64 = 10_000;
const CONTROL_PLANE_PACK_ID: &str = "control-plane";
const CONTROL_PLANE_PACK_DOMAIN: &str = "control";
const CONTROL_PLANE_PACK_VERSION: &str = "1.0.0";
const CONTROL_PLANE_PRIMARY_ADAPTER: &str = "control-plane";
const CONTROL_PLANE_KEEPALIVE_TEXT: &str = "keep-alive";
const CONTROL_PLANE_REMOTE_BOOTSTRAP_SCOPES: [ControlPlaneScope; 2] = [
    ControlPlaneScope::OperatorRead,
    ControlPlaneScope::OperatorPairing,
];

#[derive(Debug, Clone)]
struct ControlPlaneExposurePolicy {
    bind_addr: SocketAddr,
    shared_token: Option<String>,
}

impl ControlPlaneExposurePolicy {
    fn requires_remote_auth(&self) -> bool {
        !self.bind_addr.ip().is_loopback()
    }
}

fn default_loopback_exposure_policy() -> ControlPlaneExposurePolicy {
    ControlPlaneExposurePolicy {
        bind_addr: default_control_plane_bind_addr(0),
        shared_token: None,
    }
}

struct ControlPlaneKernelAuthority {
    kernel: LoongKernel<StaticPolicyEngine>,
    _audit: Arc<InMemoryAuditSink>,
    token_bindings: std::sync::RwLock<std::collections::BTreeMap<String, CapabilityToken>>,
}

#[derive(Clone)]
struct ControlPlaneHttpState {
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    connection_counter: Arc<AtomicU64>,
    connection_registry: Arc<mvp::control_plane::ControlPlaneConnectionRegistry>,
    challenge_registry: Arc<mvp::control_plane::ControlPlaneChallengeRegistry>,
    pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    kernel_authority: Arc<ControlPlaneKernelAuthority>,
    exposure_policy: Arc<ControlPlaneExposurePolicy>,
    #[cfg(feature = "memory-sqlite")]
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    #[cfg(feature = "memory-sqlite")]
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    turn_runtime: Option<Arc<ControlPlaneTurnRuntime>>,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    include_targeted: bool,
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SessionListQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct SessionReadQuery {
    session_id: String,
    #[serde(default)]
    recent_event_limit: Option<usize>,
    #[serde(default)]
    tail_after_id: Option<i64>,
    #[serde(default)]
    tail_page_limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TaskListQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct TaskReadQuery {
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalListQuery {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AcpSessionListQuery {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AcpSessionReadQuery {
    session_key: String,
}

#[derive(Debug, Deserialize)]
struct PairingListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SubscribeQuery {
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default)]
    include_targeted: bool,
}

#[derive(Debug, Deserialize)]
struct TurnResultQuery {
    turn_id: String,
}

#[derive(Debug, Deserialize)]
struct TurnStreamQuery {
    turn_id: String,
    #[serde(default)]
    after_seq: Option<u64>,
}

struct ControlPlaneSubscribeStreamState {
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    pending_events: VecDeque<mvp::control_plane::ControlPlaneEventRecord>,
    receiver: tokio::sync::broadcast::Receiver<mvp::control_plane::ControlPlaneEventRecord>,
    last_seq: u64,
    include_targeted: bool,
}

impl ControlPlaneKernelAuthority {
    fn new() -> Result<Self, String> {
        let kernel_with_audit =
            LoongKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
        let mut kernel = kernel_with_audit.0;
        let audit = kernel_with_audit.1;
        let pack = control_plane_pack();
        let register_result = kernel.register_pack(pack);
        register_result
            .map_err(|error| format!("control-plane pack registration failed: {error}"))?;
        Ok(Self {
            kernel,
            _audit: audit,
            token_bindings: std::sync::RwLock::new(std::collections::BTreeMap::new()),
        })
    }

    fn issue_scoped_token(
        &self,
        connection_token: &str,
        agent_id: &str,
        capabilities: &std::collections::BTreeSet<Capability>,
    ) -> Result<(), String> {
        let token = self
            .kernel
            .issue_scoped_token(CONTROL_PLANE_PACK_ID, agent_id, capabilities, 15 * 60)
            .map_err(|error| format!("control-plane kernel token issuance failed: {error}"))?;
        let mut token_bindings = self
            .token_bindings
            .write()
            .unwrap_or_else(|error| error.into_inner());
        token_bindings.insert(connection_token.to_owned(), token);
        Ok(())
    }

    fn authorize(
        &self,
        connection_token: &str,
        operation: &str,
        capabilities: &std::collections::BTreeSet<Capability>,
    ) -> Result<(), String> {
        let token_bindings = self
            .token_bindings
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let token = token_bindings
            .get(connection_token)
            .ok_or_else(|| "missing control-plane kernel token binding".to_owned())?;
        self.kernel
            .authorize_operation(
                CONTROL_PLANE_PACK_ID,
                token,
                ExecutionPlane::Runtime,
                PlaneTier::Core,
                CONTROL_PLANE_PRIMARY_ADAPTER,
                None,
                operation,
                capabilities,
            )
            .map_err(|error| format!("control-plane kernel authorization failed: {error}"))
    }

    fn remove_binding(&self, connection_token: &str) {
        let mut token_bindings = self
            .token_bindings
            .write()
            .unwrap_or_else(|error| error.into_inner());
        token_bindings.remove(connection_token);
    }
}

fn control_plane_pack() -> VerticalPackManifest {
    let granted_capabilities = std::collections::BTreeSet::from([
        Capability::ControlRead,
        Capability::ControlWrite,
        Capability::ControlApprovals,
        Capability::ControlPairing,
        Capability::ControlAcp,
    ]);
    let default_route = kernel::ExecutionRoute {
        harness_kind: kernel::HarnessKind::EmbeddedPi,
        adapter: None,
    };
    let allowed_connectors = std::collections::BTreeSet::new();
    let metadata = std::collections::BTreeMap::new();
    VerticalPackManifest {
        pack_id: CONTROL_PLANE_PACK_ID.to_owned(),
        domain: CONTROL_PLANE_PACK_DOMAIN.to_owned(),
        version: CONTROL_PLANE_PACK_VERSION.to_owned(),
        default_route,
        allowed_connectors,
        granted_capabilities,
        metadata,
    }
}

fn default_control_plane_bind_addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

fn resolve_control_plane_bind_addr(
    bind_override: Option<&str>,
    port: u16,
) -> Result<SocketAddr, String> {
    let Some(raw_bind_addr) = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(default_control_plane_bind_addr(port));
    };
    raw_bind_addr.parse::<SocketAddr>().map_err(|error| {
        format!("parse control-plane bind address `{raw_bind_addr}` failed: {error}")
    })
}

fn build_control_plane_exposure_policy(
    bind_addr: SocketAddr,
    config: Option<&mvp::config::LoongConfig>,
) -> Result<ControlPlaneExposurePolicy, String> {
    let is_loopback = bind_addr.ip().is_loopback();
    if is_loopback {
        return Ok(ControlPlaneExposurePolicy {
            bind_addr,
            shared_token: None,
        });
    }

    let Some(config) = config else {
        return Err(
            "non-loopback control-plane bind requires --config with control_plane.allow_remote=true"
                .to_owned(),
        );
    };

    if !config.control_plane.allow_remote {
        return Err(
            "non-loopback control-plane bind requires control_plane.allow_remote=true".to_owned(),
        );
    }

    let shared_token = config.control_plane.resolved_shared_token()?;
    let Some(shared_token) = shared_token else {
        return Err(
            "non-loopback control-plane bind requires control_plane.shared_token".to_owned(),
        );
    };

    Ok(ControlPlaneExposurePolicy {
        bind_addr,
        shared_token: Some(shared_token),
    })
}

fn default_policy() -> ControlPlanePolicy {
    ControlPlanePolicy {
        max_payload_bytes: CONTROL_PLANE_MAX_PAYLOAD_BYTES,
        max_buffered_bytes: CONTROL_PLANE_MAX_BUFFERED_BYTES,
        tick_interval_ms: CONTROL_PLANE_TICK_INTERVAL_MS,
    }
}

fn map_snapshot(snapshot: mvp::control_plane::ControlPlaneSnapshotSummary) -> ControlPlaneSnapshot {
    ControlPlaneSnapshot {
        state_version: ControlPlaneStateVersion {
            presence: snapshot.state_version.presence,
            health: snapshot.state_version.health,
            sessions: snapshot.state_version.sessions,
            approvals: snapshot.state_version.approvals,
            acp: snapshot.state_version.acp,
        },
        presence_count: snapshot.presence_count,
        session_count: snapshot.session_count,
        pending_approval_count: snapshot.pending_approval_count,
        acp_session_count: snapshot.acp_session_count,
        runtime_ready: snapshot.runtime_ready,
    }
}

fn map_event_name(kind: mvp::control_plane::ControlPlaneEventKind) -> ControlPlaneEventName {
    match kind {
        mvp::control_plane::ControlPlaneEventKind::PresenceChanged => {
            ControlPlaneEventName::PresenceChanged
        }
        mvp::control_plane::ControlPlaneEventKind::HealthChanged => {
            ControlPlaneEventName::HealthChanged
        }
        mvp::control_plane::ControlPlaneEventKind::SessionChanged => {
            ControlPlaneEventName::SessionChanged
        }
        mvp::control_plane::ControlPlaneEventKind::SessionMessage => {
            ControlPlaneEventName::SessionMessage
        }
        mvp::control_plane::ControlPlaneEventKind::ApprovalRequested => {
            ControlPlaneEventName::ApprovalRequested
        }
        mvp::control_plane::ControlPlaneEventKind::ApprovalResolved => {
            ControlPlaneEventName::ApprovalResolved
        }
        mvp::control_plane::ControlPlaneEventKind::PairingRequested => {
            ControlPlaneEventName::PairingRequested
        }
        mvp::control_plane::ControlPlaneEventKind::PairingResolved => {
            ControlPlaneEventName::PairingResolved
        }
        mvp::control_plane::ControlPlaneEventKind::AcpSessionChanged => {
            ControlPlaneEventName::AcpSessionChanged
        }
        mvp::control_plane::ControlPlaneEventKind::AcpTurnEvent => {
            ControlPlaneEventName::AcpTurnEvent
        }
    }
}

fn map_event(event: mvp::control_plane::ControlPlaneEventRecord) -> ControlPlaneEventEnvelope {
    ControlPlaneEventEnvelope {
        event: map_event_name(event.kind),
        seq: event.seq,
        state_version: Some(ControlPlaneStateVersion {
            presence: event.state_version.presence,
            health: event.state_version.health,
            sessions: event.state_version.sessions,
            approvals: event.state_version.approvals,
            acp: event.state_version.acp,
        }),
        payload: event.payload,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_kind(kind: mvp::session::repository::SessionKind) -> ControlPlaneSessionKind {
    match kind {
        mvp::session::repository::SessionKind::Root => ControlPlaneSessionKind::Root,
        mvp::session::repository::SessionKind::DelegateChild => {
            ControlPlaneSessionKind::DelegateChild
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_state(state: mvp::session::repository::SessionState) -> ControlPlaneSessionState {
    match state {
        mvp::session::repository::SessionState::Ready => ControlPlaneSessionState::Ready,
        mvp::session::repository::SessionState::Running => ControlPlaneSessionState::Running,
        mvp::session::repository::SessionState::Completed => ControlPlaneSessionState::Completed,
        mvp::session::repository::SessionState::Failed => ControlPlaneSessionState::Failed,
        mvp::session::repository::SessionState::TimedOut => ControlPlaneSessionState::TimedOut,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_workflow_continuity(
    continuity: mvp::control_plane::ControlPlaneSessionWorkflowContinuityView,
) -> ControlPlaneSessionWorkflowContinuity {
    ControlPlaneSessionWorkflowContinuity {
        present: continuity.present,
        resolved_identity_present: continuity.resolved_identity_present,
        session_profile_projection_present: continuity.session_profile_projection_present,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_workflow(
    workflow: mvp::control_plane::ControlPlaneSessionWorkflowView,
) -> ControlPlaneSessionWorkflow {
    let runtime_self_continuity = workflow
        .runtime_self_continuity
        .map(map_session_workflow_continuity);
    let binding = workflow.binding.map(map_session_workflow_binding);

    ControlPlaneSessionWorkflow {
        workflow_id: workflow.workflow_id,
        task: workflow.task,
        phase: workflow.phase,
        operation_kind: workflow.operation_kind,
        operation_scope: workflow.operation_scope,
        task_session_id: workflow.task_session_id,
        lineage_root_session_id: workflow.lineage_root_session_id,
        lineage_depth: workflow.lineage_depth,
        runtime_self_continuity,
        binding,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_workflow_binding(
    binding: mvp::control_plane::ControlPlaneSessionWorkflowBindingView,
) -> ControlPlaneSessionWorkflowBinding {
    let worktree = binding
        .worktree
        .map(|worktree| ControlPlaneSessionWorkflowBindingWorktree {
            worktree_id: worktree.worktree_id,
            workspace_root: worktree.workspace_root,
        });

    ControlPlaneSessionWorkflowBinding {
        session_id: binding.session_id,
        task_id: binding.task_id,
        mode: binding.mode,
        execution_surface: binding.execution_surface,
        worktree,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_summary(
    summary: mvp::control_plane::ControlPlaneSessionSummaryView,
) -> ControlPlaneSessionSummary {
    let session = summary.session;
    let workflow = map_session_workflow(summary.workflow);

    ControlPlaneSessionSummary {
        session_id: session.session_id,
        kind: map_session_kind(session.kind),
        parent_session_id: session.parent_session_id,
        label: session.label,
        state: map_session_state(session.state),
        created_at: session.created_at,
        updated_at: session.updated_at,
        archived_at: session.archived_at,
        turn_count: session.turn_count,
        last_turn_at: session.last_turn_at,
        last_error: session.last_error,
        workflow,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_event(
    event: mvp::session::repository::SessionEventRecord,
) -> ControlPlaneSessionEvent {
    ControlPlaneSessionEvent {
        id: event.id,
        session_id: event.session_id,
        event_kind: event.event_kind,
        actor_session_id: event.actor_session_id,
        payload: event.payload_json,
        ts: event.ts,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_terminal_outcome(
    outcome: mvp::session::repository::SessionTerminalOutcomeRecord,
) -> ControlPlaneSessionTerminalOutcome {
    ControlPlaneSessionTerminalOutcome {
        session_id: outcome.session_id,
        status: outcome.status,
        payload: outcome.payload_json,
        recorded_at: outcome.recorded_at,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_session_observation(
    observation: mvp::control_plane::ControlPlaneSessionObservationView,
) -> ControlPlaneSessionObservation {
    ControlPlaneSessionObservation {
        session: map_session_summary(observation.session),
        terminal_outcome: observation
            .terminal_outcome
            .map(map_session_terminal_outcome),
        recent_events: observation
            .recent_events
            .into_iter()
            .map(map_session_event)
            .collect::<Vec<_>>(),
        tail_events: observation
            .tail_events
            .into_iter()
            .map(map_session_event)
            .collect::<Vec<_>>(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_task_summary(
    task: mvp::control_plane::ControlPlaneTaskSummaryView,
) -> ControlPlaneTaskSummary {
    let workflow = map_session_workflow(task.workflow);
    let session_state = task.session_state;
    let delegate_phase = task.delegate_phase;
    let delegate_mode = task.delegate_mode;
    let timeout_seconds = task.timeout_seconds;
    let approval_request_count = task.approval_request_count;
    let approval_attention_count = task.approval_attention_count;
    let requested_tool_ids = task.requested_tool_ids;
    let visible_requested_tool_ids = task.visible_requested_tool_ids;
    let effective_tool_ids = task.effective_tool_ids;
    let visible_effective_tool_ids = task.visible_effective_tool_ids;
    let effective_runtime_narrowing = task.effective_runtime_narrowing;
    let label = task.label;
    let last_error = task.last_error;

    ControlPlaneTaskSummary {
        task_id: task.task_id,
        session_id: task.session_id,
        scope_session_id: task.scope_session_id,
        label,
        session_state,
        delegate_phase,
        delegate_mode,
        timeout_seconds,
        workflow,
        approval_request_count,
        approval_attention_count,
        requested_tool_ids,
        visible_requested_tool_ids,
        effective_tool_ids,
        visible_effective_tool_ids,
        effective_runtime_narrowing,
        last_error,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_approval_status(
    status: mvp::session::repository::ApprovalRequestStatus,
) -> ControlPlaneApprovalRequestStatus {
    match status {
        mvp::session::repository::ApprovalRequestStatus::Pending => {
            ControlPlaneApprovalRequestStatus::Pending
        }
        mvp::session::repository::ApprovalRequestStatus::Approved => {
            ControlPlaneApprovalRequestStatus::Approved
        }
        mvp::session::repository::ApprovalRequestStatus::Executing => {
            ControlPlaneApprovalRequestStatus::Executing
        }
        mvp::session::repository::ApprovalRequestStatus::Executed => {
            ControlPlaneApprovalRequestStatus::Executed
        }
        mvp::session::repository::ApprovalRequestStatus::Denied => {
            ControlPlaneApprovalRequestStatus::Denied
        }
        mvp::session::repository::ApprovalRequestStatus::Expired => {
            ControlPlaneApprovalRequestStatus::Expired
        }
        mvp::session::repository::ApprovalRequestStatus::Cancelled => {
            ControlPlaneApprovalRequestStatus::Cancelled
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_approval_decision(
    decision: mvp::session::repository::ApprovalDecision,
) -> ControlPlaneApprovalDecision {
    match decision {
        mvp::session::repository::ApprovalDecision::ApproveOnce => {
            ControlPlaneApprovalDecision::ApproveOnce
        }
        mvp::session::repository::ApprovalDecision::ApproveAlways => {
            ControlPlaneApprovalDecision::ApproveAlways
        }
        mvp::session::repository::ApprovalDecision::Deny => ControlPlaneApprovalDecision::Deny,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_approval_summary(
    approval: mvp::session::repository::ApprovalRequestRecord,
) -> ControlPlaneApprovalSummary {
    let reason = approval
        .governance_snapshot_json
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let rule_id = approval
        .governance_snapshot_json
        .get("rule_id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let visible_tool_name = Some(mvp::tools::user_visible_tool_name(
        approval.tool_name.as_str(),
    ));
    let raw_request = approval
        .request_payload_json
        .as_object()
        .and_then(|payload| payload.get("args_json"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let summarized_request =
        mvp::tools::summarize_tool_request_for_display(approval.tool_name.as_str(), raw_request);
    let request_summary = Some(serde_json::json!({
        "tool": visible_tool_name.clone().unwrap_or_else(|| approval.tool_name.clone()),
        "request": summarized_request,
    }));
    ControlPlaneApprovalSummary {
        approval_request_id: approval.approval_request_id,
        session_id: approval.session_id,
        turn_id: approval.turn_id,
        tool_call_id: approval.tool_call_id,
        tool_name: approval.tool_name,
        visible_tool_name,
        request_summary,
        approval_key: approval.approval_key,
        status: map_approval_status(approval.status),
        decision: approval.decision.map(map_approval_decision),
        requested_at: approval.requested_at,
        resolved_at: approval.resolved_at,
        resolved_by_session_id: approval.resolved_by_session_id,
        executed_at: approval.executed_at,
        last_error: approval.last_error,
        reason,
        rule_id,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_acp_binding_scope(binding: mvp::acp::AcpSessionBindingScope) -> ControlPlaneAcpBindingScope {
    ControlPlaneAcpBindingScope {
        route_session_id: binding.route_session_id,
        channel_id: binding.channel_id,
        account_id: binding.account_id,
        conversation_id: binding.conversation_id,
        participant_id: binding.participant_id,
        thread_id: binding.thread_id,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_acp_routing_origin(origin: mvp::acp::AcpRoutingOrigin) -> ControlPlaneAcpRoutingOrigin {
    match origin {
        mvp::acp::AcpRoutingOrigin::ExplicitRequest => {
            ControlPlaneAcpRoutingOrigin::ExplicitRequest
        }
        mvp::acp::AcpRoutingOrigin::AutomaticAgentPrefixed => {
            ControlPlaneAcpRoutingOrigin::AutomaticAgentPrefixed
        }
        mvp::acp::AcpRoutingOrigin::AutomaticDispatch => {
            ControlPlaneAcpRoutingOrigin::AutomaticDispatch
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_acp_session_mode(mode: mvp::acp::AcpSessionMode) -> ControlPlaneAcpSessionMode {
    match mode {
        mvp::acp::AcpSessionMode::Interactive => ControlPlaneAcpSessionMode::Interactive,
        mvp::acp::AcpSessionMode::Background => ControlPlaneAcpSessionMode::Background,
        mvp::acp::AcpSessionMode::Review => ControlPlaneAcpSessionMode::Review,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_acp_session_state(state: mvp::acp::AcpSessionState) -> ControlPlaneAcpSessionState {
    match state {
        mvp::acp::AcpSessionState::Initializing => ControlPlaneAcpSessionState::Initializing,
        mvp::acp::AcpSessionState::Ready => ControlPlaneAcpSessionState::Ready,
        mvp::acp::AcpSessionState::Busy => ControlPlaneAcpSessionState::Busy,
        mvp::acp::AcpSessionState::Cancelling => ControlPlaneAcpSessionState::Cancelling,
        mvp::acp::AcpSessionState::Error => ControlPlaneAcpSessionState::Error,
        mvp::acp::AcpSessionState::Closed => ControlPlaneAcpSessionState::Closed,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_acp_session_metadata(
    metadata: mvp::acp::AcpSessionMetadata,
) -> ControlPlaneAcpSessionMetadata {
    ControlPlaneAcpSessionMetadata {
        session_key: metadata.session_key,
        conversation_id: metadata.conversation_id,
        binding: metadata.binding.map(map_acp_binding_scope),
        activation_origin: metadata.activation_origin.map(map_acp_routing_origin),
        backend_id: metadata.backend_id,
        runtime_session_name: metadata.runtime_session_name,
        working_directory: metadata
            .working_directory
            .map(|path| path.display().to_string()),
        backend_session_id: metadata.backend_session_id,
        agent_session_id: metadata.agent_session_id,
        mode: metadata.mode.map(map_acp_session_mode),
        state: map_acp_session_state(metadata.state),
        last_activity_ms: metadata.last_activity_ms,
        last_error: metadata.last_error,
    }
}

#[cfg(feature = "memory-sqlite")]
fn map_acp_session_status(status: mvp::acp::AcpSessionStatus) -> ControlPlaneAcpSessionStatus {
    ControlPlaneAcpSessionStatus {
        session_key: status.session_key,
        backend_id: status.backend_id,
        conversation_id: status.conversation_id,
        binding: status.binding.map(map_acp_binding_scope),
        activation_origin: status.activation_origin.map(map_acp_routing_origin),
        state: map_acp_session_state(status.state),
        mode: status.mode.map(map_acp_session_mode),
        pending_turns: status.pending_turns,
        active_turn_id: status.active_turn_id,
        last_activity_ms: status.last_activity_ms,
        last_error: status.last_error,
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_request_status(
    raw: &str,
) -> Result<mvp::session::repository::ApprovalRequestStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Ok(mvp::session::repository::ApprovalRequestStatus::Pending),
        "approved" => Ok(mvp::session::repository::ApprovalRequestStatus::Approved),
        "executing" => Ok(mvp::session::repository::ApprovalRequestStatus::Executing),
        "executed" => Ok(mvp::session::repository::ApprovalRequestStatus::Executed),
        "denied" => Ok(mvp::session::repository::ApprovalRequestStatus::Denied),
        "expired" => Ok(mvp::session::repository::ApprovalRequestStatus::Expired),
        "cancelled" => Ok(mvp::session::repository::ApprovalRequestStatus::Cancelled),
        _ => Err(format!("unknown approval status `{raw}`")),
    }
}

fn map_pairing_status(
    status: mvp::control_plane::ControlPlanePairingStatus,
) -> ControlPlanePairingStatus {
    match status {
        mvp::control_plane::ControlPlanePairingStatus::Pending => {
            ControlPlanePairingStatus::Pending
        }
        mvp::control_plane::ControlPlanePairingStatus::Approved => {
            ControlPlanePairingStatus::Approved
        }
        mvp::control_plane::ControlPlanePairingStatus::Rejected => {
            ControlPlanePairingStatus::Rejected
        }
    }
}

fn map_pairing_request(
    request: mvp::control_plane::ControlPlanePairingRequestRecord,
) -> ControlPlanePairingRequestSummary {
    ControlPlanePairingRequestSummary {
        pairing_request_id: request.pairing_request_id,
        device_id: request.device_id,
        client_id: request.client_id,
        public_key: request.public_key,
        role: match request.role.as_str() {
            "operator" => loong_protocol::ControlPlaneRole::Operator,
            _ => loong_protocol::ControlPlaneRole::Node,
        },
        requested_scopes: request
            .requested_scopes
            .into_iter()
            .filter_map(|scope| ControlPlaneScope::parse(scope.as_str()))
            .collect::<std::collections::BTreeSet<_>>(),
        status: map_pairing_status(request.status),
        requested_at_ms: request.requested_at_ms,
        resolved_at_ms: request.resolved_at_ms,
    }
}

fn principal_from_connect(
    request: &ControlPlaneConnectRequest,
    connection_id: String,
    granted_scopes: std::collections::BTreeSet<ControlPlaneScope>,
) -> ControlPlanePrincipal {
    ControlPlanePrincipal {
        connection_id,
        client_id: request.client.id.clone(),
        role: request.role,
        scopes: granted_scopes,
        device_id: request
            .device
            .as_ref()
            .map(|device| device.device_id.clone()),
    }
}

fn parse_pairing_status(
    raw: &str,
) -> Result<mvp::control_plane::ControlPlanePairingStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Ok(mvp::control_plane::ControlPlanePairingStatus::Pending),
        "approved" => Ok(mvp::control_plane::ControlPlanePairingStatus::Approved),
        "rejected" => Ok(mvp::control_plane::ControlPlanePairingStatus::Rejected),
        _ => Err(format!("unknown pairing status `{raw}`")),
    }
}

fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    Ok(trimmed.to_owned())
}

fn require_nonempty_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    Ok(value.to_owned())
}

#[cfg(feature = "memory-sqlite")]
fn ensure_turn_session_visible(
    state: &ControlPlaneHttpState,
    session_id: &str,
) -> Option<Response> {
    let repository_view = state.repository_view.as_ref()?;
    match repository_view.ensure_visible_session_id(session_id) {
        Ok(()) => None,
        Err(error) if error == "control_plane_session_id_missing" => {
            Some(error_response(StatusCode::BAD_REQUEST, error))
        }
        Err(error) if error.starts_with("visibility_denied:") => {
            Some(error_response(StatusCode::FORBIDDEN, error))
        }
        Err(error) => Some(error_response(StatusCode::INTERNAL_SERVER_ERROR, error)),
    }
}

#[cfg(not(feature = "memory-sqlite"))]
fn ensure_turn_session_visible(
    _state: &ControlPlaneHttpState,
    _session_id: &str,
) -> Option<Response> {
    None
}

fn initial_subscribe_state(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    after_seq: u64,
    include_targeted: bool,
) -> ControlPlaneSubscribeStreamState {
    let receiver = manager.subscribe();
    let pending_events = manager.recent_events_after(
        after_seq,
        CONTROL_PLANE_DEFAULT_EVENT_LIMIT,
        include_targeted,
    );
    let pending_events = VecDeque::from(pending_events);
    ControlPlaneSubscribeStreamState {
        manager,
        pending_events,
        receiver,
        last_seq: after_seq,
        include_targeted,
    }
}

fn sse_event_from_control_plane_record(
    record: mvp::control_plane::ControlPlaneEventRecord,
) -> Result<Event, String> {
    let seq = record.seq;
    let envelope = map_event(record);
    let event_name = envelope.event.as_str();
    let event_id = seq.to_string();
    let event_builder = Event::default();
    let event_builder = event_builder.event(event_name);
    let event_builder = event_builder.id(event_id);
    event_builder
        .json_data(&envelope)
        .map_err(|error| format!("control-plane SSE event encoding failed: {error}"))
}

fn fallback_sse_error_event(message: &str) -> Event {
    let error_message = format!("{{\"error\":\"{message}\"}}");
    let base_event = Event::default();
    let named_event = base_event.event("control.error");
    named_event.data(error_message)
}

async fn next_control_plane_sse_item(
    mut state: ControlPlaneSubscribeStreamState,
) -> Option<(Result<Event, Infallible>, ControlPlaneSubscribeStreamState)> {
    loop {
        let pending_event = state.pending_events.pop_front();
        if let Some(record) = pending_event {
            state.last_seq = record.seq;
            let sse_event_result = sse_event_from_control_plane_record(record);
            let sse_event = match sse_event_result {
                Ok(event) => event,
                Err(error) => fallback_sse_error_event(error.as_str()),
            };
            return Some((Ok(sse_event), state));
        }

        let receive_result = state.receiver.recv().await;
        match receive_result {
            Ok(record) => {
                let include_targeted = state.include_targeted;
                let targeted = record.targeted;
                let already_seen = record.seq <= state.last_seq;
                if (!include_targeted && targeted) || already_seen {
                    continue;
                }
                state.last_seq = record.seq;
                let sse_event_result = sse_event_from_control_plane_record(record);
                let sse_event = match sse_event_result {
                    Ok(event) => event,
                    Err(error) => fallback_sse_error_event(error.as_str()),
                };
                return Some((Ok(sse_event), state));
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                let refill = state.manager.recent_events_after(
                    state.last_seq,
                    CONTROL_PLANE_DEFAULT_EVENT_LIMIT,
                    state.include_targeted,
                );
                state.pending_events = VecDeque::from(refill);
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

fn control_plane_subscribe_stream(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    after_seq: u64,
    include_targeted: bool,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let initial_state = initial_subscribe_state(manager, after_seq, include_targeted);
    stream::unfold(initial_state, next_control_plane_sse_item)
}

fn connection_principal_from_connect(
    request: &ControlPlaneConnectRequest,
    connection_id: String,
    granted_scopes: &std::collections::BTreeSet<ControlPlaneScope>,
) -> mvp::control_plane::ControlPlaneConnectionPrincipal {
    mvp::control_plane::ControlPlaneConnectionPrincipal {
        connection_id,
        client_id: request.client.id.clone(),
        role: request.role.as_str().to_owned(),
        scopes: granted_scopes
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>(),
        device_id: request
            .device
            .as_ref()
            .map(|device| device.device_id.clone()),
    }
}

fn granted_connect_scopes(
    state: &ControlPlaneHttpState,
    request: &ControlPlaneConnectRequest,
) -> std::collections::BTreeSet<ControlPlaneScope> {
    let remote_bootstrap = state.exposure_policy.requires_remote_auth() && request.device.is_none();
    if !remote_bootstrap {
        return request.scopes.clone();
    }

    let allowed_scopes = std::collections::BTreeSet::from(CONTROL_PLANE_REMOTE_BOOTSTRAP_SCOPES);
    let requested_scopes = request.scopes.clone();
    requested_scopes
        .intersection(&allowed_scopes)
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
}

fn extract_connection_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get("x-loong-control-token")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn connection_scoped_capabilities(
    lease: &mvp::control_plane::ControlPlaneConnectionLease,
) -> std::collections::BTreeSet<Capability> {
    let mut capabilities = std::collections::BTreeSet::new();
    for raw_scope in &lease.principal.scopes {
        let Some(scope) = ControlPlaneScope::parse(raw_scope.as_str()) else {
            continue;
        };
        match scope {
            ControlPlaneScope::OperatorRead => {
                capabilities.insert(Capability::ControlRead);
            }
            ControlPlaneScope::OperatorWrite => {
                capabilities.insert(Capability::ControlWrite);
            }
            ControlPlaneScope::OperatorApprovals => {
                capabilities.insert(Capability::ControlApprovals);
            }
            ControlPlaneScope::OperatorPairing => {
                capabilities.insert(Capability::ControlPairing);
            }
            ControlPlaneScope::OperatorAcp => {
                capabilities.insert(Capability::ControlAcp);
            }
            ControlPlaneScope::OperatorAdmin => {
                capabilities.insert(Capability::ControlRead);
                capabilities.insert(Capability::ControlWrite);
                capabilities.insert(Capability::ControlApprovals);
                capabilities.insert(Capability::ControlPairing);
                capabilities.insert(Capability::ControlAcp);
            }
        }
    }
    capabilities
}

fn required_capabilities_for_route(
    resolved: &loong_protocol::ResolvedRoute,
) -> Result<std::collections::BTreeSet<Capability>, String> {
    let mut capabilities = std::collections::BTreeSet::new();
    if let Some(required_capability) = resolved.policy.required_capability.as_deref() {
        let normalized_required = required_capability.replace('.', "_");
        let Some(capability) = Capability::parse(normalized_required.as_str()) else {
            return Err(format!(
                "unsupported control-plane required capability mapping `{required_capability}`"
            ));
        };
        let is_control_plane_capability = matches!(
            capability,
            Capability::ControlRead
                | Capability::ControlWrite
                | Capability::ControlApprovals
                | Capability::ControlPairing
                | Capability::ControlAcp
        );
        if !is_control_plane_capability {
            return Err(format!(
                "unsupported control-plane required capability mapping `{}`",
                capability.as_str()
            ));
        }
        capabilities.insert(capability);
    }
    Ok(capabilities)
}

fn authorize_control_plane_request(
    state: &ControlPlaneHttpState,
    method: &str,
    headers: &HeaderMap,
) -> Result<mvp::control_plane::ControlPlaneConnectionLease, Box<Response>> {
    let router = ProtocolRouter::default();
    let resolved = router.resolve(method).map_err(|error| {
        Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("control plane route resolution failed for `{method}`: {error}"),
        ))
    })?;

    let Some(token) = extract_connection_token(headers) else {
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!("missing control-plane token for `{method}`"),
        )));
    };
    let Some(lease) = state.connection_registry.resolve(&token).map_err(|error| {
        Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("control plane connection lookup failed: {error}"),
        ))
    })?
    else {
        state.kernel_authority.remove_binding(&token);
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!("unknown or expired control-plane token for `{method}`"),
        )));
    };

    if lease.principal.role != "operator" {
        return Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            format!(
                "role `{}` is not allowed to access `{method}`",
                lease.principal.role
            ),
        )));
    }

    let route_capabilities = required_capabilities_for_route(&resolved)
        .map_err(|error| Box::new(error_response(StatusCode::INTERNAL_SERVER_ERROR, error)))?;
    let scoped_capabilities = connection_scoped_capabilities(&lease);
    let missing_capability = route_capabilities
        .iter()
        .find(|capability| !scoped_capabilities.contains(capability))
        .copied();
    if let Some(capability) = missing_capability {
        let reason = format!(
            "missing control-plane capability `{}` for method `{method}`",
            capability.as_str()
        );
        return Err(Box::new(error_response(StatusCode::FORBIDDEN, reason)));
    }

    state
        .kernel_authority
        .authorize(&lease.token, method, &route_capabilities)
        .map_err(|error| Box::new(error_response(StatusCode::FORBIDDEN, error)))?;

    Ok(lease)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn control_plane_device_signature_message(
    request: &ControlPlaneConnectRequest,
    device: &loong_protocol::ControlPlaneDeviceIdentity,
) -> Vec<u8> {
    let scopes = request
        .scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "loong-control-plane-connect-v1\nnonce={}\ndevice_id={}\nclient_id={}\nrole={}\nscopes={}\nsigned_at_ms={}",
        device.nonce,
        device.device_id,
        request.client.id,
        request.role.as_str(),
        scopes,
        device.signed_at_ms
    )
    .into_bytes()
}

fn verify_connect_device_challenge(
    state: &ControlPlaneHttpState,
    request: &ControlPlaneConnectRequest,
) -> Result<(), Box<Response>> {
    let Some(device) = request.device.as_ref() else {
        return Ok(());
    };

    let challenge = state
        .challenge_registry
        .consume(&device.nonce)
        .map_err(|error| Box::new(error_response(StatusCode::INTERNAL_SERVER_ERROR, error)))?
        .ok_or_else(|| {
            Box::new(error_response(
                StatusCode::UNAUTHORIZED,
                format!(
                    "unknown or expired control-plane challenge `{}`",
                    device.nonce
                ),
            ))
        })?;

    let now_ms = current_time_ms();
    if device.signed_at_ms < challenge.issued_at_ms
        || device.signed_at_ms
            > challenge
                .expires_at_ms
                .saturating_add(CONTROL_PLANE_CHALLENGE_MAX_FUTURE_SKEW_MS)
        || device.signed_at_ms > now_ms.saturating_add(CONTROL_PLANE_CHALLENGE_MAX_FUTURE_SKEW_MS)
    {
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!(
                "control-plane device signature timestamp is outside the challenge window for `{}`",
                device.device_id
            ),
        )));
    }

    let public_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(device.public_key.as_bytes())
        .map_err(|error| {
            Box::new(error_response(
                StatusCode::BAD_REQUEST,
                format!("invalid control-plane device public_key encoding: {error}"),
            ))
        })?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(device.signature.as_bytes())
        .map_err(|error| {
            Box::new(error_response(
                StatusCode::BAD_REQUEST,
                format!("invalid control-plane device signature encoding: {error}"),
            ))
        })?;

    let public_key_array: [u8; 32] = public_key_bytes.try_into().map_err(|_error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "control-plane device public_key must decode to 32 bytes",
        ))
    })?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_array).map_err(|error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            format!("invalid control-plane device public_key: {error}"),
        ))
    })?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            format!("invalid control-plane device signature bytes: {error}"),
        ))
    })?;
    let message = control_plane_device_signature_message(request, device);
    verifying_key.verify(&message, &signature).map_err(|error| {
        Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!("control-plane device signature verification failed: {error}"),
        ))
    })
}

fn verify_remote_connect_bootstrap_auth(
    state: &ControlPlaneHttpState,
    request: &ControlPlaneConnectRequest,
) -> Result<(), Box<Response>> {
    let requires_remote_auth = state.exposure_policy.requires_remote_auth();
    if !requires_remote_auth {
        return Ok(());
    }

    let device_present = request.device.is_some();
    if device_present {
        return Ok(());
    }

    let shared_token = state
        .exposure_policy
        .shared_token
        .as_deref()
        .ok_or_else(|| {
            Box::new(connect_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ControlPlaneConnectErrorCode::SharedTokenRequired,
                "remote control-plane posture is missing exposure shared token",
            ))
        })?;

    let presented_token = request
        .auth
        .as_ref()
        .and_then(|auth| auth.token.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(presented_token) = presented_token else {
        return Err(Box::new(connect_error_response(
            StatusCode::UNAUTHORIZED,
            ControlPlaneConnectErrorCode::SharedTokenRequired,
            "remote non-loopback operator connect requires auth.token",
        )));
    };

    let token_matches =
        mvp::crypto::timing_safe_eq(presented_token.as_bytes(), shared_token.as_bytes());
    if !token_matches {
        return Err(Box::new(connect_error_response(
            StatusCode::UNAUTHORIZED,
            ControlPlaneConnectErrorCode::SharedTokenInvalid,
            "remote non-loopback operator connect presented an invalid auth.token",
        )));
    }

    Ok(())
}

fn pairing_required_response(
    request: &mvp::control_plane::ControlPlanePairingRequestRecord,
) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ControlPlaneConnectErrorResponse {
            code: ControlPlaneConnectErrorCode::PairingRequired,
            error: format!(
                "device `{}` requires operator pairing approval before connect can complete",
                request.device_id
            ),
            pairing_request_id: Some(request.pairing_request_id.clone()),
        }),
    )
        .into_response()
}

fn device_token_error_response(
    code: ControlPlaneConnectErrorCode,
    error: impl Into<String>,
) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ControlPlaneConnectErrorResponse {
            code,
            error: error.into(),
            pairing_request_id: None,
        }),
    )
        .into_response()
}

fn connect_error_response(
    status: StatusCode,
    code: ControlPlaneConnectErrorCode,
    error: impl Into<String>,
) -> Response {
    (
        status,
        Json(ControlPlaneConnectErrorResponse {
            code,
            error: error.into(),
            pairing_request_id: None,
        }),
    )
        .into_response()
}

fn error_response(status: StatusCode, error: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": error.into(),
        })),
    )
        .into_response()
}

async fn current_snapshot(state: &ControlPlaneHttpState) -> Result<ControlPlaneSnapshot, String> {
    let mut snapshot = map_snapshot(state.manager.snapshot());
    #[cfg(feature = "memory-sqlite")]
    if let Some(repository_view) = state.repository_view.as_ref() {
        let repository_snapshot = repository_view.snapshot_summary()?;
        snapshot.session_count = repository_snapshot.session_count;
        snapshot.pending_approval_count = repository_snapshot.pending_approval_count;
    }
    #[cfg(feature = "memory-sqlite")]
    if let Some(acp_view) = state.acp_view.as_ref() {
        snapshot.acp_session_count = acp_view.visible_session_count().await?;
    }
    Ok(snapshot)
}

async fn readyz() -> impl IntoResponse {
    StatusCode::OK
}

async fn control_challenge(State(state): State<ControlPlaneHttpState>) -> Response {
    let challenge = state.challenge_registry.issue();
    Json(ControlPlaneChallengeResponse {
        nonce: challenge.nonce,
        issued_at_ms: challenge.issued_at_ms,
        expires_at_ms: challenge.expires_at_ms,
    })
    .into_response()
}

async fn healthz(State(state): State<ControlPlaneHttpState>) -> Response {
    match current_snapshot(&state).await {
        Ok(snapshot) => Json(ControlPlaneSnapshotResponse { snapshot }).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn control_snapshot(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/snapshot", &headers) {
        return *response;
    }
    match current_snapshot(&state).await {
        Ok(snapshot) => Json(ControlPlaneSnapshotResponse { snapshot }).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn control_events(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<EventQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/events", &headers) {
        return *response;
    }
    let limit = query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_EVENT_LIMIT);
    let events = if let Some(after_seq) = query.after_seq {
        let timeout_ms = query.timeout_ms.unwrap_or(CONTROL_PLANE_TICK_INTERVAL_MS);
        state
            .manager
            .wait_for_recent_events(after_seq, limit, query.include_targeted, timeout_ms)
            .await
    } else {
        state.manager.recent_events(limit, query.include_targeted)
    };
    let events = events.into_iter().map(map_event).collect::<Vec<_>>();
    Json(ControlPlaneRecentEventsResponse { events }).into_response()
}

async fn control_subscribe(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<SubscribeQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/subscribe", &headers) {
        return *response;
    }
    let after_seq = query.after_seq.unwrap_or(0);
    let include_targeted = query.include_targeted;
    let manager = state.manager;
    let stream = control_plane_subscribe_stream(manager, after_seq, include_targeted);
    let keep_alive = KeepAlive::new()
        .interval(std::time::Duration::from_millis(
            CONTROL_PLANE_TICK_INTERVAL_MS,
        ))
        .text(CONTROL_PLANE_KEEPALIVE_TEXT);
    let sse = Sse::new(stream).keep_alive(keep_alive);
    sse.into_response()
}

async fn control_ping(headers: HeaderMap, State(state): State<ControlPlaneHttpState>) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "control/ping", &headers) {
        return *response;
    }
    match current_snapshot(&state).await {
        Ok(snapshot) => Json(serde_json::json!({
            "protocol": CONTROL_PLANE_PROTOCOL_VERSION,
            "state_version": snapshot.state_version,
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn control_connect(
    State(state): State<ControlPlaneHttpState>,
    Json(request): Json<ControlPlaneConnectRequest>,
) -> Response {
    if request.max_protocol < CONTROL_PLANE_PROTOCOL_VERSION
        || request.min_protocol > CONTROL_PLANE_PROTOCOL_VERSION
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("protocol mismatch: expected protocol {CONTROL_PLANE_PROTOCOL_VERSION}"),
        );
    }
    if let Err(response) = verify_remote_connect_bootstrap_auth(&state, &request) {
        return *response;
    }
    if let Err(response) = verify_connect_device_challenge(&state, &request) {
        return *response;
    }
    if let Some(device) = request.device.as_ref() {
        let requested_scopes = request
            .scopes
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let device_token = request
            .auth
            .as_ref()
            .and_then(|auth| auth.device_token.as_deref());
        let pairing_decision = match state.pairing_registry.evaluate_connect(
            &device.device_id,
            &request.client.id,
            &device.public_key,
            request.role.as_str(),
            &requested_scopes,
            device_token,
        ) {
            Ok(decision) => decision,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        };
        match pairing_decision {
            mvp::control_plane::ControlPlanePairingConnectDecision::Authorized => {}
            mvp::control_plane::ControlPlanePairingConnectDecision::PairingRequired {
                request: pairing_request,
                created,
            } => {
                if created {
                    let _ = state.manager.record_pairing_requested(serde_json::json!({
                        "pairing_request_id": pairing_request.pairing_request_id,
                        "device_id": pairing_request.device_id,
                        "client_id": pairing_request.client_id,
                        "role": pairing_request.role,
                    }));
                }
                return pairing_required_response(&pairing_request);
            }
            mvp::control_plane::ControlPlanePairingConnectDecision::DeviceTokenRequired => {
                return device_token_error_response(
                    ControlPlaneConnectErrorCode::DeviceTokenRequired,
                    format!(
                        "device `{}` is paired but must present auth.device_token on connect",
                        device.device_id
                    ),
                );
            }
            mvp::control_plane::ControlPlanePairingConnectDecision::DeviceTokenInvalid => {
                return device_token_error_response(
                    ControlPlaneConnectErrorCode::DeviceTokenInvalid,
                    format!(
                        "device `{}` presented an invalid auth.device_token",
                        device.device_id
                    ),
                );
            }
        }
    }

    let connection_id = format!(
        "cp-{:016x}",
        state.connection_counter.fetch_add(1, Ordering::Relaxed) + 1
    );
    let granted_scopes = granted_connect_scopes(&state, &request);
    let principal = principal_from_connect(&request, connection_id.clone(), granted_scopes.clone());
    let lease = state
        .connection_registry
        .issue(connection_principal_from_connect(
            &request,
            connection_id,
            &granted_scopes,
        ));
    let scoped_capabilities = connection_scoped_capabilities(&lease);
    let agent_id = lease.principal.client_id.clone();
    let issue_result =
        state
            .kernel_authority
            .issue_scoped_token(&lease.token, &agent_id, &scoped_capabilities);
    if let Err(error) = issue_result {
        let revoked = state.connection_registry.revoke(&lease.token);
        if revoked {
            state.kernel_authority.remove_binding(&lease.token);
        }
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    let snapshot = match current_snapshot(&state).await {
        Ok(snapshot) => snapshot,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };

    let response = ControlPlaneConnectResponse {
        protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        principal,
        connection_token: lease.token,
        connection_token_expires_at_ms: lease.expires_at_ms,
        snapshot,
        policy: default_policy(),
    };
    (StatusCode::OK, Json(response)).into_response()
}

async fn session_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<SessionListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "session/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "session/list", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "session/list requires runtime control-plane serve --config <path>",
            );
        };
        match repository_view.list_sessions(
            query.include_archived,
            query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT),
        ) {
            Ok(view) => Json(ControlPlaneSessionListResponse {
                current_session_id: view.current_session_id,
                matched_count: view.matched_count,
                returned_count: view.returned_count,
                sessions: view
                    .sessions
                    .into_iter()
                    .map(map_session_summary)
                    .collect::<Vec<_>>(),
            })
            .into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

async fn session_read(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<SessionReadQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "session/read requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "session/read", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "session/read requires runtime control-plane serve --config <path>",
            );
        };
        match repository_view.read_session(
            &query.session_id,
            query
                .recent_event_limit
                .unwrap_or(CONTROL_PLANE_DEFAULT_SESSION_RECENT_LIMIT),
            query.tail_after_id,
            query
                .tail_page_limit
                .unwrap_or(CONTROL_PLANE_DEFAULT_SESSION_TAIL_LIMIT),
        ) {
            Ok(Some(observation)) => Json(ControlPlaneSessionReadResponse {
                current_session_id: repository_view.current_session_id().to_owned(),
                observation: map_session_observation(observation),
            })
            .into_response(),
            Ok(None) => error_response(
                StatusCode::NOT_FOUND,
                format!("session `{}` not found", query.session_id.trim()),
            ),
            Err(error) if error == "control_plane_session_id_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::FORBIDDEN, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

async fn task_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TaskListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "task/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "task/list", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "task/list requires runtime control-plane serve --config <path>",
            );
        };
        let limit = query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT);
        match repository_view.list_background_tasks(query.include_archived, limit) {
            Ok(view) => {
                let tasks = view
                    .tasks
                    .into_iter()
                    .map(map_task_summary)
                    .collect::<Vec<_>>();
                let response = ControlPlaneTaskListResponse {
                    current_session_id: view.current_session_id,
                    matched_count: view.matched_count,
                    returned_count: view.returned_count,
                    tasks,
                };
                Json(response).into_response()
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

async fn task_read(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TaskReadQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "task/read requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "task/read", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "task/read requires runtime control-plane serve --config <path>",
            );
        };
        match repository_view.read_background_task(&query.task_id) {
            Ok(Some(task_view)) => {
                let task = map_task_summary(task_view);
                let response = ControlPlaneTaskReadResponse {
                    current_session_id: repository_view.current_session_id().to_owned(),
                    task,
                };
                Json(response).into_response()
            }
            Ok(None) => error_response(
                StatusCode::NOT_FOUND,
                format!("background task `{}` not found", query.task_id.trim()),
            ),
            Err(error) if error == "control_plane_session_id_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::NOT_FOUND, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

async fn approval_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<ApprovalListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "approval/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "approval/list", &headers) {
            return *response;
        }
        let Some(repository_view) = state.repository_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "approval/list requires runtime control-plane serve --config <path>",
            );
        };
        let status = match query.status.as_deref() {
            Some(raw) => match parse_approval_request_status(raw) {
                Ok(status) => Some(status),
                Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
            },
            None => None,
        };
        match repository_view.list_approvals(
            query.session_id.as_deref(),
            status,
            query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT),
        ) {
            Ok(view) => Json(ControlPlaneApprovalListResponse {
                current_session_id: view.current_session_id,
                matched_count: view.matched_count,
                returned_count: view.returned_count,
                approvals: view
                    .approvals
                    .into_iter()
                    .map(map_approval_summary)
                    .collect::<Vec<_>>(),
            })
            .into_response(),
            Err(error) if error == "control_plane_session_id_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::FORBIDDEN, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

async fn pairing_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<PairingListQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "pairing/list", &headers) {
        return *response;
    }
    let status = match query.status.as_deref() {
        Some(raw) => match parse_pairing_status(raw) {
            Ok(status) => Some(status),
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
        },
        None => None,
    };
    let requests = state.pairing_registry.list_requests(
        status,
        query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT),
    );
    let matched_count = requests.len();
    let returned_count = matched_count;
    Json(ControlPlanePairingListResponse {
        matched_count,
        returned_count,
        requests: requests
            .into_iter()
            .map(map_pairing_request)
            .collect::<Vec<_>>(),
    })
    .into_response()
}

async fn pairing_resolve(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Json(request): Json<ControlPlanePairingResolveRequest>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "pairing/resolve", &headers) {
        return *response;
    }
    match state
        .pairing_registry
        .resolve_request(&request.pairing_request_id, request.approve)
    {
        Ok(Some(record)) => {
            let _ = state.manager.record_pairing_resolved(
                serde_json::json!({
                    "pairing_request_id": record.pairing_request_id,
                    "device_id": record.device_id,
                    "status": record.status.as_str(),
                }),
                false,
            );
            Json(ControlPlanePairingResolveResponse {
                request: map_pairing_request(record.clone()),
                device_token: record.device_token,
            })
            .into_response()
        }
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            format!(
                "pairing request `{}` not found",
                request.pairing_request_id.trim()
            ),
        ),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

async fn acp_session_list(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<AcpSessionListQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "acp/session/list requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "acp/session/list", &headers)
        {
            return *response;
        }
        let Some(acp_view) = state.acp_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp/session/list requires runtime control-plane serve --config <path>",
            );
        };
        match acp_view.list_sessions(query.limit.unwrap_or(CONTROL_PLANE_DEFAULT_LIST_LIMIT)) {
            Ok(view) => Json(ControlPlaneAcpSessionListResponse {
                current_session_id: view.current_session_id,
                matched_count: view.matched_count,
                returned_count: view.returned_count,
                sessions: view
                    .sessions
                    .into_iter()
                    .map(map_acp_session_metadata)
                    .collect::<Vec<_>>(),
            })
            .into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

async fn acp_session_read(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<AcpSessionReadQuery>,
) -> Response {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (state, query);
        error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "acp/session/read requires daemon memory-sqlite support",
        )
    }
    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(response) = authorize_control_plane_request(&state, "acp/session/read", &headers)
        {
            return *response;
        }
        let Some(acp_view) = state.acp_view.as_ref() else {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp/session/read requires runtime control-plane serve --config <path>",
            );
        };
        match acp_view.read_session(&query.session_key).await {
            Ok(Some(view)) => Json(ControlPlaneAcpSessionReadResponse {
                current_session_id: view.current_session_id,
                metadata: map_acp_session_metadata(view.metadata),
                status: map_acp_session_status(view.status),
            })
            .into_response(),
            Ok(None) => error_response(
                StatusCode::NOT_FOUND,
                format!("ACP session `{}` not found", query.session_key.trim()),
            ),
            Err(error) if error == "control_plane_acp_session_key_missing" => {
                error_response(StatusCode::BAD_REQUEST, error)
            }
            Err(error) if error.starts_with("visibility_denied:") => {
                error_response(StatusCode::FORBIDDEN, error)
            }
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }
}

async fn turn_submit(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Json(request): Json<ControlPlaneTurnSubmitRequest>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "turn/submit", &headers) {
        return *response;
    }

    let Some(turn_runtime) = state.turn_runtime.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/submit requires runtime control-plane serve --config <path>",
        );
    };

    if !turn_runtime.acp_enabled() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/submit requires ACP to be enabled (`acp.enabled=true`)",
        );
    }

    let session_id = match normalize_required_text(request.session_id.as_str(), "session_id") {
        Ok(session_id) => session_id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };
    if let Some(response) = ensure_turn_session_visible(&state, session_id.as_str()) {
        return response;
    }
    let input = match require_nonempty_text(request.input.as_str(), "input") {
        Ok(input) => input,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    if let Err(error) = crate::build_acp_dispatch_address(
        session_id.as_str(),
        request.channel_id.as_deref(),
        request.conversation_id.as_deref(),
        request.account_id.as_deref(),
        request.participant_id.as_deref(),
        request.thread_id.as_deref(),
    ) {
        return error_response(StatusCode::BAD_REQUEST, error);
    }

    let turn_snapshot = turn_runtime.submit(state.manager.clone(), session_id, input, request);

    let response = ControlPlaneTurnSubmitResponse {
        turn: map_turn_summary(&turn_snapshot),
    };
    (StatusCode::ACCEPTED, Json(response)).into_response()
}

async fn turn_result(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TurnResultQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "turn/result", &headers) {
        return *response;
    }

    let Some(turn_runtime) = state.turn_runtime.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/result requires runtime control-plane serve --config <path>",
        );
    };

    let turn_id = match normalize_required_text(query.turn_id.as_str(), "turn_id") {
        Ok(turn_id) => turn_id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    let snapshot = match turn_runtime.read_turn(turn_id.as_str()) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            let message = format!("turn `{}` not found", turn_id);
            return error_response(StatusCode::NOT_FOUND, message);
        }
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    if let Some(response) = ensure_turn_session_visible(&state, snapshot.session_id.as_str()) {
        return response;
    }

    Json(map_turn_result(&snapshot)).into_response()
}

async fn turn_stream(
    headers: HeaderMap,
    State(state): State<ControlPlaneHttpState>,
    Query(query): Query<TurnStreamQuery>,
) -> Response {
    if let Err(response) = authorize_control_plane_request(&state, "turn/stream", &headers) {
        return *response;
    }

    let Some(turn_runtime) = state.turn_runtime.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "turn/stream requires runtime control-plane serve --config <path>",
        );
    };

    let turn_id = match normalize_required_text(query.turn_id.as_str(), "turn_id") {
        Ok(turn_id) => turn_id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error),
    };

    let snapshot = match turn_runtime.read_turn(turn_id.as_str()) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            let message = format!("turn `{}` not found", turn_id);
            return error_response(StatusCode::NOT_FOUND, message);
        }
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    if let Some(response) = ensure_turn_session_visible(&state, snapshot.session_id.as_str()) {
        return response;
    }
    if !ControlPlaneTurnRuntime::snapshot_has_streamable_events(&snapshot) {
        return error_response(
            StatusCode::CONFLICT,
            format!("turn `{}` completed without any streamable events", turn_id),
        );
    }

    let after_seq = query.after_seq.unwrap_or(0);
    let stream = match turn_runtime.stream(turn_id, after_seq) {
        Ok(stream) => stream,
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    let keep_alive = KeepAlive::new()
        .interval(std::time::Duration::from_millis(
            CONTROL_PLANE_TICK_INTERVAL_MS,
        ))
        .text(CONTROL_PLANE_KEEPALIVE_TEXT);
    Sse::new(stream).keep_alive(keep_alive).into_response()
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_router_with_runtime(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    turn_runtime: Option<Arc<ControlPlaneTurnRuntime>>,
    pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    exposure_policy: ControlPlaneExposurePolicy,
) -> Result<Router, String> {
    let kernel_authority = Arc::new(ControlPlaneKernelAuthority::new()?);
    let state = ControlPlaneHttpState {
        manager,
        connection_counter: Arc::new(AtomicU64::new(0)),
        connection_registry: Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new()),
        challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
        pairing_registry,
        kernel_authority,
        exposure_policy: Arc::new(exposure_policy),
        repository_view,
        acp_view,
        turn_runtime,
    };

    let router = Router::new()
        .route("/readyz", get(readyz))
        .route("/healthz", get(healthz))
        .route("/control/challenge", get(control_challenge))
        .route("/control/ping", get(control_ping))
        .route("/control/connect", post(control_connect))
        .route("/control/subscribe", get(control_subscribe))
        .route("/control/snapshot", get(control_snapshot))
        .route("/control/events", get(control_events))
        .route("/session/list", get(session_list))
        .route("/session/read", get(session_read))
        .route("/task/list", get(task_list))
        .route("/task/read", get(task_read))
        .route("/turn/submit", post(turn_submit))
        .route("/turn/result", get(turn_result))
        .route("/turn/stream", get(turn_stream))
        .route("/approval/list", get(approval_list))
        .route("/pairing/list", get(pairing_list))
        .route("/pairing/resolve", post(pairing_resolve))
        .route("/acp/session/list", get(acp_session_list))
        .route("/acp/session/read", get(acp_session_read))
        .with_state(state);
    Ok(router)
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_router_with_views(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
) -> Result<Router, String> {
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    let exposure_policy = default_loopback_exposure_policy();
    build_control_plane_router_with_runtime(
        manager,
        repository_view,
        acp_view,
        None,
        pairing_registry,
        exposure_policy,
    )
}

#[cfg(not(feature = "memory-sqlite"))]
fn build_control_plane_router_without_repository(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    exposure_policy: ControlPlaneExposurePolicy,
) -> Result<Router, String> {
    let kernel_authority = Arc::new(ControlPlaneKernelAuthority::new()?);
    let state = ControlPlaneHttpState {
        manager,
        connection_counter: Arc::new(AtomicU64::new(0)),
        connection_registry: Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new()),
        challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
        pairing_registry: Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new()),
        kernel_authority,
        exposure_policy: Arc::new(exposure_policy),
        turn_runtime: None,
    };

    let router = Router::new()
        .route("/readyz", get(readyz))
        .route("/healthz", get(healthz))
        .route("/control/challenge", get(control_challenge))
        .route("/control/ping", get(control_ping))
        .route("/control/connect", post(control_connect))
        .route("/control/subscribe", get(control_subscribe))
        .route("/control/snapshot", get(control_snapshot))
        .route("/control/events", get(control_events))
        .route("/session/list", get(session_list))
        .route("/session/read", get(session_read))
        .route("/task/list", get(task_list))
        .route("/task/read", get(task_read))
        .route("/turn/submit", post(turn_submit))
        .route("/turn/result", get(turn_result))
        .route("/turn/stream", get(turn_stream))
        .route("/approval/list", get(approval_list))
        .route("/pairing/list", get(pairing_list))
        .route("/pairing/resolve", post(pairing_resolve))
        .route("/acp/session/list", get(acp_session_list))
        .route("/acp/session/read", get(acp_session_read))
        .with_state(state);
    Ok(router)
}

pub fn build_control_plane_router(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
) -> Result<Router, String> {
    #[cfg(feature = "memory-sqlite")]
    {
        build_control_plane_router_with_views(manager, None, None)
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let exposure_policy = default_loopback_exposure_policy();
        build_control_plane_router_without_repository(manager, exposure_policy)
    }
}

pub async fn run_control_plane_serve_cli(
    config_path: Option<&str>,
    current_session_id: Option<&str>,
    bind_override: Option<&str>,
    port: u16,
) -> CliResult<()> {
    // The control plane can boot in a config-less/read-only shape, but
    // turn execution and repository-backed views only come online when a
    // concrete config is available to seed runtime state.
    if current_session_id.is_some() && config_path.is_none() {
        return Err("runtime control-plane serve --session requires --config".to_owned());
    }
    let bind_addr = resolve_control_plane_bind_addr(bind_override, port)?;
    let loaded_config = match config_path {
        Some(config_path) => {
            let (resolved_path, config) = mvp::config::load(Some(config_path))?;
            Some((resolved_path, config))
        }
        None => None,
    };
    let exposure_policy = build_control_plane_exposure_policy(
        bind_addr,
        loaded_config.as_ref().map(|(_, config)| config),
    )?;
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    // Turn execution is only enabled when the control plane was launched with a
    // concrete config; config-less mode still exposes read-only/control state
    // routes but cannot synthesize governed chat turns.
    let turn_runtime = match loaded_config.as_ref() {
        Some((resolved_path, config)) => Some(Arc::new(ControlPlaneTurnRuntime::new(
            resolved_path.clone(),
            config.clone(),
        )?)),
        None => None,
    };
    #[cfg(feature = "memory-sqlite")]
    let (repository_view, acp_view) = match loaded_config.as_ref() {
        Some((resolved_path, config)) => {
            let memory_config =
                mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                    &config.memory,
                );
            let session_store_config =
                mvp::session::store::SessionStoreConfig::from(&memory_config);
            let session_id = current_session_id.unwrap_or("default");
            println!(
                "loong control plane session view rooted at `{session_id}` from {}",
                resolved_path.display()
            );
            (
                Some(Arc::new(
                    mvp::control_plane::ControlPlaneRepositoryView::new(
                        session_store_config,
                        config.tools.clone(),
                        session_id,
                    ),
                )),
                Some(Arc::new(mvp::control_plane::ControlPlaneAcpView::new(
                    config.clone(),
                    session_id,
                ))),
            )
        }
        None => (None, None),
    };
    #[cfg(feature = "memory-sqlite")]
    let pairing_registry = match loaded_config.as_ref() {
        Some((_, config)) => {
            let memory_config =
                mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                    &config.memory,
                );
            let session_store_config =
                mvp::session::store::SessionStoreConfig::from(&memory_config);
            Arc::new(
                mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(
                    session_store_config,
                )?,
            )
        }
        None => Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new()),
    };
    #[cfg(not(feature = "memory-sqlite"))]
    let _ = (config_path, current_session_id);

    #[cfg(feature = "memory-sqlite")]
    let router = build_control_plane_router_with_runtime(
        manager,
        repository_view,
        acp_view,
        turn_runtime,
        pairing_registry,
        exposure_policy,
    )?;
    #[cfg(not(feature = "memory-sqlite"))]
    let router = build_control_plane_router_without_repository(manager, exposure_policy)?;
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|error| format!("bind control-plane listener failed: {error}"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("read control-plane local address failed: {error}"))?;

    println!("loong control plane listening on http://{local_addr}");
    axum::serve(listener, router)
        .await
        .map_err(|error| format!("control-plane listener failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use loong_contracts::SecretRef;

    fn build_control_plane_router(manager: Arc<mvp::control_plane::ControlPlaneManager>) -> Router {
        super::build_control_plane_router(manager).expect("router")
    }

    #[cfg(feature = "memory-sqlite")]
    fn build_control_plane_router_with_views(
        manager: Arc<mvp::control_plane::ControlPlaneManager>,
        repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
        acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    ) -> Router {
        super::build_control_plane_router_with_views(manager, repository_view, acp_view)
            .expect("router")
    }

    fn build_control_plane_router_with_turn_runtime(
        manager: Arc<mvp::control_plane::ControlPlaneManager>,
        turn_runtime: Arc<ControlPlaneTurnRuntime>,
    ) -> Router {
        let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
        let exposure_policy = default_loopback_exposure_policy();
        super::build_control_plane_router_with_runtime(
            manager,
            None,
            None,
            Some(turn_runtime),
            pairing_registry,
            exposure_policy,
        )
        .expect("router")
    }

    #[cfg(feature = "memory-sqlite")]
    fn build_control_plane_router_with_turn_runtime_and_views(
        manager: Arc<mvp::control_plane::ControlPlaneManager>,
        repository_view: Arc<mvp::control_plane::ControlPlaneRepositoryView>,
        acp_view: Arc<mvp::control_plane::ControlPlaneAcpView>,
        turn_runtime: Arc<ControlPlaneTurnRuntime>,
    ) -> Router {
        let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
        let exposure_policy = default_loopback_exposure_policy();
        super::build_control_plane_router_with_runtime(
            manager,
            Some(repository_view),
            Some(acp_view),
            Some(turn_runtime),
            pairing_registry,
            exposure_policy,
        )
        .expect("router")
    }

    #[derive(Default)]
    struct TestTurnBackendState {
        sink_calls: std::sync::atomic::AtomicUsize,
    }

    struct TestTurnBackend {
        id: &'static str,
        state: Arc<TestTurnBackendState>,
    }

    impl mvp::acp::AcpRuntimeBackend for TestTurnBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> mvp::acp::AcpBackendMetadata {
            mvp::acp::AcpBackendMetadata::new(
                self.id(),
                [
                    mvp::acp::AcpCapability::SessionLifecycle,
                    mvp::acp::AcpCapability::TurnExecution,
                    mvp::acp::AcpCapability::TurnEventStreaming,
                ],
                "Control-plane turn backend for daemon tests",
            )
        }

        fn ensure_session<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _config: &'life1 mvp::config::LoongConfig,
            request: &'life2 mvp::acp::AcpSessionBootstrap,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = CliResult<mvp::acp::AcpSessionHandle>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                Ok(mvp::acp::AcpSessionHandle {
                    session_key: request.session_key.clone(),
                    backend_id: self.id().to_owned(),
                    runtime_session_name: format!("test-runtime-{}", request.session_key),
                    working_directory: request.working_directory.clone(),
                    backend_session_id: Some(format!("backend-{}", request.session_key)),
                    agent_session_id: Some(format!("agent-{}", request.session_key)),
                    binding: request.binding.clone(),
                })
            })
        }

        fn run_turn<'life0, 'life1, 'life2, 'life3, 'async_trait>(
            &'life0 self,
            _config: &'life1 mvp::config::LoongConfig,
            _session: &'life2 mvp::acp::AcpSessionHandle,
            request: &'life3 mvp::acp::AcpTurnRequest,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = CliResult<mvp::acp::AcpTurnResult>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            'life3: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                Ok(mvp::acp::AcpTurnResult {
                    output_text: format!("echo: {}", request.input),
                    state: mvp::acp::AcpSessionState::Ready,
                    usage: None,
                    events: Vec::new(),
                    stop_reason: Some(mvp::acp::AcpTurnStopReason::Completed),
                })
            })
        }

        fn run_turn_with_sink<'life0, 'life1, 'life2, 'life3, 'life5, 'async_trait>(
            &'life0 self,
            _config: &'life1 mvp::config::LoongConfig,
            _session: &'life2 mvp::acp::AcpSessionHandle,
            request: &'life3 mvp::acp::AcpTurnRequest,
            _abort: Option<mvp::acp::AcpAbortSignal>,
            sink: Option<&'life5 dyn mvp::acp::AcpTurnEventSink>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = CliResult<mvp::acp::AcpTurnResult>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            'life3: 'async_trait,
            'life5: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                if let Some(sink) = sink {
                    sink.on_event(&serde_json::json!({
                        "type": "text",
                        "content": format!("chunk:{}", request.input),
                    }))?;
                    sink.on_event(&serde_json::json!({
                        "type": "done",
                        "stopReason": "completed",
                    }))?;
                }
                self.state
                    .sink_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(mvp::acp::AcpTurnResult {
                    output_text: format!("streamed: {}", request.input),
                    state: mvp::acp::AcpSessionState::Ready,
                    usage: Some(serde_json::json!({
                        "total_tokens": 7
                    })),
                    events: Vec::new(),
                    stop_reason: Some(mvp::acp::AcpTurnStopReason::Completed),
                })
            })
        }

        fn cancel<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _config: &'life1 mvp::config::LoongConfig,
            _session: &'life2 mvp::acp::AcpSessionHandle,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { Ok(()) })
        }

        fn close<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _config: &'life1 mvp::config::LoongConfig,
            _session: &'life2 mvp::acp::AcpSessionHandle,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move { Ok(()) })
        }
    }

    fn turn_runtime_test_config(backend_id: &str) -> mvp::config::LoongConfig {
        let mut config = mvp::config::LoongConfig::default();
        config.acp.enabled = true;
        config.acp.backend = Some(backend_id.to_owned());
        config.audit.mode = mvp::config::AuditMode::InMemory;
        config
    }

    fn seeded_turn_runtime(
        backend_id: &'static str,
        state: Arc<TestTurnBackendState>,
    ) -> Arc<ControlPlaneTurnRuntime> {
        mvp::acp::register_acp_backend(backend_id, {
            move || {
                Box::new(TestTurnBackend {
                    id: backend_id,
                    state: state.clone(),
                })
            }
        })
        .expect("register control-plane turn backend");
        let config = turn_runtime_test_config(backend_id);
        let temp_root = std::env::temp_dir().join(format!(
            "loong-control-plane-turn-runtime-{}-{}",
            backend_id,
            current_time_ms()
        ));
        std::fs::create_dir_all(&temp_root).expect("create control-plane turn runtime temp root");
        let resolved_path = temp_root.join("config.toml");
        mvp::config::write(
            Some(resolved_path.to_str().expect("utf8 config path")),
            &config,
            true,
        )
        .expect("write control-plane turn runtime config");
        let acp_manager = Arc::new(mvp::acp::AcpSessionManager::default());
        Arc::new(ControlPlaneTurnRuntime::with_manager(
            resolved_path,
            config,
            acp_manager,
        ))
    }

    fn remote_control_plane_config(shared_token: &str) -> mvp::config::LoongConfig {
        let mut config = mvp::config::LoongConfig::default();
        config.control_plane.allow_remote = true;
        config.control_plane.shared_token = Some(SecretRef::Inline(shared_token.to_owned()));
        config
    }

    fn non_loopback_bind_addr() -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], 4317))
    }

    fn build_remote_control_plane_router(
        manager: Arc<mvp::control_plane::ControlPlaneManager>,
        shared_token: &str,
    ) -> Router {
        let config = remote_control_plane_config(shared_token);
        let bind_addr = non_loopback_bind_addr();
        let exposure_policy =
            build_control_plane_exposure_policy(bind_addr, Some(&config)).expect("policy");
        let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
        super::build_control_plane_router_with_runtime(
            manager,
            None,
            None,
            None,
            pairing_registry,
            exposure_policy,
        )
        .expect("router")
    }

    async fn connect_token(
        router: &Router,
        scopes: std::collections::BTreeSet<ControlPlaneScope>,
    ) -> String {
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes,
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: None,
        };

        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let connect: ControlPlaneConnectResponse =
            serde_json::from_slice(&body).expect("connect json");
        connect.connection_token
    }

    fn bearer_request(method: &str, uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .method(method)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request")
    }

    async fn issue_challenge(router: &Router) -> ControlPlaneChallengeResponse {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/challenge")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("challenge response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        serde_json::from_slice(&body).expect("challenge json")
    }

    fn signed_device_for_request(
        client_id: &str,
        role: ControlPlaneRole,
        scopes: std::collections::BTreeSet<ControlPlaneScope>,
        challenge: &ControlPlaneChallengeResponse,
    ) -> loong_protocol::ControlPlaneDeviceIdentity {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let device_template = loong_protocol::ControlPlaneDeviceIdentity {
            device_id: "device-1".to_owned(),
            public_key: String::new(),
            signature: String::new(),
            signed_at_ms: challenge.issued_at_ms,
            nonce: challenge.nonce.clone(),
        };
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: client_id.to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role,
            scopes,
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device_template.clone()),
        };
        let message = control_plane_device_signature_message(&request, &device_template);
        let signature = signing_key.sign(&message);
        loong_protocol::ControlPlaneDeviceIdentity {
            device_id: "device-1".to_owned(),
            public_key: base64::engine::general_purpose::STANDARD
                .encode(signing_key.verifying_key().to_bytes()),
            signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
            signed_at_ms: challenge.issued_at_ms,
            nonce: challenge.nonce.clone(),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn isolated_memory_config(test_name: &str) -> mvp::session::store::SessionStoreConfig {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ISOLATED_MEMORY_CONFIG_ID: AtomicU64 = AtomicU64::new(1);
        let nonce = NEXT_ISOLATED_MEMORY_CONFIG_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "loong-control-plane-server-{test_name}-{}-{nonce}",
            std::process::id(),
        ));
        let _ = std::fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(base.join("memory.sqlite3-wal"));
        let _ = std::fs::remove_file(base.join("memory.sqlite3-shm"));
        mvp::session::store::SessionStoreConfig {
            sqlite_path: Some(db_path),
            runtime_config: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn seeded_repository_view(
        test_name: &str,
    ) -> Arc<mvp::control_plane::ControlPlaneRepositoryView> {
        let config = isolated_memory_config(test_name);
        let repo = mvp::session::repository::SessionRepository::new(&config).expect("repository");
        repo.create_session(mvp::session::repository::NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: mvp::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: mvp::session::repository::SessionState::Running,
        })
        .expect("create root session");
        repo.create_session(mvp::session::repository::NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: mvp::session::repository::SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: mvp::session::repository::SessionState::Running,
        })
        .expect("create child session");
        repo.append_event(mvp::session::repository::NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: serde_json::json!({
                "task": "research control plane parity",
                "label": "Child",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 90,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["file.read"],
                    "workspace_root": "/tmp/loong/control-plane/child-session",
                    "kernel_bound": false,
                    "runtime_narrowing": {}
                }
            }),
        })
        .expect("append child event");
        repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
            approval_request_id: "apr-visible".to_owned(),
            session_id: "child-session".to_owned(),
            turn_id: "turn-visible".to_owned(),
            tool_call_id: "call-visible".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: serde_json::json!({
                "tool": "delegate",
            }),
            governance_snapshot_json: serde_json::json!({
                "reason": "governed_tool_requires_approval",
                "rule_id": "approval-visible",
            }),
        })
        .expect("create visible approval");
        repo.upsert_session_tool_policy(mvp::session::repository::NewSessionToolPolicyRecord {
            session_id: "child-session".to_owned(),
            requested_tool_ids: vec!["file.read".to_owned()],
            runtime_narrowing: mvp::tools::runtime_config::ToolRuntimeNarrowing::default(),
        })
        .expect("create visible tool policy");
        repo.create_session(mvp::session::repository::NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: mvp::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: mvp::session::repository::SessionState::Ready,
        })
        .expect("create hidden root");
        repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
            approval_request_id: "apr-hidden".to_owned(),
            session_id: "hidden-root".to_owned(),
            turn_id: "turn-hidden".to_owned(),
            tool_call_id: "call-hidden".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: serde_json::json!({
                "tool": "delegate_async",
            }),
            governance_snapshot_json: serde_json::json!({
                "reason": "governed_tool_requires_approval",
                "rule_id": "approval-hidden",
            }),
        })
        .expect("create hidden approval");

        Arc::new(mvp::control_plane::ControlPlaneRepositoryView::new(
            config,
            mvp::config::ToolConfig::default(),
            "root-session",
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    fn seeded_control_plane_views(
        test_name: &str,
    ) -> (
        Arc<mvp::control_plane::ControlPlaneRepositoryView>,
        Arc<mvp::control_plane::ControlPlaneAcpView>,
    ) {
        let memory_config = isolated_memory_config(test_name);
        let repo =
            mvp::session::repository::SessionRepository::new(&memory_config).expect("repository");
        repo.create_session(mvp::session::repository::NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: mvp::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: mvp::session::repository::SessionState::Running,
        })
        .expect("create root session");
        repo.create_session(mvp::session::repository::NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: mvp::session::repository::SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: mvp::session::repository::SessionState::Running,
        })
        .expect("create child session");
        repo.append_event(mvp::session::repository::NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: serde_json::json!({
                "status": "started",
            }),
        })
        .expect("append child event");
        repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
            approval_request_id: "apr-visible".to_owned(),
            session_id: "child-session".to_owned(),
            turn_id: "turn-visible".to_owned(),
            tool_call_id: "call-visible".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: serde_json::json!({
                "tool": "delegate",
            }),
            governance_snapshot_json: serde_json::json!({
                "reason": "governed_tool_requires_approval",
                "rule_id": "approval-visible",
            }),
        })
        .expect("create visible approval");
        repo.create_session(mvp::session::repository::NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: mvp::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: mvp::session::repository::SessionState::Ready,
        })
        .expect("create hidden root");
        repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
            approval_request_id: "apr-hidden".to_owned(),
            session_id: "hidden-root".to_owned(),
            turn_id: "turn-hidden".to_owned(),
            tool_call_id: "call-hidden".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: serde_json::json!({
                "tool": "delegate_async",
            }),
            governance_snapshot_json: serde_json::json!({
                "reason": "governed_tool_requires_approval",
                "rule_id": "approval-hidden",
            }),
        })
        .expect("create hidden approval");

        let mut config = mvp::config::LoongConfig::default();
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        config.memory.sqlite_path = sqlite_path;
        config.acp.enabled = true;

        let store =
            mvp::acp::AcpSqliteSessionStore::new(Some(config.memory.resolved_sqlite_path()));
        mvp::acp::AcpSessionStore::upsert(
            &store,
            mvp::acp::AcpSessionMetadata {
                session_key: "agent:codex:child-session".to_owned(),
                conversation_id: Some("conversation-visible".to_owned()),
                binding: Some(mvp::acp::AcpSessionBindingScope {
                    route_session_id: "child-session".to_owned(),
                    channel_id: Some("feishu".to_owned()),
                    account_id: Some("lark-prod".to_owned()),
                    conversation_id: Some("oc-visible".to_owned()),
                    participant_id: None,
                    thread_id: Some("thread-visible".to_owned()),
                }),
                activation_origin: Some(mvp::acp::AcpRoutingOrigin::ExplicitRequest),
                backend_id: "acpx".to_owned(),
                runtime_session_name: "runtime-visible".to_owned(),
                working_directory: None,
                backend_session_id: Some("backend-visible".to_owned()),
                agent_session_id: Some("agent-visible".to_owned()),
                mode: Some(mvp::acp::AcpSessionMode::Interactive),
                state: mvp::acp::AcpSessionState::Ready,
                last_activity_ms: 100,
                last_error: None,
            },
        )
        .expect("seed visible ACP session");
        mvp::acp::AcpSessionStore::upsert(
            &store,
            mvp::acp::AcpSessionMetadata {
                session_key: "agent:codex:hidden-root".to_owned(),
                conversation_id: Some("conversation-hidden".to_owned()),
                binding: Some(mvp::acp::AcpSessionBindingScope {
                    route_session_id: "hidden-root".to_owned(),
                    channel_id: Some("telegram".to_owned()),
                    account_id: None,
                    conversation_id: Some("hidden".to_owned()),
                    participant_id: None,
                    thread_id: None,
                }),
                activation_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticDispatch),
                backend_id: "acpx".to_owned(),
                runtime_session_name: "runtime-hidden".to_owned(),
                working_directory: None,
                backend_session_id: Some("backend-hidden".to_owned()),
                agent_session_id: Some("agent-hidden".to_owned()),
                mode: Some(mvp::acp::AcpSessionMode::Review),
                state: mvp::acp::AcpSessionState::Busy,
                last_activity_ms: 200,
                last_error: Some("hidden".to_owned()),
            },
        )
        .expect("seed hidden ACP session");

        (
            Arc::new(mvp::control_plane::ControlPlaneRepositoryView::new(
                memory_config,
                mvp::config::ToolConfig::default(),
                "root-session",
            )),
            Arc::new(mvp::control_plane::ControlPlaneAcpView::new(
                config,
                "root-session",
            )),
        )
    }

    #[test]
    fn default_bind_addr_is_loopback() {
        let addr = default_control_plane_bind_addr(0);
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(addr.port(), 0);
    }

    #[test]
    fn resolve_control_plane_bind_addr_accepts_explicit_override() {
        let bind_addr =
            resolve_control_plane_bind_addr(Some("0.0.0.0:4317"), 0).expect("bind addr");
        assert_eq!(bind_addr, non_loopback_bind_addr());
    }

    #[test]
    fn non_loopback_exposure_requires_explicit_remote_opt_in() {
        let config = mvp::config::LoongConfig::default();
        let error = build_control_plane_exposure_policy(non_loopback_bind_addr(), Some(&config))
            .expect_err("remote bind should require explicit opt-in");
        assert!(error.contains("control_plane.allow_remote=true"));
    }

    #[test]
    fn non_loopback_exposure_requires_shared_token() {
        let mut config = mvp::config::LoongConfig::default();
        config.control_plane.allow_remote = true;
        let error = build_control_plane_exposure_policy(non_loopback_bind_addr(), Some(&config))
            .expect_err("remote bind should require shared token");
        assert!(error.contains("control_plane.shared_token"));
    }

    #[tokio::test]
    async fn readyz_returns_ok() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router(manager);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("readyz response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn control_challenge_returns_nonce_payload() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router(manager);
        let challenge = issue_challenge(&router).await;
        assert!(challenge.nonce.starts_with("cpc-"));
        assert!(challenge.expires_at_ms >= challenge.issued_at_ms);
    }

    #[tokio::test]
    async fn healthz_returns_snapshot_json() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        manager.set_presence_count(2);
        manager.set_session_count(3);
        manager.set_pending_approval_count(1);
        manager.set_acp_session_count(4);
        let router = build_control_plane_router(manager);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("healthz response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let snapshot: ControlPlaneSnapshotResponse =
            serde_json::from_slice(&body).expect("snapshot json");
        assert!(snapshot.snapshot.runtime_ready);
        assert_eq!(snapshot.snapshot.presence_count, 2);
        assert_eq!(snapshot.snapshot.session_count, 3);
        assert_eq!(snapshot.snapshot.pending_approval_count, 1);
        assert_eq!(snapshot.snapshot.acp_session_count, 4);
    }

    #[tokio::test]
    async fn control_connect_returns_protocol_response() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: None,
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let connect: ControlPlaneConnectResponse =
            serde_json::from_slice(&body).expect("connect json");
        assert_eq!(connect.protocol, CONTROL_PLANE_PROTOCOL_VERSION);
        assert_eq!(connect.principal.client_id, "cli");
        assert_eq!(connect.principal.role, ControlPlaneRole::Operator);
        assert!(connect.connection_token.starts_with("cpt-"));
        assert!(connect.connection_token_expires_at_ms > 0);
        assert!(connect.snapshot.runtime_ready);
        assert_eq!(
            connect.policy.tick_interval_ms,
            CONTROL_PLANE_TICK_INTERVAL_MS
        );
    }

    #[tokio::test]
    async fn remote_control_connect_requires_shared_token_for_non_device_operator() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_remote_control_plane_router(manager, "bootstrap-token");
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: None,
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&body).expect("error json");
        assert_eq!(
            error.code,
            ControlPlaneConnectErrorCode::SharedTokenRequired
        );
    }

    #[tokio::test]
    async fn remote_control_connect_rejects_invalid_shared_token() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_remote_control_plane_router(manager, "bootstrap-token");
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: Some(loong_protocol::ControlPlaneAuthClaims {
                token: Some("wrong-token".to_owned()),
                device_token: None,
                bootstrap_token: None,
                password: None,
            }),
            device: None,
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&body).expect("error json");
        assert_eq!(error.code, ControlPlaneConnectErrorCode::SharedTokenInvalid);
    }

    #[tokio::test]
    async fn remote_control_connect_accepts_valid_shared_token() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_remote_control_plane_router(manager, "bootstrap-token");
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: Some(loong_protocol::ControlPlaneAuthClaims {
                token: Some("bootstrap-token".to_owned()),
                device_token: None,
                bootstrap_token: None,
                password: None,
            }),
            device: None,
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let connect: ControlPlaneConnectResponse =
            serde_json::from_slice(&body).expect("connect json");
        assert_eq!(connect.principal.client_id, "cli");
    }

    #[tokio::test]
    async fn remote_control_connect_clamps_bootstrap_scopes_to_safe_subset() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_remote_control_plane_router(manager, "bootstrap-token");
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: std::collections::BTreeSet::from([
                ControlPlaneScope::OperatorRead,
                ControlPlaneScope::OperatorAdmin,
                ControlPlaneScope::OperatorPairing,
            ]),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: Some(loong_protocol::ControlPlaneAuthClaims {
                token: Some("bootstrap-token".to_owned()),
                device_token: None,
                bootstrap_token: None,
                password: None,
            }),
            device: None,
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let connect: ControlPlaneConnectResponse =
            serde_json::from_slice(&body).expect("connect json");
        assert!(
            connect
                .principal
                .scopes
                .contains(&ControlPlaneScope::OperatorRead)
        );
        assert!(
            connect
                .principal
                .scopes
                .contains(&ControlPlaneScope::OperatorPairing)
        );
        assert!(
            !connect
                .principal
                .scopes
                .contains(&ControlPlaneScope::OperatorAdmin)
        );
    }

    #[tokio::test]
    async fn control_connect_rejects_protocol_mismatch() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router(manager);
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION + 1,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION + 1,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: None,
            },
            role: ControlPlaneRole::Operator,
            scopes: std::collections::BTreeSet::new(),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: None,
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn control_connect_requires_pairing_before_signed_device_can_connect() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);
        let challenge = issue_challenge(&router).await;
        let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
        let device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            scopes.clone(),
            &challenge,
        );
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes,
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device),
        };

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("connect response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&body).expect("connect error json");
        assert_eq!(error.code, ControlPlaneConnectErrorCode::PairingRequired);
        assert!(error.pairing_request_id.is_some());
    }

    #[tokio::test]
    async fn control_connect_rejects_reused_device_challenge() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);
        let challenge = issue_challenge(&router).await;
        let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
        let device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            scopes.clone(),
            &challenge,
        );
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes,
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device),
        };

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("first connect response");
        assert_eq!(first.status(), StatusCode::FORBIDDEN);

        let second = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("second connect response");
        assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn pairing_resolve_approves_device_and_connect_accepts_device_token() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);

        let challenge = issue_challenge(&router).await;
        let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
        let device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            scopes.clone(),
            &challenge,
        );
        let pairing_request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: scopes.clone(),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device.clone()),
        };

        let pairing_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&pairing_request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("pairing response");
        assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);
        let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let pairing_error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&pairing_body).expect("pairing error json");
        let pairing_request_id = pairing_error
            .pairing_request_id
            .expect("pairing request id");

        let operator_token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
        )
        .await;
        let resolve_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/pairing/resolve")
                    .method("POST")
                    .header("authorization", format!("Bearer {operator_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&ControlPlanePairingResolveRequest {
                            pairing_request_id,
                            approve: true,
                        })
                        .expect("encode resolve request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("resolve response");
        assert_eq!(resolve_response.status(), StatusCode::OK);
        let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let resolve: ControlPlanePairingResolveResponse =
            serde_json::from_slice(&resolve_body).expect("resolve json");
        let device_token = resolve.device_token.expect("device token");

        let reconnect_challenge = issue_challenge(&router).await;
        let reconnect_device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            scopes.clone(),
            &reconnect_challenge,
        );
        let reconnect_request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes,
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: Some(loong_protocol::ControlPlaneAuthClaims {
                token: None,
                device_token: Some(device_token),
                bootstrap_token: None,
                password: None,
            }),
            device: Some(reconnect_device),
        };

        let reconnect = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&reconnect_request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("reconnect response");
        assert_eq!(reconnect.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn control_connect_requires_repairing_for_scope_upgrade() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);

        let initial_scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
        let challenge = issue_challenge(&router).await;
        let device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            initial_scopes.clone(),
            &challenge,
        );
        let initial_request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: initial_scopes.clone(),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device),
        };

        let pairing_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&initial_request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("pairing response");
        let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let pairing_error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&pairing_body).expect("pairing error json");
        let pairing_request_id = pairing_error
            .pairing_request_id
            .expect("pairing request id");

        let operator_token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
        )
        .await;
        let resolve_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/pairing/resolve")
                    .method("POST")
                    .header("authorization", format!("Bearer {operator_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&ControlPlanePairingResolveRequest {
                            pairing_request_id,
                            approve: true,
                        })
                        .expect("encode resolve request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("resolve response");
        let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let resolve: ControlPlanePairingResolveResponse =
            serde_json::from_slice(&resolve_body).expect("resolve json");
        let device_token = resolve.device_token.expect("device token");

        let upgraded_scopes = std::collections::BTreeSet::from([
            ControlPlaneScope::OperatorRead,
            ControlPlaneScope::OperatorAcp,
        ]);
        let upgrade_challenge = issue_challenge(&router).await;
        let upgrade_device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            upgraded_scopes.clone(),
            &upgrade_challenge,
        );
        let upgrade_request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: upgraded_scopes,
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: Some(loong_protocol::ControlPlaneAuthClaims {
                token: None,
                device_token: Some(device_token),
                bootstrap_token: None,
                password: None,
            }),
            device: Some(upgrade_device),
        };

        let upgrade_response = router
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&upgrade_request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("upgrade response");
        assert_eq!(upgrade_response.status(), StatusCode::FORBIDDEN);
        let upgrade_body = to_bytes(upgrade_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let upgrade_error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&upgrade_body).expect("upgrade error json");
        assert_eq!(
            upgrade_error.code,
            ControlPlaneConnectErrorCode::PairingRequired
        );
        assert!(upgrade_error.pairing_request_id.is_some());
    }

    #[tokio::test]
    async fn pairing_list_surfaces_pending_request_for_unpaired_device() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);

        let challenge = issue_challenge(&router).await;
        let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
        let device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            scopes.clone(),
            &challenge,
        );
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes,
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device),
        };

        let pairing_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("pairing response");
        assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);

        let operator_token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
        )
        .await;
        let list_response = router
            .oneshot(bearer_request(
                "GET",
                "/pairing/list?status=pending&limit=10",
                &operator_token,
            ))
            .await
            .expect("pairing list response");
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = to_bytes(list_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let list: ControlPlanePairingListResponse =
            serde_json::from_slice(&list_body).expect("pairing list json");
        assert_eq!(list.matched_count, 1);
        assert_eq!(list.returned_count, 1);
        assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Pending);
        assert_eq!(list.requests[0].device_id, "device-1");
    }

    #[tokio::test]
    async fn pairing_list_surfaces_approved_request_after_resolution() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);

        let challenge = issue_challenge(&router).await;
        let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
        let device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            scopes.clone(),
            &challenge,
        );
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: scopes.clone(),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device),
        };

        let pairing_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("pairing response");
        assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);
        let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let pairing_error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&pairing_body).expect("pairing error json");
        let pairing_request_id = pairing_error
            .pairing_request_id
            .expect("pairing request id");

        let operator_token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
        )
        .await;
        let resolve_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/pairing/resolve")
                    .method("POST")
                    .header("authorization", format!("Bearer {operator_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&ControlPlanePairingResolveRequest {
                            pairing_request_id: pairing_request_id.clone(),
                            approve: true,
                        })
                        .expect("encode resolve request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("resolve response");
        assert_eq!(resolve_response.status(), StatusCode::OK);
        let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let resolve: ControlPlanePairingResolveResponse =
            serde_json::from_slice(&resolve_body).expect("resolve json");
        assert_eq!(resolve.request.status, ControlPlanePairingStatus::Approved,);
        assert_eq!(resolve.request.pairing_request_id, pairing_request_id);
        assert_eq!(resolve.request.requested_scopes, scopes);
        assert!(resolve.request.resolved_at_ms.is_some());
        assert!(resolve.device_token.is_some());

        let list_response = router
            .oneshot(bearer_request(
                "GET",
                "/pairing/list?status=approved&limit=10",
                &operator_token,
            ))
            .await
            .expect("pairing list response");
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = to_bytes(list_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let list: ControlPlanePairingListResponse =
            serde_json::from_slice(&list_body).expect("pairing list json");
        assert_eq!(list.matched_count, 1);
        assert_eq!(list.returned_count, 1);
        assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Approved);
        assert_eq!(list.requests[0].pairing_request_id, pairing_request_id);
        assert_eq!(list.requests[0].requested_scopes, scopes);
        assert!(list.requests[0].resolved_at_ms.is_some());
    }

    #[tokio::test]
    async fn pairing_list_surfaces_rejected_request_after_resolution() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        let router = build_control_plane_router(manager);

        let challenge = issue_challenge(&router).await;
        let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
        let device = signed_device_for_request(
            "cli",
            ControlPlaneRole::Operator,
            scopes.clone(),
            &challenge,
        );
        let request = ControlPlaneConnectRequest {
            min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
            client: ControlPlaneClientIdentity {
                id: "cli".to_owned(),
                version: "1.0.0".to_owned(),
                mode: "operator_ui".to_owned(),
                platform: "macos".to_owned(),
                display_name: Some("Loong CLI".to_owned()),
            },
            role: ControlPlaneRole::Operator,
            scopes: scopes.clone(),
            caps: std::collections::BTreeSet::new(),
            commands: std::collections::BTreeSet::new(),
            permissions: std::collections::BTreeMap::new(),
            auth: None,
            device: Some(device),
        };

        let pairing_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/control/connect")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("pairing response");
        assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);
        let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let pairing_error: ControlPlaneConnectErrorResponse =
            serde_json::from_slice(&pairing_body).expect("pairing error json");
        let pairing_request_id = pairing_error
            .pairing_request_id
            .expect("pairing request id");

        let operator_token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
        )
        .await;
        let resolve_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/pairing/resolve")
                    .method("POST")
                    .header("authorization", format!("Bearer {operator_token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&ControlPlanePairingResolveRequest {
                            pairing_request_id: pairing_request_id.clone(),
                            approve: false,
                        })
                        .expect("encode resolve request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("resolve response");
        assert_eq!(resolve_response.status(), StatusCode::OK);
        let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let resolve: ControlPlanePairingResolveResponse =
            serde_json::from_slice(&resolve_body).expect("resolve json");
        assert_eq!(resolve.request.status, ControlPlanePairingStatus::Rejected,);
        assert_eq!(resolve.request.pairing_request_id, pairing_request_id);
        assert_eq!(resolve.request.requested_scopes, scopes);
        assert!(resolve.request.resolved_at_ms.is_some());
        assert!(resolve.device_token.is_none());

        let list_response = router
            .oneshot(bearer_request(
                "GET",
                "/pairing/list?status=rejected&limit=10",
                &operator_token,
            ))
            .await
            .expect("pairing list response");
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = to_bytes(list_response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let list: ControlPlanePairingListResponse =
            serde_json::from_slice(&list_body).expect("pairing list json");
        assert_eq!(list.matched_count, 1);
        assert_eq!(list.returned_count, 1);
        assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Rejected);
        assert_eq!(list.requests[0].pairing_request_id, pairing_request_id);
        assert_eq!(list.requests[0].requested_scopes, scopes);
        assert!(list.requests[0].resolved_at_ms.is_some());
    }

    #[tokio::test]
    async fn control_snapshot_returns_snapshot_payload() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        manager.set_session_count(7);
        let router = build_control_plane_router(manager);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request("GET", "/control/snapshot", &token))
            .await
            .expect("snapshot response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let snapshot: ControlPlaneSnapshotResponse =
            serde_json::from_slice(&body).expect("snapshot json");
        assert_eq!(snapshot.snapshot.session_count, 7);
    }

    #[tokio::test]
    async fn control_events_returns_recent_events_with_limit() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));
        let _ = manager.record_session_message(serde_json::json!({ "idx": 3 }), true);
        let router = build_control_plane_router(manager);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request("GET", "/control/events?limit=2", &token))
            .await
            .expect("events response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let events: ControlPlaneRecentEventsResponse =
            serde_json::from_slice(&body).expect("events json");
        assert_eq!(events.events.len(), 2);
        assert_eq!(events.events[0].seq, 1);
        assert_eq!(events.events[1].seq, 2);
        assert_eq!(events.events[0].payload["idx"], 1);
        assert_eq!(events.events[1].payload["idx"], 2);
    }

    #[tokio::test]
    async fn control_events_can_include_targeted_records_when_requested() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let _ = manager.record_session_message(serde_json::json!({ "kind": "broadcast" }), false);
        let _ = manager.record_session_message(serde_json::json!({ "kind": "targeted" }), true);
        let router = build_control_plane_router(manager);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request(
                "GET",
                "/control/events?limit=10&include_targeted=true",
                &token,
            ))
            .await
            .expect("events response");
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let events: ControlPlaneRecentEventsResponse =
            serde_json::from_slice(&body).expect("events json");
        assert_eq!(events.events.len(), 2);
        assert_eq!(events.events[1].payload["kind"], "targeted");
    }

    #[tokio::test]
    async fn control_events_supports_after_seq_long_poll() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let router = build_control_plane_router(manager.clone());
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;

        let request_future = {
            let router = router.clone();
            let token = token.clone();
            tokio::spawn(async move {
                router
                    .oneshot(bearer_request(
                        "GET",
                        "/control/events?after_seq=1&timeout_ms=1000",
                        &token,
                    ))
                    .await
            })
        };

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

        let response = request_future
            .await
            .expect("join")
            .expect("events response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let events: ControlPlaneRecentEventsResponse =
            serde_json::from_slice(&body).expect("events json");
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].payload["idx"], 2);
        assert_eq!(events.events[0].seq, 2);
    }

    #[tokio::test]
    async fn control_events_after_seq_returns_empty_on_timeout() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let router = build_control_plane_router(manager);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;

        let response = router
            .oneshot(bearer_request(
                "GET",
                "/control/events?after_seq=1&timeout_ms=20",
                &token,
            ))
            .await
            .expect("events response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let events: ControlPlaneRecentEventsResponse =
            serde_json::from_slice(&body).expect("events json");
        assert!(events.events.is_empty());
    }

    #[tokio::test]
    async fn control_subscribe_rejects_missing_token() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router(manager);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/subscribe")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("subscribe response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn control_subscribe_returns_sse_content_type() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let router = build_control_plane_router(manager);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request(
                "GET",
                "/control/subscribe?after_seq=0",
                &token,
            ))
            .await
            .expect("subscribe response");
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(content_type.starts_with("text/event-stream"));
    }

    #[tokio::test]
    async fn control_subscribe_stream_yields_backlog_event() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));
        let stream = control_plane_subscribe_stream(manager, 1, true);
        let mut stream = Box::pin(stream);
        let next = stream.next().await.expect("stream item");
        let event = next.expect("event");
        let event_debug = format!("{event:?}");
        assert!(!event_debug.is_empty());
    }

    #[tokio::test]
    async fn control_subscribe_stream_yields_live_event_after_wait() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
        let stream = control_plane_subscribe_stream(manager.clone(), 1, true);
        let mut stream = Box::pin(stream);

        let waiter = tokio::spawn(async move { stream.next().await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

        let next = waiter.await.expect("join").expect("stream item");
        let event = next.expect("event");
        let event_debug = format!("{event:?}");
        assert!(!event_debug.is_empty());
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn control_snapshot_uses_repository_backed_session_counts_when_available() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        manager.set_runtime_ready(true);
        manager.set_session_count(99);
        let (repository_view, acp_view) = seeded_control_plane_views("snapshot-repo");
        let router =
            build_control_plane_router_with_views(manager, Some(repository_view), Some(acp_view));
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request("GET", "/control/snapshot", &token))
            .await
            .expect("snapshot response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let snapshot: ControlPlaneSnapshotResponse =
            serde_json::from_slice(&body).expect("snapshot json");
        assert_eq!(snapshot.snapshot.session_count, 2);
        assert_eq!(snapshot.snapshot.pending_approval_count, 1);
        assert_eq!(snapshot.snapshot.acp_session_count, 1);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn session_list_returns_visible_repository_sessions() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router_with_views(
            manager,
            Some(seeded_repository_view("session-list")),
            None,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request("GET", "/session/list?limit=10", &token))
            .await
            .expect("session list response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let sessions: ControlPlaneSessionListResponse =
            serde_json::from_slice(&body).expect("session list json");
        assert_eq!(sessions.current_session_id, "root-session");
        assert_eq!(sessions.matched_count, 2);
        assert_eq!(sessions.returned_count, 2);
        assert!(
            sessions
                .sessions
                .iter()
                .any(|session| session.session_id == "root-session")
        );
        let child = sessions
            .sessions
            .iter()
            .find(|session| session.session_id == "child-session")
            .expect("child session");
        assert_eq!(child.workflow.workflow_id, "root-session");
        assert_eq!(
            child.workflow.task.as_deref(),
            Some("research control plane parity")
        );
        assert_eq!(child.workflow.phase.as_deref(), Some("execute"));
        assert_eq!(
            child
                .workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .mode,
            "advisory_only"
        );
        assert!(
            !sessions
                .sessions
                .iter()
                .any(|session| session.session_id == "hidden-root")
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn session_read_returns_repository_observation_for_visible_session() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router_with_views(
            manager,
            Some(seeded_repository_view("session-read")),
            None,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request(
                "GET",
                "/session/read?session_id=child-session&recent_event_limit=10",
                &token,
            ))
            .await
            .expect("session read response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let session: ControlPlaneSessionReadResponse =
            serde_json::from_slice(&body).expect("session read json");
        assert_eq!(session.current_session_id, "root-session");
        assert_eq!(session.observation.session.session_id, "child-session");
        assert_eq!(
            session.observation.session.workflow.workflow_id,
            "root-session"
        );
        assert_eq!(
            session.observation.session.workflow.task.as_deref(),
            Some("research control plane parity")
        );
        assert_eq!(
            session.observation.session.workflow.phase.as_deref(),
            Some("execute")
        );
        assert_eq!(
            session
                .observation
                .session
                .workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .execution_surface,
            "delegate.async"
        );
        assert_eq!(session.observation.recent_events.len(), 1);
        assert_eq!(
            session.observation.recent_events[0].event_kind,
            "delegate_started"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn task_list_returns_visible_background_tasks() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router_with_views(
            manager,
            Some(seeded_repository_view("task-list")),
            None,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request("GET", "/task/list?limit=10", &token))
            .await
            .expect("task list response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let tasks: ControlPlaneTaskListResponse =
            serde_json::from_slice(&body).expect("task list json");
        assert_eq!(tasks.current_session_id, "root-session");
        assert_eq!(tasks.matched_count, 1);
        assert_eq!(tasks.returned_count, 1);
        let task = tasks.tasks.first().expect("task summary");
        assert_eq!(task.task_id, "child-session");
        assert_eq!(task.workflow.workflow_id, "root-session");
        assert_eq!(
            task.workflow.task.as_deref(),
            Some("research control plane parity")
        );
        assert_eq!(task.workflow.phase.as_deref(), Some("execute"));
        assert_eq!(
            task.workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .task_id,
            "child-session"
        );
        assert_eq!(task.delegate_mode.as_deref(), Some("async"));
        assert_eq!(task.requested_tool_ids, vec!["file.read".to_owned()]);
        assert_eq!(task.visible_requested_tool_ids, vec!["read".to_owned()]);
        assert_eq!(task.effective_tool_ids, vec!["file.read".to_owned()]);
        assert_eq!(task.visible_effective_tool_ids, vec!["read".to_owned()]);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn task_read_returns_visible_background_task_detail() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router_with_views(
            manager,
            Some(seeded_repository_view("task-read")),
            None,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request(
                "GET",
                "/task/read?task_id=child-session",
                &token,
            ))
            .await
            .expect("task read response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let task: ControlPlaneTaskReadResponse =
            serde_json::from_slice(&body).expect("task read json");
        assert_eq!(task.current_session_id, "root-session");
        assert_eq!(task.task.task_id, "child-session");
        assert_eq!(task.task.workflow.workflow_id, "root-session");
        assert_eq!(
            task.task
                .workflow
                .binding
                .as_ref()
                .expect("workflow binding")
                .worktree
                .as_ref()
                .expect("worktree binding")
                .worktree_id,
            "child-session"
        );
        assert_eq!(task.task.delegate_phase.as_deref(), Some("running"));
        assert_eq!(task.task.approval_request_count, 1);
        assert_eq!(task.task.requested_tool_ids, vec!["file.read".to_owned()]);
        assert_eq!(
            task.task.visible_requested_tool_ids,
            vec!["read".to_owned()]
        );
        assert_eq!(task.task.effective_tool_ids, vec!["file.read".to_owned()]);
        assert_eq!(
            task.task.visible_effective_tool_ids,
            vec!["read".to_owned()]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn task_routes_reject_insufficient_scope() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router_with_views(
            manager,
            Some(seeded_repository_view("task-scope")),
            None,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
        )
        .await;

        let list_response = router
            .clone()
            .oneshot(bearer_request("GET", "/task/list?limit=10", &token))
            .await
            .expect("task list response");
        assert_eq!(list_response.status(), StatusCode::FORBIDDEN);

        let read_response = router
            .oneshot(bearer_request(
                "GET",
                "/task/read?task_id=child-session",
                &token,
            ))
            .await
            .expect("task read response");
        assert_eq!(read_response.status(), StatusCode::FORBIDDEN);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_list_returns_only_visible_requests() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router_with_views(
            manager,
            Some(seeded_repository_view("approval-list")),
            None,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorApprovals]),
        )
        .await;
        let response = router
            .oneshot(bearer_request(
                "GET",
                "/approval/list?status=pending&limit=10",
                &token,
            ))
            .await
            .expect("approval list response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let approvals: ControlPlaneApprovalListResponse =
            serde_json::from_slice(&body).expect("approval list json");
        assert_eq!(approvals.current_session_id, "root-session");
        assert_eq!(approvals.matched_count, 1);
        assert_eq!(approvals.returned_count, 1);
        assert_eq!(approvals.approvals[0].approval_request_id, "apr-visible");
        assert_eq!(
            approvals.approvals[0].status,
            ControlPlaneApprovalRequestStatus::Pending
        );
        assert_eq!(
            approvals.approvals[0].reason.as_deref(),
            Some("governed_tool_requires_approval")
        );
        assert_eq!(
            approvals.approvals[0].visible_tool_name.as_deref(),
            Some("delegate")
        );
        assert_eq!(
            approvals.approvals[0].request_summary.as_ref(),
            Some(&serde_json::json!({
                "tool": "delegate",
                "request": {}
            }))
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn acp_session_list_returns_only_visible_sessions() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let (_repository_view, acp_view) = seeded_control_plane_views("acp-list");
        let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorAcp]),
        )
        .await;
        let response = router
            .oneshot(bearer_request("GET", "/acp/session/list?limit=10", &token))
            .await
            .expect("ACP session list response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let sessions: ControlPlaneAcpSessionListResponse =
            serde_json::from_slice(&body).expect("ACP session list json");
        assert_eq!(sessions.current_session_id, "root-session");
        assert_eq!(sessions.matched_count, 1);
        assert_eq!(sessions.returned_count, 1);
        assert_eq!(
            sessions.sessions[0].session_key,
            "agent:codex:child-session"
        );
        assert_eq!(
            sessions.sessions[0]
                .binding
                .as_ref()
                .expect("binding")
                .route_session_id,
            "child-session"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn acp_session_read_returns_live_status_for_visible_session() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let (_repository_view, acp_view) = seeded_control_plane_views("acp-read");
        let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorAcp]),
        )
        .await;
        let response = router
            .oneshot(bearer_request(
                "GET",
                "/acp/session/read?session_key=agent:codex:child-session",
                &token,
            ))
            .await
            .expect("ACP session read response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        let session: ControlPlaneAcpSessionReadResponse =
            serde_json::from_slice(&body).expect("ACP session read json");
        assert_eq!(session.current_session_id, "root-session");
        assert_eq!(session.metadata.session_key, "agent:codex:child-session");
        assert_eq!(session.status.session_key, "agent:codex:child-session");
        assert_eq!(session.status.state, ControlPlaneAcpSessionState::Ready);
        assert_eq!(
            session.status.mode,
            Some(ControlPlaneAcpSessionMode::Interactive)
        );
        assert!(
            session
                .status
                .last_error
                .as_deref()
                .is_some_and(|error| error.starts_with("status_unavailable:")),
            "expected ACP session read to degrade with status_unavailable when backend is absent"
        );
    }

    #[tokio::test]
    async fn control_snapshot_rejects_missing_token() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router(manager);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/control/snapshot")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("snapshot response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn turn_submit_returns_service_unavailable_without_runtime() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router(manager);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
        )
        .await;
        let request = ControlPlaneTurnSubmitRequest {
            session_id: "session-1".to_owned(),
            input: "hello".to_owned(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            working_directory: None,
            metadata: std::collections::BTreeMap::new(),
        };
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/turn/submit")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode turn submit request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("turn submit response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn turn_submit_rejects_insufficient_scope() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let state = Arc::new(TestTurnBackendState::default());
        let backend_id: &'static str =
            Box::leak(format!("control-plane-turn-scope-{}", current_time_ms()).into_boxed_str());
        let turn_runtime = seeded_turn_runtime(backend_id, state);
        let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let request = ControlPlaneTurnSubmitRequest {
            session_id: "session-1".to_owned(),
            input: "hello".to_owned(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            working_directory: None,
            metadata: std::collections::BTreeMap::new(),
        };
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/turn/submit")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode turn submit request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("turn submit response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn turn_submit_rejects_hidden_session_visibility() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let backend_state = Arc::new(TestTurnBackendState::default());
        let backend_id: &'static str =
            Box::leak(format!("control-plane-turn-hidden-{}", current_time_ms()).into_boxed_str());
        let turn_runtime = seeded_turn_runtime(backend_id, backend_state);
        let (repository_view, acp_view) = seeded_control_plane_views("turn-hidden-session");
        let router = build_control_plane_router_with_turn_runtime_and_views(
            manager,
            repository_view,
            acp_view,
            turn_runtime,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
        )
        .await;
        let request = ControlPlaneTurnSubmitRequest {
            session_id: "hidden-root".to_owned(),
            input: "hello".to_owned(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            working_directory: None,
            metadata: std::collections::BTreeMap::new(),
        };
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/turn/submit")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode turn submit request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("turn submit response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn turn_result_and_stream_reject_hidden_session_visibility() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let backend_state = Arc::new(TestTurnBackendState::default());
        let backend_id: &'static str = Box::leak(
            format!("control-plane-turn-hidden-result-{}", current_time_ms()).into_boxed_str(),
        );
        let turn_runtime = seeded_turn_runtime(backend_id, backend_state);
        let turn_snapshot = turn_runtime.registry.issue_turn("hidden-root");
        let turn_id = turn_snapshot.turn_id.clone();
        let (repository_view, acp_view) = seeded_control_plane_views("turn-hidden-result");
        let result_router = build_control_plane_router_with_turn_runtime_and_views(
            manager,
            repository_view,
            acp_view,
            turn_runtime.clone(),
        );
        let token = connect_token(
            &result_router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let result_response = result_router
            .clone()
            .oneshot(bearer_request(
                "GET",
                format!("/turn/result?turn_id={turn_id}").as_str(),
                &token,
            ))
            .await
            .expect("turn result response");
        assert_eq!(result_response.status(), StatusCode::FORBIDDEN);
        let stream_response = result_router
            .oneshot(bearer_request(
                "GET",
                format!("/turn/stream?turn_id={turn_id}").as_str(),
                &token,
            ))
            .await
            .expect("turn stream response");
        assert_eq!(stream_response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn turn_submit_and_result_fetch_complete_with_streamed_backend() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let state = Arc::new(TestTurnBackendState::default());
        let backend_id: &'static str =
            Box::leak(format!("control-plane-turn-success-{}", current_time_ms()).into_boxed_str());
        let turn_runtime = seeded_turn_runtime(backend_id, state.clone());
        let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
        )
        .await;
        let request = ControlPlaneTurnSubmitRequest {
            session_id: "session-1".to_owned(),
            input: "hello".to_owned(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            working_directory: None,
            metadata: std::collections::BTreeMap::new(),
        };
        let submit_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/turn/submit")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode turn submit request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("turn submit response");
        assert_eq!(submit_response.status(), StatusCode::ACCEPTED);
        let submit_body = to_bytes(submit_response.into_body(), usize::MAX)
            .await
            .expect("submit body");
        let submit: ControlPlaneTurnSubmitResponse =
            serde_json::from_slice(&submit_body).expect("submit json");
        assert_eq!(submit.turn.status, ControlPlaneTurnStatus::Running);

        let turn_id = submit.turn.turn_id.clone();
        let mut final_result = None;
        for _ in 0..20 {
            let result_response = router
                .clone()
                .oneshot(bearer_request(
                    "GET",
                    format!("/turn/result?turn_id={turn_id}").as_str(),
                    &token,
                ))
                .await
                .expect("turn result response");
            assert_eq!(result_response.status(), StatusCode::OK);
            let result_body = to_bytes(result_response.into_body(), usize::MAX)
                .await
                .expect("result body");
            let result: ControlPlaneTurnResultResponse =
                serde_json::from_slice(&result_body).expect("result json");
            if result.turn.status.is_terminal() {
                final_result = Some(result);
                break;
            }
            tokio::task::yield_now().await;
        }

        let final_result = final_result.expect("turn should reach a terminal state");
        assert_eq!(
            final_result.turn.status,
            ControlPlaneTurnStatus::Completed,
            "turn result error: {:?}",
            final_result.error
        );
        assert_eq!(final_result.output_text.as_deref(), Some("streamed: hello"));
        assert_eq!(final_result.stop_reason.as_deref(), Some("completed"));
        assert_eq!(
            final_result
                .usage
                .as_ref()
                .and_then(|usage| usage.get("total_tokens")),
            Some(&serde_json::json!(7))
        );
        assert!(
            final_result.turn.event_count >= 3,
            "expected runtime events plus terminal event"
        );
        assert_eq!(
            state.sink_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn turn_stream_replays_buffered_runtime_and_terminal_events() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let state = Arc::new(TestTurnBackendState::default());
        let backend_id: &'static str =
            Box::leak(format!("control-plane-turn-stream-{}", current_time_ms()).into_boxed_str());
        let turn_runtime = seeded_turn_runtime(backend_id, state);
        let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
        )
        .await;
        let request = ControlPlaneTurnSubmitRequest {
            session_id: "session-stream".to_owned(),
            input: "stream me".to_owned(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            working_directory: None,
            metadata: std::collections::BTreeMap::new(),
        };
        let submit_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/turn/submit")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode turn submit request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("turn submit response");
        let submit_body = to_bytes(submit_response.into_body(), usize::MAX)
            .await
            .expect("submit body");
        let submit: ControlPlaneTurnSubmitResponse =
            serde_json::from_slice(&submit_body).expect("submit json");
        let turn_id = submit.turn.turn_id;

        for _ in 0..20 {
            let result_response = router
                .clone()
                .oneshot(bearer_request(
                    "GET",
                    format!("/turn/result?turn_id={turn_id}").as_str(),
                    &token,
                ))
                .await
                .expect("turn result response");
            let result_body = to_bytes(result_response.into_body(), usize::MAX)
                .await
                .expect("result body");
            let result: ControlPlaneTurnResultResponse =
                serde_json::from_slice(&result_body).expect("result json");
            if result.turn.status.is_terminal() {
                break;
            }
            tokio::task::yield_now().await;
        }

        let stream_response = router
            .oneshot(bearer_request(
                "GET",
                format!("/turn/stream?turn_id={turn_id}").as_str(),
                &token,
            ))
            .await
            .expect("turn stream response");
        assert_eq!(stream_response.status(), StatusCode::OK);
        let stream_body = to_bytes(stream_response.into_body(), usize::MAX)
            .await
            .expect("stream body");
        let stream_text = String::from_utf8(stream_body.to_vec()).expect("utf8 stream body");
        assert!(stream_text.contains("event: turn.event"));
        assert!(stream_text.contains("event: turn.terminal"));
        assert!(stream_text.contains("\"type\":\"text\""));
        assert!(stream_text.contains("chunk:stream me"));
        assert!(stream_text.contains("\"event_type\":\"turn.completed\""));
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_list_rejects_insufficient_scope() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let router = build_control_plane_router_with_views(
            manager,
            Some(seeded_repository_view("approval-list-scope")),
            None,
        );
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request(
                "GET",
                "/approval/list?status=pending&limit=10",
                &token,
            ))
            .await
            .expect("approval list response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn turn_submit_preserves_input_whitespace() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let state = Arc::new(TestTurnBackendState::default());
        let backend_id: &'static str = Box::leak(
            format!("control-plane-turn-whitespace-{}", current_time_ms()).into_boxed_str(),
        );
        let turn_runtime = seeded_turn_runtime(backend_id, state);
        let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
        )
        .await;
        let input = "  hello\n\n```rust\nfn main() {}\n```\n".to_owned();
        let request = ControlPlaneTurnSubmitRequest {
            session_id: "session-whitespace".to_owned(),
            input: input.clone(),
            channel_id: None,
            account_id: None,
            conversation_id: None,
            participant_id: None,
            thread_id: None,
            working_directory: None,
            metadata: std::collections::BTreeMap::new(),
        };
        let submit_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/turn/submit")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&request).expect("encode turn submit request"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("turn submit response");
        assert_eq!(submit_response.status(), StatusCode::ACCEPTED);
        let submit_body = to_bytes(submit_response.into_body(), usize::MAX)
            .await
            .expect("submit body");
        let submit: ControlPlaneTurnSubmitResponse =
            serde_json::from_slice(&submit_body).expect("submit json");
        let turn_id = submit.turn.turn_id;
        let mut final_result = None;
        for _ in 0..20 {
            let result_response = router
                .clone()
                .oneshot(bearer_request(
                    "GET",
                    format!("/turn/result?turn_id={turn_id}").as_str(),
                    &token,
                ))
                .await
                .expect("turn result response");
            let result_body = to_bytes(result_response.into_body(), usize::MAX)
                .await
                .expect("result body");
            let result: ControlPlaneTurnResultResponse =
                serde_json::from_slice(&result_body).expect("result json");
            if result.turn.status.is_terminal() {
                final_result = Some(result);
                break;
            }
            tokio::task::yield_now().await;
        }
        let final_result = final_result.expect("turn should reach a terminal state");
        let expected_output = format!("streamed: {input}");
        assert_eq!(
            final_result.output_text.as_deref(),
            Some(expected_output.as_str()),
            "turn result error: {:?}",
            final_result.error
        );
    }

    #[tokio::test]
    async fn turn_stream_stops_when_retention_prunes_completed_turn() {
        let registry = Arc::new(mvp::control_plane::ControlPlaneTurnRegistry::new());
        let turn = registry.issue_turn("session-pruned");
        let turn_id = turn.turn_id.clone();
        registry
            .complete_success(turn_id.as_str(), "done", Some("completed"), None)
            .expect("complete pruned turn");
        let initial_state =
            initial_turn_stream_state(registry.clone(), turn_id.as_str(), 1).expect("state");
        for index in 0..300 {
            let session_id = format!("session-retained-{index}");
            let output_text = format!("output-{index}");
            let retained_turn = registry.issue_turn(session_id.as_str());
            registry
                .complete_success(
                    retained_turn.turn_id.as_str(),
                    output_text.as_str(),
                    Some("completed"),
                    None,
                )
                .expect("complete retained turn");
        }
        let next_item = next_turn_sse_item(initial_state).await;
        assert!(next_item.is_none());
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn acp_session_list_rejects_insufficient_scope() {
        let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
        let (_repository_view, acp_view) = seeded_control_plane_views("acp-list-scope");
        let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
        let token = connect_token(
            &router,
            std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        )
        .await;
        let response = router
            .oneshot(bearer_request("GET", "/acp/session/list?limit=10", &token))
            .await
            .expect("ACP session list response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
