use super::*;

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
        task_session_id: task.task_session_id,
        owner_session_id: task.owner_session_id,
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
    let visible_tool_name = Some(mvp::tools::legacy_display_tool_name(
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
