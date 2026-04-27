use super::*;

pub(super) fn default_policy() -> ControlPlanePolicy {
    ControlPlanePolicy {
        max_payload_bytes: CONTROL_PLANE_MAX_PAYLOAD_BYTES,
        max_buffered_bytes: CONTROL_PLANE_MAX_BUFFERED_BYTES,
        tick_interval_ms: CONTROL_PLANE_TICK_INTERVAL_MS,
    }
}

pub(super) fn map_snapshot(
    snapshot: mvp::control_plane::ControlPlaneSnapshotSummary,
) -> ControlPlaneSnapshot {
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

pub(super) fn map_event_name(
    kind: mvp::control_plane::ControlPlaneEventKind,
) -> ControlPlaneEventName {
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

pub(super) fn map_event(
    event: mvp::control_plane::ControlPlaneEventRecord,
) -> ControlPlaneEventEnvelope {
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
pub(super) fn map_session_kind(
    kind: mvp::session::repository::SessionKind,
) -> ControlPlaneSessionKind {
    match kind {
        mvp::session::repository::SessionKind::Root => ControlPlaneSessionKind::Root,
        mvp::session::repository::SessionKind::DelegateChild => {
            ControlPlaneSessionKind::DelegateChild
        }
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn map_session_state(
    state: mvp::session::repository::SessionState,
) -> ControlPlaneSessionState {
    match state {
        mvp::session::repository::SessionState::Ready => ControlPlaneSessionState::Ready,
        mvp::session::repository::SessionState::Running => ControlPlaneSessionState::Running,
        mvp::session::repository::SessionState::Completed => ControlPlaneSessionState::Completed,
        mvp::session::repository::SessionState::Failed => ControlPlaneSessionState::Failed,
        mvp::session::repository::SessionState::TimedOut => ControlPlaneSessionState::TimedOut,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn map_session_workflow_continuity(
    continuity: mvp::control_plane::ControlPlaneSessionWorkflowContinuityView,
) -> ControlPlaneSessionWorkflowContinuity {
    ControlPlaneSessionWorkflowContinuity {
        present: continuity.present,
        resolved_identity_present: continuity.resolved_identity_present,
        session_profile_projection_present: continuity.session_profile_projection_present,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn map_session_workflow(
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
pub(super) fn map_session_workflow_binding(
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
pub(super) fn map_session_summary(
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
pub(super) fn map_session_event(
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
pub(super) fn map_session_terminal_outcome(
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
pub(super) fn map_session_observation(
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
pub(super) fn map_task_summary(
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
pub(super) fn map_approval_status(
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
pub(super) fn map_approval_decision(
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
pub(super) fn map_approval_summary(
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
pub(super) fn map_acp_binding_scope(
    binding: mvp::acp::AcpSessionBindingScope,
) -> ControlPlaneAcpBindingScope {
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
pub(super) fn map_acp_routing_origin(
    origin: mvp::acp::AcpRoutingOrigin,
) -> ControlPlaneAcpRoutingOrigin {
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
pub(super) fn map_acp_session_mode(mode: mvp::acp::AcpSessionMode) -> ControlPlaneAcpSessionMode {
    match mode {
        mvp::acp::AcpSessionMode::Interactive => ControlPlaneAcpSessionMode::Interactive,
        mvp::acp::AcpSessionMode::Background => ControlPlaneAcpSessionMode::Background,
        mvp::acp::AcpSessionMode::Review => ControlPlaneAcpSessionMode::Review,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn map_acp_session_state(
    state: mvp::acp::AcpSessionState,
) -> ControlPlaneAcpSessionState {
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
pub(super) fn map_acp_session_metadata(
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
pub(super) fn map_acp_session_status(
    status: mvp::acp::AcpSessionStatus,
) -> ControlPlaneAcpSessionStatus {
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
pub(super) fn parse_approval_request_status(
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

pub(super) fn map_pairing_status(
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

pub(super) fn map_pairing_request(
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

pub(super) fn principal_from_connect(
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

pub(super) fn parse_pairing_status(
    raw: &str,
) -> Result<mvp::control_plane::ControlPlanePairingStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Ok(mvp::control_plane::ControlPlanePairingStatus::Pending),
        "approved" => Ok(mvp::control_plane::ControlPlanePairingStatus::Approved),
        "rejected" => Ok(mvp::control_plane::ControlPlanePairingStatus::Rejected),
        _ => Err(format!("unknown pairing status `{raw}`")),
    }
}

pub(super) fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    Ok(trimmed.to_owned())
}

pub(super) fn require_nonempty_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    Ok(value.to_owned())
}
