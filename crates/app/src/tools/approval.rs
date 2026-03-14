#[cfg(feature = "memory-sqlite")]
use std::{cmp::Ordering, collections::BTreeMap};

use async_trait::async_trait;
use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{json, Value};

#[cfg(feature = "memory-sqlite")]
use super::catalog::{ToolExecutionPlane, ToolGovernanceScope, ToolRiskClass};
#[cfg(feature = "memory-sqlite")]
use crate::config::SessionVisibility;
use crate::config::ToolConfig;
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalDecision, ApprovalRequestRecord, ApprovalRequestStatus, SessionRepository,
};
use crate::KernelContext;

#[cfg(feature = "memory-sqlite")]
const APPROVAL_REQUEST_EVIDENCE_EVENT_LIMIT: usize = 64;
#[cfg(feature = "memory-sqlite")]
const APPROVAL_REQUEST_EVIDENCE_EVENT_BUDGET_PER_RETURNED_REQUEST: usize = 4;
#[cfg(feature = "memory-sqlite")]
const APPROVAL_ATTENTION_FRESH_MAX_SECONDS: i64 = 15 * 60;
#[cfg(feature = "memory-sqlite")]
const APPROVAL_ATTENTION_STALE_MAX_SECONDS: i64 = 4 * 60 * 60;

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalRequestsListRequest {
    session_id: Option<String>,
    status: Option<ApprovalRequestStatus>,
    execution_plane: Option<ToolExecutionPlane>,
    governance_scope: Option<ToolGovernanceScope>,
    risk_class: Option<ToolRiskClass>,
    pending_age_bucket: Option<ApprovalAttentionAgeBucket>,
    tool_name: Option<String>,
    approval_key: Option<String>,
    rule_id: Option<String>,
    decision: Option<ApprovalDecision>,
    replay_result: Option<ApprovalReplayResult>,
    integrity_status: Option<ApprovalExecutionIntegrityStatus>,
    needs_attention: Option<bool>,
    attention_reason: Option<ApprovalAttentionReason>,
    recommended_action: Option<ApprovalRecommendedAction>,
    attention_age_bucket: Option<ApprovalAttentionAgeBucket>,
    escalation_level: Option<ApprovalEscalationLevel>,
    prioritize_pending: bool,
    prioritize_attention: bool,
    limit: usize,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalExecutionIntegrityStatus {
    NotStarted,
    InProgress,
    Complete,
    Incomplete,
}

#[cfg(feature = "memory-sqlite")]
impl ApprovalExecutionIntegrityStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::InProgress => "in_progress",
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalAttentionReason {
    IntegrityGap,
    ExecutionFailed,
}

#[cfg(feature = "memory-sqlite")]
impl ApprovalAttentionReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::IntegrityGap => "integrity_gap",
            Self::ExecutionFailed => "execution_failed",
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalRecommendedAction {
    InspectExecutionEventStream,
    InspectReplayPersistence,
    InspectToolExecutionFailure,
    InspectApprovalIntegrity,
}

#[cfg(feature = "memory-sqlite")]
impl ApprovalRecommendedAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::InspectExecutionEventStream => "inspect_execution_event_stream",
            Self::InspectReplayPersistence => "inspect_replay_persistence",
            Self::InspectToolExecutionFailure => "inspect_tool_execution_failure",
            Self::InspectApprovalIntegrity => "inspect_approval_integrity",
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalAttentionAgeBucket {
    Fresh,
    Stale,
    Overdue,
}

#[cfg(feature = "memory-sqlite")]
impl ApprovalAttentionAgeBucket {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Overdue => "overdue",
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalEscalationLevel {
    Elevated,
    Critical,
}

#[cfg(feature = "memory-sqlite")]
impl ApprovalEscalationLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Elevated => "elevated",
            Self::Critical => "critical",
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalReplayResult {
    NotAttempted,
    InProgress,
    CompletedCleanly,
    CompletedWithAttention,
}

#[cfg(feature = "memory-sqlite")]
impl ApprovalReplayResult {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotAttempted => "not_attempted",
            Self::InProgress => "in_progress",
            Self::CompletedCleanly => "completed_cleanly",
            Self::CompletedWithAttention => "completed_with_attention",
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalRequestResolveRequest {
    approval_request_id: String,
    decision: ApprovalDecision,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
pub(crate) struct ApprovalResolutionRequest {
    pub current_session_id: String,
    pub approval_request_id: String,
    pub decision: ApprovalDecision,
    pub visibility: SessionVisibility,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
pub(crate) struct ApprovalResolutionOutcome {
    pub approval_request: ApprovalRequestRecord,
    pub resumed_tool_output: Option<ToolCoreOutcome>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
pub(crate) trait ApprovalResolutionRuntime: Send + Sync {
    async fn resolve_approval_request(
        &self,
        request: ApprovalResolutionRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ApprovalResolutionOutcome, String>;
}

pub fn execute_approval_tool_with_policies(
    request: ToolCoreRequest,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, current_session_id, config, tool_config);
        return Err(
            "approval tools require sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if !tool_config.sessions.enabled {
            return Err("app_tool_disabled: session tools are disabled by config".to_owned());
        }
        match request.tool_name.as_str() {
            "approval_requests_list" => execute_approval_requests_list(
                request.payload,
                current_session_id,
                config,
                tool_config,
            ),
            "approval_request_status" => execute_approval_request_status(
                request.payload,
                current_session_id,
                config,
                tool_config,
            ),
            "approval_request_resolve" => {
                Err("app_tool_requires_runtime_support: approval_request_resolve".to_owned())
            }
            other => Err(format!(
                "app_tool_not_found: unknown approval tool `{other}`"
            )),
        }
    }
}

pub async fn execute_approval_tool_with_runtime_support(
    request: ToolCoreRequest,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    runtime: Option<&(dyn ApprovalResolutionRuntime + '_)>,
    kernel_ctx: Option<&KernelContext>,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (
            request,
            current_session_id,
            config,
            tool_config,
            runtime,
            kernel_ctx,
        );
        return Err(
            "approval tools require sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        match request.tool_name.as_str() {
            "approval_request_resolve" => {
                let runtime =
                    runtime.ok_or_else(|| "approval_request_runtime_not_configured".to_owned())?;
                execute_approval_request_resolve(
                    request.payload,
                    current_session_id,
                    config,
                    tool_config,
                    runtime,
                    kernel_ctx,
                )
                .await
            }
            _ => execute_approval_tool_with_policies(
                request,
                current_session_id,
                config,
                tool_config,
            ),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn execute_approval_requests_list(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_approval_requests_list_request(&payload, tool_config)?;
    let target_session_ids = match request.session_id.as_deref() {
        Some(session_id) => {
            ensure_visible(
                &repo,
                current_session_id,
                session_id,
                tool_config.sessions.visibility,
            )?;
            vec![session_id.to_owned()]
        }
        None => visible_session_ids(&repo, current_session_id, tool_config.sessions.visibility)?,
    };

    let mut requests = Vec::new();
    for session_id in &target_session_ids {
        requests.extend(repo.list_approval_requests_for_session(session_id, request.status)?);
    }
    requests.sort_by(|left, right| {
        right
            .requested_at
            .cmp(&left.requested_at)
            .then_with(|| left.approval_request_id.cmp(&right.approval_request_id))
    });

    let recent_events_by_session_id =
        approval_request_recent_events_by_session_id(&repo, &requests)?;
    let mut request_summaries = requests
        .into_iter()
        .map(|record| {
            let session_events = recent_events_by_session_id
                .get(&record.session_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let execution_evidence = approval_request_execution_evidence_json_from_events(
                session_events,
                &record.approval_request_id,
            );
            approval_request_summary_json_with_evidence(
                &record,
                execution_evidence.clone(),
                approval_request_execution_integrity_json(&record, &execution_evidence),
            )
        })
        .collect::<Vec<_>>();
    if let Some(integrity_status) = request.integrity_status {
        request_summaries.retain(|item| {
            item.get("execution_integrity")
                .and_then(approval_request_execution_integrity_status_from_json)
                == Some(integrity_status)
        });
    }
    if let Some(decision) = request.decision {
        request_summaries
            .retain(|item| approval_request_decision_from_json(item) == Some(decision));
    }
    if let Some(execution_plane) = request.execution_plane {
        request_summaries.retain(|item| {
            approval_request_execution_plane_from_json(item) == Some(execution_plane)
        });
    }
    if let Some(governance_scope) = request.governance_scope {
        request_summaries.retain(|item| {
            approval_request_governance_scope_from_json(item) == Some(governance_scope)
        });
    }
    if let Some(risk_class) = request.risk_class {
        request_summaries
            .retain(|item| approval_request_risk_class_from_json(item) == Some(risk_class));
    }
    if let Some(tool_name) = request.tool_name.as_deref() {
        request_summaries
            .retain(|item| item.get("tool_name").and_then(Value::as_str) == Some(tool_name));
    }
    if let Some(approval_key) = request.approval_key.as_deref() {
        request_summaries
            .retain(|item| item.get("approval_key").and_then(Value::as_str) == Some(approval_key));
    }
    if let Some(rule_id) = request.rule_id.as_deref() {
        request_summaries
            .retain(|item| item.get("rule_id").and_then(Value::as_str) == Some(rule_id));
    }
    if let Some(pending_age_bucket) = request.pending_age_bucket {
        request_summaries.retain(|item| {
            approval_request_pending_age_bucket_from_json(item) == Some(pending_age_bucket)
        });
    }
    if let Some(replay_result) = request.replay_result {
        request_summaries
            .retain(|item| approval_request_replay_result_from_json(item) == Some(replay_result));
    }
    if let Some(needs_attention) = request.needs_attention {
        request_summaries.retain(|item| {
            item.get("execution_integrity")
                .and_then(|value| value.get("needs_attention"))
                .and_then(Value::as_bool)
                == Some(needs_attention)
        });
    }
    if let Some(attention_reason) = request.attention_reason {
        request_summaries.retain(|item| {
            item.get("execution_integrity")
                .and_then(approval_request_attention_reason_from_json)
                == Some(attention_reason)
        });
    }
    if let Some(recommended_action) = request.recommended_action {
        request_summaries.retain(|item| {
            item.get("execution_integrity")
                .and_then(approval_request_recommended_action_from_json)
                == Some(recommended_action)
        });
    }
    if let Some(attention_age_bucket) = request.attention_age_bucket {
        request_summaries.retain(|item| {
            item.get("execution_integrity")
                .and_then(approval_request_attention_age_bucket_from_json)
                == Some(attention_age_bucket)
        });
    }
    if let Some(escalation_level) = request.escalation_level {
        request_summaries.retain(|item| {
            item.get("execution_integrity")
                .and_then(approval_request_escalation_level_from_json)
                == Some(escalation_level)
        });
    }
    if request.prioritize_pending || request.prioritize_attention {
        request_summaries.sort_by(|left, right| {
            let mut ordering = Ordering::Equal;
            if request.prioritize_pending {
                ordering = approval_request_pending_priority(left)
                    .cmp(&approval_request_pending_priority(right))
                    .then_with(|| {
                        approval_request_pending_age_seconds(right)
                            .cmp(&approval_request_pending_age_seconds(left))
                    });
            }
            if request.prioritize_attention {
                ordering = ordering
                    .then_with(|| {
                        approval_request_escalation_priority(left)
                            .cmp(&approval_request_escalation_priority(right))
                    })
                    .then_with(|| {
                        approval_request_attention_priority(left)
                            .cmp(&approval_request_attention_priority(right))
                    });
            }
            ordering
                .then_with(|| {
                    approval_request_requested_at(right).cmp(&approval_request_requested_at(left))
                })
                .then_with(|| approval_request_id(left).cmp(approval_request_id(right)))
        });
    }
    let pending_summary = approval_request_list_pending_summary_json(&request_summaries);
    let integrity_summary = approval_request_list_integrity_summary_json(&request_summaries);
    let resolution_summary = approval_request_list_resolution_summary_json(&request_summaries);
    let correlation_summary = approval_request_list_correlation_summary_json(&request_summaries);
    let governance_summary = approval_request_list_governance_summary_json(&request_summaries);
    let session_summary = approval_request_list_session_summary_json(&request_summaries);
    let matched_count = request_summaries.len();
    request_summaries.truncate(request.limit);
    let returned_count = request_summaries.len();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "filter": {
                "session_id": request.session_id,
                "status": request.status.map(ApprovalRequestStatus::as_str),
                "execution_plane": request.execution_plane.map(tool_execution_plane_as_str),
                "governance_scope": request.governance_scope.map(tool_governance_scope_as_str),
                "risk_class": request.risk_class.map(tool_risk_class_as_str),
                "pending_age_bucket": request.pending_age_bucket.map(ApprovalAttentionAgeBucket::as_str),
                "tool_name": request.tool_name,
                "approval_key": request.approval_key,
                "rule_id": request.rule_id,
                "decision": request.decision.map(ApprovalDecision::as_str),
                "replay_result": request.replay_result.map(ApprovalReplayResult::as_str),
                "integrity_status": request.integrity_status.map(ApprovalExecutionIntegrityStatus::as_str),
                "needs_attention": request.needs_attention,
                "attention_reason": request.attention_reason.map(ApprovalAttentionReason::as_str),
                "recommended_action": request.recommended_action.map(ApprovalRecommendedAction::as_str),
                "attention_age_bucket": request.attention_age_bucket.map(ApprovalAttentionAgeBucket::as_str),
                "escalation_level": request.escalation_level.map(ApprovalEscalationLevel::as_str),
                "prioritize_pending": request.prioritize_pending,
                "prioritize_attention": request.prioritize_attention,
                "limit": request.limit,
            },
            "visible_session_ids": target_session_ids,
            "matched_count": matched_count,
            "returned_count": returned_count,
            "pending_summary": pending_summary,
            "integrity_summary": integrity_summary,
            "resolution_summary": resolution_summary,
            "correlation_summary": correlation_summary,
            "governance_summary": governance_summary,
            "session_summary": session_summary,
            "requests": request_summaries,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_approval_request_status(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let approval_request_id = required_payload_string(&payload, "approval_request_id")?;
    let repo = SessionRepository::new(config)?;
    let request = repo
        .load_approval_request(&approval_request_id)?
        .ok_or_else(|| format!("approval_request_not_found: `{approval_request_id}`"))?;
    ensure_visible(
        &repo,
        current_session_id,
        &request.session_id,
        tool_config.sessions.visibility,
    )?;
    let execution_evidence = approval_request_execution_evidence_json(&repo, &request)?;
    let execution_integrity =
        approval_request_execution_integrity_json(&request, &execution_evidence);

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "approval_request": approval_request_detail_json_with_evidence(
                &request,
                execution_evidence,
                execution_integrity,
            ),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
async fn execute_approval_request_resolve(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    runtime: &(dyn ApprovalResolutionRuntime + '_),
    kernel_ctx: Option<&KernelContext>,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_approval_request_resolve_request(&payload)?;
    let outcome = runtime
        .resolve_approval_request(
            ApprovalResolutionRequest {
                current_session_id: current_session_id.to_owned(),
                approval_request_id: request.approval_request_id,
                decision: request.decision,
                visibility: tool_config.sessions.visibility,
            },
            kernel_ctx,
        )
        .await?;
    let repo = SessionRepository::new(config)?;
    let execution_evidence =
        approval_request_execution_evidence_json(&repo, &outcome.approval_request)?;
    let execution_integrity =
        approval_request_execution_integrity_json(&outcome.approval_request, &execution_evidence);
    let resolution = approval_request_resolution_json(
        &outcome.approval_request,
        &execution_evidence,
        &execution_integrity,
    );

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "resolution": resolution,
            "approval_request": approval_request_detail_json_with_evidence(
                &outcome.approval_request,
                execution_evidence,
                execution_integrity,
            ),
            "resumed_tool_output": outcome.resumed_tool_output,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_summary_json_with_evidence(
    record: &ApprovalRequestRecord,
    execution_evidence: Value,
    execution_integrity: Value,
) -> Value {
    let resolution =
        approval_request_resolution_json(record, &execution_evidence, &execution_integrity);
    let pending_queue = approval_request_pending_queue_json(record);
    json!({
        "approval_request_id": record.approval_request_id,
        "session_id": record.session_id,
        "turn_id": record.turn_id,
        "tool_call_id": record.tool_call_id,
        "tool_name": record.tool_name,
        "approval_key": record.approval_key,
        "execution_plane": record
            .governance_snapshot_json
            .get("execution_plane")
            .and_then(Value::as_str),
        "governance_scope": record
            .governance_snapshot_json
            .get("governance_scope")
            .and_then(Value::as_str),
        "risk_class": record
            .governance_snapshot_json
            .get("risk_class")
            .and_then(Value::as_str),
        "status": record.status.as_str(),
        "decision": record.decision.map(|decision| decision.as_str()),
        "requested_at": record.requested_at,
        "resolved_at": record.resolved_at,
        "resolved_by_session_id": record.resolved_by_session_id,
        "executed_at": record.executed_at,
        "last_error": record.last_error,
        "reason": record
            .governance_snapshot_json
            .get("reason")
            .and_then(Value::as_str),
        "rule_id": record
            .governance_snapshot_json
            .get("rule_id")
            .and_then(Value::as_str),
        "pending_queue": pending_queue,
        "resolution": resolution,
        "execution_evidence": execution_evidence,
        "execution_integrity": execution_integrity,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_pending_queue_json(record: &ApprovalRequestRecord) -> Value {
    let awaiting_decision = matches!(record.status, ApprovalRequestStatus::Pending);
    let age_seconds = if awaiting_decision {
        Some((approval_unix_ts_now() - record.requested_at).max(0))
    } else {
        None
    };
    let age_bucket = approval_request_attention_age_bucket(age_seconds);

    json!({
        "awaiting_decision": awaiting_decision,
        "age_seconds": age_seconds,
        "age_bucket": age_bucket.map(ApprovalAttentionAgeBucket::as_str),
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_list_pending_summary_json(requests: &[Value]) -> Value {
    let mut pending_count = 0usize;
    let mut counts_by_age_bucket = BTreeMap::from([
        ("fresh".to_owned(), 0usize),
        ("stale".to_owned(), 0usize),
        ("overdue".to_owned(), 0usize),
    ]);
    let mut oldest_pending_requested_at: Option<i64> = None;
    let mut oldest_pending_age_seconds: Option<i64> = None;

    for request in requests {
        let pending_queue = request.get("pending_queue").unwrap_or(&Value::Null);
        if pending_queue
            .get("awaiting_decision")
            .and_then(Value::as_bool)
            != Some(true)
        {
            continue;
        }
        pending_count += 1;
        if let Some(age_bucket) = pending_queue.get("age_bucket").and_then(Value::as_str) {
            *counts_by_age_bucket
                .entry(age_bucket.to_owned())
                .or_default() += 1;
        }
        let requested_at = approval_request_requested_at(request);
        oldest_pending_requested_at = Some(
            oldest_pending_requested_at
                .map(|current| current.min(requested_at))
                .unwrap_or(requested_at),
        );
        if let Some(age_seconds) = pending_queue.get("age_seconds").and_then(Value::as_i64) {
            oldest_pending_age_seconds = Some(
                oldest_pending_age_seconds
                    .map(|current| current.max(age_seconds))
                    .unwrap_or(age_seconds),
            );
        }
    }

    json!({
        "pending_count": pending_count,
        "counts_by_age_bucket": counts_by_age_bucket,
        "oldest_pending_requested_at": oldest_pending_requested_at,
        "oldest_pending_age_seconds": oldest_pending_age_seconds,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_list_integrity_summary_json(requests: &[Value]) -> Value {
    let mut counts_by_status = BTreeMap::from([
        ("not_started".to_owned(), 0usize),
        ("in_progress".to_owned(), 0usize),
        ("complete".to_owned(), 0usize),
        ("incomplete".to_owned(), 0usize),
    ]);
    let mut incomplete_gap_counts = BTreeMap::<String, usize>::new();
    let mut needs_attention_count = 0usize;
    let mut counts_by_reason = BTreeMap::from([
        ("integrity_gap".to_owned(), 0usize),
        ("execution_failed".to_owned(), 0usize),
    ]);
    let mut recommended_action_counts = BTreeMap::from([
        ("inspect_execution_event_stream".to_owned(), 0usize),
        ("inspect_replay_persistence".to_owned(), 0usize),
        ("inspect_tool_execution_failure".to_owned(), 0usize),
        ("inspect_approval_integrity".to_owned(), 0usize),
    ]);
    let mut age_bucket_counts = BTreeMap::from([
        ("fresh".to_owned(), 0usize),
        ("stale".to_owned(), 0usize),
        ("overdue".to_owned(), 0usize),
    ]);
    let mut counts_by_escalation = BTreeMap::from([
        ("elevated".to_owned(), 0usize),
        ("critical".to_owned(), 0usize),
    ]);

    for request in requests {
        let execution_integrity = request.get("execution_integrity").unwrap_or(&Value::Null);
        let status = execution_integrity
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if let Some(count) = counts_by_status.get_mut(status) {
            *count += 1;
        }
        if status == "incomplete" {
            if let Some(gap) = execution_integrity.get("gap").and_then(Value::as_str) {
                *incomplete_gap_counts.entry(gap.to_owned()).or_default() += 1;
            }
        }
        if execution_integrity
            .get("needs_attention")
            .and_then(Value::as_bool)
            == Some(true)
        {
            needs_attention_count += 1;
            if let Some(reason) = execution_integrity
                .get("attention_reason")
                .and_then(Value::as_str)
            {
                *counts_by_reason.entry(reason.to_owned()).or_default() += 1;
            }
            if let Some(action) = execution_integrity
                .get("recommended_action")
                .and_then(Value::as_str)
            {
                *recommended_action_counts
                    .entry(action.to_owned())
                    .or_default() += 1;
            }
            if let Some(age_bucket) = execution_integrity
                .get("attention_age_bucket")
                .and_then(Value::as_str)
            {
                *age_bucket_counts.entry(age_bucket.to_owned()).or_default() += 1;
            }
            if let Some(escalation) = execution_integrity
                .get("escalation_level")
                .and_then(Value::as_str)
            {
                *counts_by_escalation
                    .entry(escalation.to_owned())
                    .or_default() += 1;
            }
        }
    }

    json!({
        "counts_by_status": counts_by_status,
        "incomplete_gap_counts": incomplete_gap_counts,
        "attention_summary": {
            "needs_attention_count": needs_attention_count,
            "counts_by_reason": counts_by_reason,
            "recommended_action_counts": recommended_action_counts,
            "age_bucket_counts": age_bucket_counts,
            "counts_by_escalation": counts_by_escalation,
        },
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_list_resolution_summary_json(requests: &[Value]) -> Value {
    let mut decision_counts = BTreeMap::from([
        ("unresolved".to_owned(), 0usize),
        ("approve_once".to_owned(), 0usize),
        ("approve_always".to_owned(), 0usize),
        ("deny".to_owned(), 0usize),
    ]);
    let mut request_status_counts = BTreeMap::from([
        ("pending".to_owned(), 0usize),
        ("approved".to_owned(), 0usize),
        ("executing".to_owned(), 0usize),
        ("executed".to_owned(), 0usize),
        ("denied".to_owned(), 0usize),
        ("expired".to_owned(), 0usize),
        ("cancelled".to_owned(), 0usize),
    ]);
    let mut replay_result_counts = BTreeMap::from([
        ("not_attempted".to_owned(), 0usize),
        ("in_progress".to_owned(), 0usize),
        ("completed_cleanly".to_owned(), 0usize),
        ("completed_with_attention".to_owned(), 0usize),
    ]);

    for request in requests {
        let resolution = request.get("resolution").unwrap_or(&Value::Null);
        match resolution.get("decision").and_then(Value::as_str) {
            Some(decision) => {
                *decision_counts.entry(decision.to_owned()).or_default() += 1;
            }
            None => {
                *decision_counts.entry("unresolved".to_owned()).or_default() += 1;
            }
        }
        if let Some(request_status) = resolution.get("request_status").and_then(Value::as_str) {
            *request_status_counts
                .entry(request_status.to_owned())
                .or_default() += 1;
        }
        if let Some(replay_result) = resolution.get("replay_result").and_then(Value::as_str) {
            *replay_result_counts
                .entry(replay_result.to_owned())
                .or_default() += 1;
        }
    }

    json!({
        "decision_counts": decision_counts,
        "request_status_counts": request_status_counts,
        "replay_result_counts": replay_result_counts,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_list_correlation_summary_json(requests: &[Value]) -> Value {
    let mut tool_name_counts = BTreeMap::<String, usize>::new();
    let mut approval_key_counts = BTreeMap::<String, usize>::new();
    let mut rule_id_counts = BTreeMap::<String, usize>::new();

    for request in requests {
        if let Some(tool_name) = request.get("tool_name").and_then(Value::as_str) {
            *tool_name_counts.entry(tool_name.to_owned()).or_default() += 1;
        }
        if let Some(approval_key) = request.get("approval_key").and_then(Value::as_str) {
            *approval_key_counts
                .entry(approval_key.to_owned())
                .or_default() += 1;
        }
        if let Some(rule_id) = request.get("rule_id").and_then(Value::as_str) {
            *rule_id_counts.entry(rule_id.to_owned()).or_default() += 1;
        }
    }

    json!({
        "tool_name_counts": tool_name_counts,
        "approval_key_counts": approval_key_counts,
        "rule_id_counts": rule_id_counts,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_list_governance_summary_json(requests: &[Value]) -> Value {
    let mut execution_plane_counts = BTreeMap::from([
        ("Core".to_owned(), 0usize),
        ("App".to_owned(), 0usize),
        ("Orchestration".to_owned(), 0usize),
    ]);
    let mut governance_scope_counts = BTreeMap::from([
        ("Routine".to_owned(), 0usize),
        ("TopologyMutation".to_owned(), 0usize),
    ]);
    let mut risk_class_counts = BTreeMap::from([
        ("Low".to_owned(), 0usize),
        ("Elevated".to_owned(), 0usize),
        ("High".to_owned(), 0usize),
    ]);

    for request in requests {
        if let Some(execution_plane) = request.get("execution_plane").and_then(Value::as_str) {
            *execution_plane_counts
                .entry(execution_plane.to_owned())
                .or_default() += 1;
        }
        if let Some(governance_scope) = request.get("governance_scope").and_then(Value::as_str) {
            *governance_scope_counts
                .entry(governance_scope.to_owned())
                .or_default() += 1;
        }
        if let Some(risk_class) = request.get("risk_class").and_then(Value::as_str) {
            *risk_class_counts.entry(risk_class.to_owned()).or_default() += 1;
        }
    }

    json!({
        "execution_plane_counts": execution_plane_counts,
        "governance_scope_counts": governance_scope_counts,
        "risk_class_counts": risk_class_counts,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_list_session_summary_json(requests: &[Value]) -> Value {
    #[derive(Default)]
    struct SessionSummaryAccumulator {
        request_count: usize,
        pending_count: usize,
        attention_count: usize,
        oldest_pending_requested_at: Option<i64>,
        oldest_pending_age_seconds: Option<i64>,
        oldest_pending_age_bucket: Option<ApprovalAttentionAgeBucket>,
    }

    let mut sessions = BTreeMap::<String, SessionSummaryAccumulator>::new();
    for request in requests {
        let Some(session_id) = request.get("session_id").and_then(Value::as_str) else {
            continue;
        };
        let session = sessions.entry(session_id.to_owned()).or_default();
        session.request_count += 1;

        if request
            .get("execution_integrity")
            .and_then(|value| value.get("needs_attention"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            session.attention_count += 1;
        }

        let pending_queue = request.get("pending_queue").unwrap_or(&Value::Null);
        if pending_queue
            .get("awaiting_decision")
            .and_then(Value::as_bool)
            != Some(true)
        {
            continue;
        }

        session.pending_count += 1;
        let requested_at = approval_request_requested_at(request);
        session.oldest_pending_requested_at = Some(
            session
                .oldest_pending_requested_at
                .map(|current| current.min(requested_at))
                .unwrap_or(requested_at),
        );

        if let Some(age_seconds) = pending_queue.get("age_seconds").and_then(Value::as_i64) {
            let is_older = session
                .oldest_pending_age_seconds
                .map(|current| age_seconds > current)
                .unwrap_or(true);
            if is_older {
                session.oldest_pending_age_seconds = Some(age_seconds);
                session.oldest_pending_age_bucket =
                    approval_request_attention_age_bucket(Some(age_seconds));
            }
        }
    }

    let mut sessions = sessions.into_iter().collect::<Vec<_>>();
    sessions.sort_by(|(left_session_id, left), (right_session_id, right)| {
        approval_request_session_hotspot_priority(left.oldest_pending_age_bucket)
            .cmp(&approval_request_session_hotspot_priority(
                right.oldest_pending_age_bucket,
            ))
            .then_with(|| right.pending_count.cmp(&left.pending_count))
            .then_with(|| right.attention_count.cmp(&left.attention_count))
            .then_with(|| {
                right
                    .oldest_pending_age_seconds
                    .unwrap_or_default()
                    .cmp(&left.oldest_pending_age_seconds.unwrap_or_default())
            })
            .then_with(|| right.request_count.cmp(&left.request_count))
            .then_with(|| left_session_id.cmp(right_session_id))
    });

    let sessions = sessions
        .into_iter()
        .map(|(session_id, session)| {
            json!({
                "session_id": session_id,
                "request_count": session.request_count,
                "pending_count": session.pending_count,
                "attention_count": session.attention_count,
                "oldest_pending_requested_at": session.oldest_pending_requested_at,
                "oldest_pending_age_seconds": session.oldest_pending_age_seconds,
                "oldest_pending_age_bucket": session.oldest_pending_age_bucket.map(ApprovalAttentionAgeBucket::as_str),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "session_count": sessions.len(),
        "sessions": sessions,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_detail_json_with_evidence(
    record: &ApprovalRequestRecord,
    execution_evidence: Value,
    execution_integrity: Value,
) -> Value {
    let resolution =
        approval_request_resolution_json(record, &execution_evidence, &execution_integrity);
    let pending_queue = approval_request_pending_queue_json(record);
    json!({
        "approval_request_id": record.approval_request_id,
        "session_id": record.session_id,
        "turn_id": record.turn_id,
        "tool_call_id": record.tool_call_id,
        "tool_name": record.tool_name,
        "approval_key": record.approval_key,
        "execution_plane": record
            .governance_snapshot_json
            .get("execution_plane")
            .and_then(Value::as_str),
        "governance_scope": record
            .governance_snapshot_json
            .get("governance_scope")
            .and_then(Value::as_str),
        "risk_class": record
            .governance_snapshot_json
            .get("risk_class")
            .and_then(Value::as_str),
        "status": record.status.as_str(),
        "decision": record.decision.map(|decision| decision.as_str()),
        "requested_at": record.requested_at,
        "resolved_at": record.resolved_at,
        "resolved_by_session_id": record.resolved_by_session_id,
        "executed_at": record.executed_at,
        "last_error": record.last_error,
        "request_payload": record.request_payload_json,
        "governance_snapshot": record.governance_snapshot_json,
        "pending_queue": pending_queue,
        "resolution": resolution,
        "execution_evidence": execution_evidence,
        "execution_integrity": execution_integrity,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_resolution_json(
    record: &ApprovalRequestRecord,
    execution_evidence: &Value,
    execution_integrity: &Value,
) -> Value {
    let started_event_kind = execution_evidence
        .get("started_event_kind")
        .and_then(Value::as_str);
    let terminal_event_kind = execution_evidence
        .get("terminal_event_kind")
        .and_then(Value::as_str);
    let replay_attempted = started_event_kind.is_some()
        || terminal_event_kind.is_some()
        || matches!(
            record.status,
            ApprovalRequestStatus::Executing | ApprovalRequestStatus::Executed
        );
    let needs_attention = execution_integrity
        .get("needs_attention")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let replay_result = if !replay_attempted {
        "not_attempted"
    } else if matches!(record.status, ApprovalRequestStatus::Executing)
        || (started_event_kind.is_some() && terminal_event_kind.is_none())
    {
        "in_progress"
    } else if needs_attention {
        "completed_with_attention"
    } else {
        "completed_cleanly"
    };

    json!({
        "decision": record.decision.map(|decision| decision.as_str()),
        "request_status": record.status.as_str(),
        "replay_attempted": replay_attempted,
        "replay_result": replay_result,
        "integrity_status": execution_integrity.get("status").cloned().unwrap_or(Value::Null),
        "needs_attention": needs_attention,
        "attention_reason": execution_integrity
            .get("attention_reason")
            .cloned()
            .unwrap_or(Value::Null),
        "recommended_action": execution_integrity
            .get("recommended_action")
            .cloned()
            .unwrap_or(Value::Null),
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_execution_evidence_json(
    repo: &SessionRepository,
    record: &ApprovalRequestRecord,
) -> Result<Value, String> {
    let events =
        repo.list_recent_events(&record.session_id, APPROVAL_REQUEST_EVIDENCE_EVENT_LIMIT)?;
    Ok(approval_request_execution_evidence_json_from_events(
        &events,
        &record.approval_request_id,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_execution_integrity_json(
    record: &ApprovalRequestRecord,
    execution_evidence: &Value,
) -> Value {
    let started_event_kind = execution_evidence
        .get("started_event_kind")
        .and_then(Value::as_str);
    let terminal_event_kind = execution_evidence
        .get("terminal_event_kind")
        .and_then(Value::as_str);
    let replay_decision_persisted = execution_evidence
        .get("replay_decision_persisted")
        .and_then(Value::as_bool);
    let replay_outcome_persisted = execution_evidence
        .get("replay_outcome_persisted")
        .and_then(Value::as_bool);
    let replay_decision_persist_error = execution_evidence
        .get("replay_decision_persist_error")
        .cloned()
        .unwrap_or(Value::Null);
    let replay_outcome_persist_error = execution_evidence
        .get("replay_outcome_persist_error")
        .cloned()
        .unwrap_or(Value::Null);
    let execution_error = execution_evidence
        .get("execution_error")
        .cloned()
        .unwrap_or(Value::Null);
    let evidence_complete = execution_evidence
        .get("evidence_complete")
        .cloned()
        .unwrap_or(Value::Null);

    let (status, gap, integrity_error) = if started_event_kind.is_none()
        && terminal_event_kind.is_none()
    {
        match record.status {
            ApprovalRequestStatus::Pending
            | ApprovalRequestStatus::Approved
            | ApprovalRequestStatus::Denied
            | ApprovalRequestStatus::Expired
            | ApprovalRequestStatus::Cancelled => (
                ApprovalExecutionIntegrityStatus::NotStarted,
                Value::Null,
                Value::Null,
            ),
            ApprovalRequestStatus::Executing => (
                ApprovalExecutionIntegrityStatus::Incomplete,
                Value::String("started_event_missing".to_owned()),
                first_non_null_json_value(
                    &[
                        &replay_decision_persist_error,
                        &replay_outcome_persist_error,
                    ],
                    Some("approval_request_missing_execution_started_event"),
                ),
            ),
            ApprovalRequestStatus::Executed => (
                ApprovalExecutionIntegrityStatus::Incomplete,
                Value::String("execution_events_missing".to_owned()),
                first_non_null_json_value(
                    &[
                        &replay_decision_persist_error,
                        &replay_outcome_persist_error,
                    ],
                    Some("approval_request_missing_execution_events"),
                ),
            ),
        }
    } else if started_event_kind.is_none() && terminal_event_kind.is_some() {
        (
            ApprovalExecutionIntegrityStatus::Incomplete,
            Value::String("started_event_missing".to_owned()),
            first_non_null_json_value(
                &[
                    &replay_decision_persist_error,
                    &replay_outcome_persist_error,
                ],
                Some("approval_request_missing_execution_started_event"),
            ),
        )
    } else if started_event_kind.is_some() && terminal_event_kind.is_none() {
        match record.status {
            ApprovalRequestStatus::Executing => (
                ApprovalExecutionIntegrityStatus::InProgress,
                Value::Null,
                Value::Null,
            ),
            _ => (
                ApprovalExecutionIntegrityStatus::Incomplete,
                Value::String("terminal_event_missing".to_owned()),
                first_non_null_json_value(
                    &[
                        &replay_decision_persist_error,
                        &replay_outcome_persist_error,
                    ],
                    Some("approval_request_missing_execution_terminal_event"),
                ),
            ),
        }
    } else if replay_decision_persisted == Some(true) && replay_outcome_persisted == Some(true) {
        (
            ApprovalExecutionIntegrityStatus::Complete,
            Value::Null,
            Value::Null,
        )
    } else if replay_decision_persisted != Some(true) && replay_outcome_persisted != Some(true) {
        (
            ApprovalExecutionIntegrityStatus::Incomplete,
            Value::String("replay_decision_and_outcome_missing".to_owned()),
            first_non_null_json_value(
                &[
                    &replay_decision_persist_error,
                    &replay_outcome_persist_error,
                ],
                record.last_error.as_deref(),
            ),
        )
    } else if replay_decision_persisted != Some(true) {
        (
            ApprovalExecutionIntegrityStatus::Incomplete,
            Value::String("replay_decision_missing".to_owned()),
            first_non_null_json_value(
                &[
                    &replay_decision_persist_error,
                    &replay_outcome_persist_error,
                ],
                record.last_error.as_deref(),
            ),
        )
    } else {
        (
            ApprovalExecutionIntegrityStatus::Incomplete,
            Value::String("replay_outcome_missing".to_owned()),
            first_non_null_json_value(
                &[
                    &replay_outcome_persist_error,
                    &replay_decision_persist_error,
                ],
                record.last_error.as_deref(),
            ),
        )
    };

    let (needs_attention, attention_reason) =
        if matches!(status, ApprovalExecutionIntegrityStatus::Incomplete) {
            (true, Value::String("integrity_gap".to_owned()))
        } else if matches!(status, ApprovalExecutionIntegrityStatus::Complete)
            && !execution_error.is_null()
        {
            (true, Value::String("execution_failed".to_owned()))
        } else {
            (false, Value::Null)
        };
    let recommended_action =
        approval_request_recommended_action(status, gap.as_str(), execution_error.is_null());
    let attention_age_seconds = if needs_attention {
        Some((approval_unix_ts_now() - record.requested_at).max(0))
    } else {
        None
    };
    let attention_age_bucket = approval_request_attention_age_bucket(attention_age_seconds);
    let escalation_level =
        approval_request_escalation_level(attention_age_bucket, recommended_action);

    json!({
        "status": status.as_str(),
        "gap": gap,
        "integrity_error": integrity_error,
        "execution_error": execution_error,
        "evidence_complete": evidence_complete,
        "needs_attention": needs_attention,
        "attention_reason": attention_reason,
        "recommended_action": recommended_action.map(ApprovalRecommendedAction::as_str),
        "attention_age_seconds": attention_age_seconds,
        "attention_age_bucket": attention_age_bucket.map(ApprovalAttentionAgeBucket::as_str),
        "escalation_level": escalation_level.map(ApprovalEscalationLevel::as_str),
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_recent_events_by_session_id(
    repo: &SessionRepository,
    requests: &[ApprovalRequestRecord],
) -> Result<BTreeMap<String, Vec<crate::session::repository::SessionEventRecord>>, String> {
    let mut request_count_by_session_id = BTreeMap::<String, usize>::new();
    for record in requests {
        *request_count_by_session_id
            .entry(record.session_id.clone())
            .or_default() += 1;
    }

    let mut events_by_session_id = BTreeMap::new();
    for (session_id, request_count) in request_count_by_session_id {
        let event_limit = APPROVAL_REQUEST_EVIDENCE_EVENT_LIMIT.max(
            request_count
                .saturating_mul(APPROVAL_REQUEST_EVIDENCE_EVENT_BUDGET_PER_RETURNED_REQUEST),
        );
        let events = repo.list_recent_events(&session_id, event_limit)?;
        events_by_session_id.insert(session_id, events);
    }
    Ok(events_by_session_id)
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_execution_evidence_json_from_events(
    events: &[crate::session::repository::SessionEventRecord],
    approval_request_id: &str,
) -> Value {
    let mut started_event: Option<&crate::session::repository::SessionEventRecord> = None;
    let mut terminal_event: Option<&crate::session::repository::SessionEventRecord> = None;

    for event in events {
        if event
            .payload_json
            .get("approval_request_id")
            .and_then(Value::as_str)
            != Some(approval_request_id)
        {
            continue;
        }
        match event.event_kind.as_str() {
            "tool_approval_execution_started" => started_event = Some(event),
            "tool_approval_execution_finished" | "tool_approval_execution_failed" => {
                terminal_event = Some(event)
            }
            _ => {}
        }
    }

    let replay_decision_persisted = started_event
        .and_then(|event| event.payload_json.get("replay_decision_persisted"))
        .and_then(Value::as_bool);
    let replay_decision_persist_error = started_event
        .and_then(|event| event.payload_json.get("replay_decision_persist_error"))
        .cloned()
        .unwrap_or(Value::Null);
    let replay_outcome_persisted = terminal_event
        .and_then(|event| event.payload_json.get("replay_outcome_persisted"))
        .and_then(Value::as_bool);
    let replay_outcome_persist_error = terminal_event
        .and_then(|event| event.payload_json.get("replay_outcome_persist_error"))
        .cloned()
        .unwrap_or(Value::Null);
    let execution_error = terminal_event
        .and_then(|event| event.payload_json.get("error"))
        .cloned()
        .unwrap_or(Value::Null);
    let evidence_complete = terminal_event
        .map(|_| {
            Value::Bool(
                replay_decision_persisted.unwrap_or(false)
                    && replay_outcome_persisted.unwrap_or(false),
            )
        })
        .unwrap_or(Value::Null);

    json!({
        "started_event_kind": started_event.map(|event| event.event_kind.clone()),
        "terminal_event_kind": terminal_event.map(|event| event.event_kind.clone()),
        "replay_decision_persisted": replay_decision_persisted,
        "replay_decision_persist_error": replay_decision_persist_error,
        "replay_outcome_persisted": replay_outcome_persisted,
        "replay_outcome_persist_error": replay_outcome_persist_error,
        "execution_error": execution_error,
        "evidence_complete": evidence_complete,
    })
}

#[cfg(feature = "memory-sqlite")]
fn first_non_null_json_value(values: &[&Value], fallback: Option<&str>) -> Value {
    values
        .iter()
        .find(|value| !value.is_null())
        .map(|value| (*value).clone())
        .or_else(|| fallback.map(|value| Value::String(value.to_owned())))
        .unwrap_or(Value::Null)
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_execution_integrity_status_from_json(
    execution_integrity: &Value,
) -> Option<ApprovalExecutionIntegrityStatus> {
    execution_integrity
        .get("status")
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_execution_integrity_status(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_execution_plane_from_json(request: &Value) -> Option<ToolExecutionPlane> {
    request
        .get("execution_plane")
        .and_then(Value::as_str)
        .and_then(|value| parse_tool_execution_plane(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_governance_scope_from_json(request: &Value) -> Option<ToolGovernanceScope> {
    request
        .get("governance_scope")
        .and_then(Value::as_str)
        .and_then(|value| parse_tool_governance_scope(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_risk_class_from_json(request: &Value) -> Option<ToolRiskClass> {
    request
        .get("risk_class")
        .and_then(Value::as_str)
        .and_then(|value| parse_tool_risk_class(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_decision_from_json(request: &Value) -> Option<ApprovalDecision> {
    request
        .get("resolution")
        .and_then(|value| value.get("decision"))
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_list_decision(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_replay_result_from_json(request: &Value) -> Option<ApprovalReplayResult> {
    request
        .get("resolution")
        .and_then(|value| value.get("replay_result"))
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_replay_result(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_attention_reason_from_json(
    execution_integrity: &Value,
) -> Option<ApprovalAttentionReason> {
    execution_integrity
        .get("attention_reason")
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_attention_reason(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_recommended_action(
    status: ApprovalExecutionIntegrityStatus,
    gap: Option<&str>,
    execution_error_is_null: bool,
) -> Option<ApprovalRecommendedAction> {
    match status {
        ApprovalExecutionIntegrityStatus::Incomplete => Some(match gap {
            Some("started_event_missing")
            | Some("terminal_event_missing")
            | Some("execution_events_missing") => {
                ApprovalRecommendedAction::InspectExecutionEventStream
            }
            Some("replay_decision_and_outcome_missing")
            | Some("replay_decision_missing")
            | Some("replay_outcome_missing") => ApprovalRecommendedAction::InspectReplayPersistence,
            Some(_) | None => ApprovalRecommendedAction::InspectApprovalIntegrity,
        }),
        ApprovalExecutionIntegrityStatus::Complete if !execution_error_is_null => {
            Some(ApprovalRecommendedAction::InspectToolExecutionFailure)
        }
        _ => None,
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_recommended_action_from_json(
    execution_integrity: &Value,
) -> Option<ApprovalRecommendedAction> {
    execution_integrity
        .get("recommended_action")
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_recommended_action(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_attention_age_bucket_from_json(
    execution_integrity: &Value,
) -> Option<ApprovalAttentionAgeBucket> {
    execution_integrity
        .get("attention_age_bucket")
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_attention_age_bucket(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_escalation_level_from_json(
    execution_integrity: &Value,
) -> Option<ApprovalEscalationLevel> {
    execution_integrity
        .get("escalation_level")
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_escalation_level(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_attention_age_bucket(
    attention_age_seconds: Option<i64>,
) -> Option<ApprovalAttentionAgeBucket> {
    let attention_age_seconds = attention_age_seconds?;
    if attention_age_seconds < APPROVAL_ATTENTION_FRESH_MAX_SECONDS {
        Some(ApprovalAttentionAgeBucket::Fresh)
    } else if attention_age_seconds < APPROVAL_ATTENTION_STALE_MAX_SECONDS {
        Some(ApprovalAttentionAgeBucket::Stale)
    } else {
        Some(ApprovalAttentionAgeBucket::Overdue)
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_escalation_level(
    attention_age_bucket: Option<ApprovalAttentionAgeBucket>,
    recommended_action: Option<ApprovalRecommendedAction>,
) -> Option<ApprovalEscalationLevel> {
    let attention_age_bucket = attention_age_bucket?;
    if matches!(attention_age_bucket, ApprovalAttentionAgeBucket::Overdue)
        || matches!(
            recommended_action,
            Some(ApprovalRecommendedAction::InspectExecutionEventStream)
        )
    {
        Some(ApprovalEscalationLevel::Critical)
    } else {
        Some(ApprovalEscalationLevel::Elevated)
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_attention_priority(request: &Value) -> u8 {
    let execution_integrity = request.get("execution_integrity").unwrap_or(&Value::Null);
    if execution_integrity
        .get("needs_attention")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return 3;
    }
    match execution_integrity
        .get("attention_reason")
        .and_then(Value::as_str)
    {
        Some("integrity_gap") => 0,
        Some("execution_failed") => 1,
        Some(_) | None => 2,
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_escalation_priority(request: &Value) -> u8 {
    match request
        .get("execution_integrity")
        .and_then(approval_request_escalation_level_from_json)
    {
        Some(ApprovalEscalationLevel::Critical) => 0,
        Some(ApprovalEscalationLevel::Elevated) => 1,
        None => 2,
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_requested_at(request: &Value) -> i64 {
    request
        .get("requested_at")
        .and_then(Value::as_i64)
        .unwrap_or(i64::MIN)
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_pending_age_seconds(request: &Value) -> i64 {
    request
        .get("pending_queue")
        .and_then(|value| value.get("age_seconds"))
        .and_then(Value::as_i64)
        .unwrap_or_default()
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_pending_age_bucket_from_json(
    request: &Value,
) -> Option<ApprovalAttentionAgeBucket> {
    request
        .get("pending_queue")
        .and_then(|value| value.get("age_bucket"))
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_pending_age_bucket(value).ok())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_pending_priority(request: &Value) -> usize {
    let pending_queue = request.get("pending_queue").unwrap_or(&Value::Null);
    if pending_queue
        .get("awaiting_decision")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return 3;
    }
    match approval_request_pending_age_bucket_from_json(request) {
        Some(ApprovalAttentionAgeBucket::Overdue) => 0,
        Some(ApprovalAttentionAgeBucket::Stale) => 1,
        Some(ApprovalAttentionAgeBucket::Fresh) => 2,
        None => 2,
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_session_hotspot_priority(
    age_bucket: Option<ApprovalAttentionAgeBucket>,
) -> usize {
    match age_bucket {
        Some(ApprovalAttentionAgeBucket::Overdue) => 0,
        Some(ApprovalAttentionAgeBucket::Stale) => 1,
        Some(ApprovalAttentionAgeBucket::Fresh) => 2,
        None => 3,
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_unix_ts_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_id(request: &Value) -> &str {
    request
        .get("approval_request_id")
        .and_then(Value::as_str)
        .unwrap_or("")
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_requests_list_request(
    payload: &Value,
    tool_config: &ToolConfig,
) -> Result<ApprovalRequestsListRequest, String> {
    Ok(ApprovalRequestsListRequest {
        session_id: optional_payload_string(payload, "session_id"),
        status: optional_payload_approval_request_status(payload, "status")?,
        execution_plane: optional_payload_tool_execution_plane(payload, "execution_plane")?,
        governance_scope: optional_payload_tool_governance_scope(payload, "governance_scope")?,
        risk_class: optional_payload_tool_risk_class(payload, "risk_class")?,
        pending_age_bucket: optional_payload_approval_pending_age_bucket(
            payload,
            "pending_age_bucket",
        )?,
        tool_name: optional_payload_string(payload, "tool_name"),
        approval_key: optional_payload_string(payload, "approval_key"),
        rule_id: optional_payload_string(payload, "rule_id"),
        decision: optional_payload_approval_list_decision(payload, "decision")?,
        replay_result: optional_payload_approval_replay_result(payload, "replay_result")?,
        integrity_status: optional_payload_approval_execution_integrity_status(
            payload,
            "integrity_status",
        )?,
        needs_attention: payload.get("needs_attention").and_then(Value::as_bool),
        attention_reason: optional_payload_approval_attention_reason(payload, "attention_reason")?,
        recommended_action: optional_payload_approval_recommended_action(
            payload,
            "recommended_action",
        )?,
        attention_age_bucket: optional_payload_approval_attention_age_bucket(
            payload,
            "attention_age_bucket",
        )?,
        escalation_level: optional_payload_approval_escalation_level(payload, "escalation_level")?,
        prioritize_pending: payload
            .get("prioritize_pending")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        prioritize_attention: payload
            .get("prioritize_attention")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        limit: optional_payload_limit(
            payload,
            "limit",
            tool_config.sessions.list_limit,
            tool_config.sessions.list_limit,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_request_resolve_request(
    payload: &Value,
) -> Result<ApprovalRequestResolveRequest, String> {
    Ok(ApprovalRequestResolveRequest {
        approval_request_id: required_payload_string(payload, "approval_request_id")?,
        decision: parse_approval_decision(&required_payload_string(payload, "decision")?)?,
    })
}

#[cfg(feature = "memory-sqlite")]
fn visible_session_ids(
    repo: &SessionRepository,
    current_session_id: &str,
    visibility: SessionVisibility,
) -> Result<Vec<String>, String> {
    match visibility {
        SessionVisibility::SelfOnly => Ok(vec![current_session_id.to_owned()]),
        SessionVisibility::Children => Ok(repo
            .list_visible_sessions(current_session_id)?
            .into_iter()
            .map(|session| session.session_id)
            .collect()),
    }
}

#[cfg(feature = "memory-sqlite")]
fn ensure_visible(
    repo: &SessionRepository,
    current_session_id: &str,
    target_session_id: &str,
    visibility: SessionVisibility,
) -> Result<(), String> {
    let is_visible = match visibility {
        SessionVisibility::SelfOnly => current_session_id == target_session_id,
        SessionVisibility::Children => {
            repo.is_session_visible(current_session_id, target_session_id)?
        }
    };
    if is_visible {
        return Ok(());
    }
    Err(format!(
        "visibility_denied: session `{target_session_id}` is not visible from `{current_session_id}`"
    ))
}

fn required_payload_string(payload: &Value, field: &str) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("approval tool requires payload.{field}"))
}

fn optional_payload_string(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_payload_limit(payload: &Value, field: &str, default: usize, max: usize) -> usize {
    payload
        .get(field)
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, max as u64) as usize)
        .unwrap_or(default)
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_request_status(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalRequestStatus>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_request_status(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_tool_execution_plane(
    payload: &Value,
    field: &str,
) -> Result<Option<ToolExecutionPlane>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_tool_execution_plane(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_tool_governance_scope(
    payload: &Value,
    field: &str,
) -> Result<Option<ToolGovernanceScope>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_tool_governance_scope(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_tool_risk_class(
    payload: &Value,
    field: &str,
) -> Result<Option<ToolRiskClass>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_tool_risk_class(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_execution_integrity_status(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalExecutionIntegrityStatus>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_execution_integrity_status(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_pending_age_bucket(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalAttentionAgeBucket>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_pending_age_bucket(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_list_decision(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalDecision>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_list_decision(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_replay_result(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalReplayResult>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_replay_result(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_attention_reason(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalAttentionReason>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_attention_reason(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_recommended_action(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalRecommendedAction>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_recommended_action(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_attention_age_bucket(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalAttentionAgeBucket>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_attention_age_bucket(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_approval_escalation_level(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalEscalationLevel>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_escalation_level(value.as_str()))
        .transpose()
}

#[cfg(feature = "memory-sqlite")]
fn tool_execution_plane_as_str(value: ToolExecutionPlane) -> &'static str {
    match value {
        ToolExecutionPlane::Core => "Core",
        ToolExecutionPlane::App => "App",
        ToolExecutionPlane::Orchestration => "Orchestration",
    }
}

#[cfg(feature = "memory-sqlite")]
fn tool_governance_scope_as_str(value: ToolGovernanceScope) -> &'static str {
    match value {
        ToolGovernanceScope::Routine => "Routine",
        ToolGovernanceScope::TopologyMutation => "TopologyMutation",
    }
}

#[cfg(feature = "memory-sqlite")]
fn tool_risk_class_as_str(value: ToolRiskClass) -> &'static str {
    match value {
        ToolRiskClass::Low => "Low",
        ToolRiskClass::Elevated => "Elevated",
        ToolRiskClass::High => "High",
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_tool_execution_plane(value: &str) -> Result<ToolExecutionPlane, String> {
    match value {
        "Core" => Ok(ToolExecutionPlane::Core),
        "App" => Ok(ToolExecutionPlane::App),
        "Orchestration" => Ok(ToolExecutionPlane::Orchestration),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown execution_plane `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_tool_governance_scope(value: &str) -> Result<ToolGovernanceScope, String> {
    match value {
        "Routine" => Ok(ToolGovernanceScope::Routine),
        "TopologyMutation" => Ok(ToolGovernanceScope::TopologyMutation),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown governance_scope `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_tool_risk_class(value: &str) -> Result<ToolRiskClass, String> {
    match value {
        "Low" => Ok(ToolRiskClass::Low),
        "Elevated" => Ok(ToolRiskClass::Elevated),
        "High" => Ok(ToolRiskClass::High),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown risk_class `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_request_status(value: &str) -> Result<ApprovalRequestStatus, String> {
    match value {
        "pending" => Ok(ApprovalRequestStatus::Pending),
        "approved" => Ok(ApprovalRequestStatus::Approved),
        "executing" => Ok(ApprovalRequestStatus::Executing),
        "executed" => Ok(ApprovalRequestStatus::Executed),
        "denied" => Ok(ApprovalRequestStatus::Denied),
        "expired" => Ok(ApprovalRequestStatus::Expired),
        "cancelled" => Ok(ApprovalRequestStatus::Cancelled),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown status `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_execution_integrity_status(
    value: &str,
) -> Result<ApprovalExecutionIntegrityStatus, String> {
    match value {
        "not_started" => Ok(ApprovalExecutionIntegrityStatus::NotStarted),
        "in_progress" => Ok(ApprovalExecutionIntegrityStatus::InProgress),
        "complete" => Ok(ApprovalExecutionIntegrityStatus::Complete),
        "incomplete" => Ok(ApprovalExecutionIntegrityStatus::Incomplete),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown integrity_status `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_list_decision(value: &str) -> Result<ApprovalDecision, String> {
    match value {
        "approve_once" => Ok(ApprovalDecision::ApproveOnce),
        "approve_always" => Ok(ApprovalDecision::ApproveAlways),
        "deny" => Ok(ApprovalDecision::Deny),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown decision `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_replay_result(value: &str) -> Result<ApprovalReplayResult, String> {
    match value {
        "not_attempted" => Ok(ApprovalReplayResult::NotAttempted),
        "in_progress" => Ok(ApprovalReplayResult::InProgress),
        "completed_cleanly" => Ok(ApprovalReplayResult::CompletedCleanly),
        "completed_with_attention" => Ok(ApprovalReplayResult::CompletedWithAttention),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown replay_result `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_pending_age_bucket(value: &str) -> Result<ApprovalAttentionAgeBucket, String> {
    match value {
        "fresh" => Ok(ApprovalAttentionAgeBucket::Fresh),
        "stale" => Ok(ApprovalAttentionAgeBucket::Stale),
        "overdue" => Ok(ApprovalAttentionAgeBucket::Overdue),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown pending_age_bucket `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_attention_reason(value: &str) -> Result<ApprovalAttentionReason, String> {
    match value {
        "integrity_gap" => Ok(ApprovalAttentionReason::IntegrityGap),
        "execution_failed" => Ok(ApprovalAttentionReason::ExecutionFailed),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown attention_reason `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_recommended_action(value: &str) -> Result<ApprovalRecommendedAction, String> {
    match value {
        "inspect_execution_event_stream" => {
            Ok(ApprovalRecommendedAction::InspectExecutionEventStream)
        }
        "inspect_replay_persistence" => Ok(ApprovalRecommendedAction::InspectReplayPersistence),
        "inspect_tool_execution_failure" => {
            Ok(ApprovalRecommendedAction::InspectToolExecutionFailure)
        }
        "inspect_approval_integrity" => Ok(ApprovalRecommendedAction::InspectApprovalIntegrity),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown recommended_action `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_attention_age_bucket(value: &str) -> Result<ApprovalAttentionAgeBucket, String> {
    match value {
        "fresh" => Ok(ApprovalAttentionAgeBucket::Fresh),
        "stale" => Ok(ApprovalAttentionAgeBucket::Stale),
        "overdue" => Ok(ApprovalAttentionAgeBucket::Overdue),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown attention_age_bucket `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_escalation_level(value: &str) -> Result<ApprovalEscalationLevel, String> {
    match value {
        "elevated" => Ok(ApprovalEscalationLevel::Elevated),
        "critical" => Ok(ApprovalEscalationLevel::Critical),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown escalation_level `{value}`"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_approval_decision(value: &str) -> Result<ApprovalDecision, String> {
    match value {
        "approve_once" => Ok(ApprovalDecision::ApproveOnce),
        "approve_always" => Ok(ApprovalDecision::ApproveAlways),
        "deny" => Ok(ApprovalDecision::Deny),
        _ => Err(format!(
            "approval_request_resolve_invalid_request: unknown decision `{value}`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use loongclaw_contracts::ToolCoreRequest;
    use rusqlite::params;
    use serde_json::json;
    use serde_json::Value;

    use crate::config::ToolConfig;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        ApprovalDecision, ApprovalRequestStatus, NewApprovalRequestRecord, NewSessionRecord,
        SessionKind, SessionRepository, SessionState, TransitionApprovalRequestIfCurrentRequest,
    };

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-approval-tools-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn seed_session(
        repo: &SessionRepository,
        session_id: &str,
        kind: SessionKind,
        parent_session_id: Option<&str>,
    ) {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind,
            parent_session_id: parent_session_id.map(str::to_owned),
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        })
        .expect("create session");
    }

    #[cfg(feature = "memory-sqlite")]
    fn seed_request(
        repo: &SessionRepository,
        approval_request_id: &str,
        session_id: &str,
        tool_name: &str,
        rule_id: &str,
    ) {
        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: approval_request_id.to_owned(),
            session_id: session_id.to_owned(),
            turn_id: format!("turn-{approval_request_id}"),
            tool_call_id: format!("call-{approval_request_id}"),
            tool_name: tool_name.to_owned(),
            approval_key: format!("tool:{tool_name}"),
            request_payload_json: json!({
                "session_id": session_id,
                "tool_name": tool_name,
                "args_json": {
                    "task": format!("run-{approval_request_id}")
                },
            }),
            governance_snapshot_json: json!({
                "reason": format!("approval required for {tool_name}"),
                "rule_id": rule_id,
                "execution_plane": "Orchestration",
            }),
        })
        .expect("seed approval request");
    }

    #[cfg(feature = "memory-sqlite")]
    fn seed_request_with_governance(
        repo: &SessionRepository,
        approval_request_id: &str,
        session_id: &str,
        tool_name: &str,
        rule_id: &str,
        execution_plane: &str,
        governance_scope: &str,
        risk_class: &str,
    ) {
        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: approval_request_id.to_owned(),
            session_id: session_id.to_owned(),
            turn_id: format!("turn-{approval_request_id}"),
            tool_call_id: format!("call-{approval_request_id}"),
            tool_name: tool_name.to_owned(),
            approval_key: format!("tool:{tool_name}"),
            request_payload_json: json!({
                "session_id": session_id,
                "tool_name": tool_name,
                "args_json": {
                    "task": format!("run-{approval_request_id}")
                },
            }),
            governance_snapshot_json: json!({
                "reason": format!("approval required for {tool_name}"),
                "rule_id": rule_id,
                "execution_plane": execution_plane,
                "governance_scope": governance_scope,
                "risk_class": risk_class,
                "approval_mode": "PolicyDriven",
            }),
        })
        .expect("seed approval request with governance");
    }

    #[cfg(feature = "memory-sqlite")]
    fn transition_request_status(
        repo: &SessionRepository,
        approval_request_id: &str,
        expected_status: ApprovalRequestStatus,
        next_status: ApprovalRequestStatus,
        last_error: Option<&str>,
    ) {
        repo.transition_approval_request_if_current(
            approval_request_id,
            TransitionApprovalRequestIfCurrentRequest {
                expected_status,
                next_status,
                decision: None,
                resolved_by_session_id: None,
                executed_at: Some(1_773_000_000),
                last_error: last_error.map(str::to_owned),
            },
        )
        .expect("transition approval request")
        .expect("approval request should transition");
    }

    #[cfg(feature = "memory-sqlite")]
    fn test_unix_ts_now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default()
    }

    #[cfg(feature = "memory-sqlite")]
    fn overwrite_request_requested_at(
        config: &MemoryRuntimeConfig,
        approval_request_id: &str,
        requested_at: i64,
    ) {
        let db_path = config
            .sqlite_path
            .as_ref()
            .expect("sqlite path should be configured");
        let conn = rusqlite::Connection::open(db_path).expect("open sqlite db");
        let affected = conn
            .execute(
                "UPDATE approval_requests
                 SET requested_at = ?2
                 WHERE approval_request_id = ?1",
                params![approval_request_id, requested_at],
            )
            .expect("overwrite approval request requested_at");
        assert_eq!(affected, 1, "expected to update one approval request row");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_returns_only_visible_requests() {
        let config = isolated_memory_config("approval-query-list-visible");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );
        seed_session(&repo, "hidden-root", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-root-visible",
            "root-session",
            "delegate_async",
            "rule-root",
        );
        seed_request(
            &repo,
            "apr-child-visible",
            "child-session",
            "delegate",
            "rule-child",
        );
        seed_request(
            &repo,
            "apr-hidden",
            "hidden-root",
            "delegate_async",
            "rule-hidden",
        );

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["matched_count"], 2);
        assert_eq!(outcome.payload["returned_count"], 2);
        let requests = outcome.payload["requests"]
            .as_array()
            .expect("requests array");
        let request_ids = requests
            .iter()
            .filter_map(|item| item.get("approval_request_id"))
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(request_ids.contains(&"apr-root-visible"));
        assert!(request_ids.contains(&"apr-child-visible"));
        assert!(!request_ids.contains(&"apr-hidden"));
        let root_request = requests
            .iter()
            .find(|item| item["approval_request_id"] == "apr-root-visible")
            .expect("root visible request");
        assert_eq!(
            root_request["execution_evidence"]["terminal_event_kind"],
            Value::Null
        );
        assert_eq!(
            root_request["execution_evidence"]["evidence_complete"],
            Value::Null
        );
        assert_eq!(root_request["execution_integrity"]["status"], "not_started");
        assert_eq!(root_request["execution_integrity"]["gap"], Value::Null);
        assert_eq!(
            root_request["execution_integrity"]["needs_attention"],
            false
        );
        assert_eq!(
            root_request["execution_integrity"]["attention_reason"],
            Value::Null
        );
        assert_eq!(
            root_request["execution_integrity"]["recommended_action"],
            Value::Null
        );
        assert_eq!(
            root_request["execution_integrity"]["attention_age_seconds"],
            Value::Null
        );
        assert_eq!(
            root_request["execution_integrity"]["attention_age_bucket"],
            Value::Null
        );
        assert_eq!(
            root_request["execution_integrity"]["escalation_level"],
            Value::Null
        );
        assert_eq!(root_request["resolution"]["decision"], Value::Null);
        assert_eq!(root_request["resolution"]["request_status"], "pending");
        assert_eq!(root_request["resolution"]["replay_attempted"], false);
        assert_eq!(root_request["resolution"]["replay_result"], "not_attempted");
        assert_eq!(
            root_request["resolution"]["integrity_status"],
            "not_started"
        );
        assert_eq!(root_request["resolution"]["needs_attention"], false);
        assert_eq!(root_request["resolution"]["attention_reason"], Value::Null);
        assert_eq!(
            root_request["resolution"]["recommended_action"],
            Value::Null
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_surfaces_execution_evidence() {
        let config = isolated_memory_config("approval-query-list-evidence");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-list-evidence",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-list-evidence",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-list-evidence",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append finished event");

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome");

        let requests = outcome.payload["requests"]
            .as_array()
            .expect("requests array");
        let request = requests
            .iter()
            .find(|item| item["approval_request_id"] == "apr-list-evidence")
            .expect("visible request");
        let evidence = &request["execution_evidence"];
        assert_eq!(
            evidence["terminal_event_kind"],
            "tool_approval_execution_finished"
        );
        assert_eq!(evidence["replay_decision_persisted"], true);
        assert_eq!(evidence["replay_outcome_persisted"], true);
        assert_eq!(evidence["evidence_complete"], true);
        let integrity = &request["execution_integrity"];
        assert_eq!(integrity["status"], "complete");
        assert_eq!(integrity["gap"], Value::Null);
        assert_eq!(integrity["execution_error"], Value::Null);
        assert_eq!(integrity["integrity_error"], Value::Null);
        assert_eq!(integrity["needs_attention"], false);
        assert_eq!(integrity["attention_reason"], Value::Null);
        assert_eq!(integrity["recommended_action"], Value::Null);
        assert_eq!(integrity["attention_age_seconds"], Value::Null);
        assert_eq!(integrity["attention_age_bucket"], Value::Null);
        assert_eq!(integrity["escalation_level"], Value::Null);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_scales_execution_evidence_window_for_returned_requests() {
        let config = isolated_memory_config("approval-query-list-evidence-window");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        for index in 0..40 {
            let approval_request_id = format!("apr-list-window-{index:02}");
            seed_request(
                &repo,
                &approval_request_id,
                "root-session",
                "session_cancel",
                "governed_tool_requires_per_call_approval",
            );
            repo.append_event(crate::session::repository::NewSessionEvent {
                session_id: "root-session".to_owned(),
                event_kind: "tool_approval_execution_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({
                    "approval_request_id": approval_request_id,
                    "replay_decision_persisted": true,
                    "replay_decision_persist_error": null,
                }),
            })
            .expect("append started event");
            repo.append_event(crate::session::repository::NewSessionEvent {
                session_id: "root-session".to_owned(),
                event_kind: "tool_approval_execution_finished".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({
                    "approval_request_id": approval_request_id,
                    "resumed_tool_status": "ok",
                    "replay_outcome_persisted": true,
                    "replay_outcome_persist_error": null,
                }),
            })
            .expect("append finished event");
        }

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "limit": 40,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome");

        let requests = outcome.payload["requests"]
            .as_array()
            .expect("requests array");
        assert_eq!(requests.len(), 40);
        let complete_evidence_count = requests
            .iter()
            .filter(|item| item["execution_evidence"]["evidence_complete"] == Value::Bool(true))
            .count();
        assert_eq!(
            complete_evidence_count, 40,
            "all returned requests should retain complete execution evidence"
        );
        let complete_integrity_count = requests
            .iter()
            .filter(|item| {
                item["execution_integrity"]["status"] == Value::String("complete".to_owned())
            })
            .count();
        assert_eq!(complete_integrity_count, 40);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_filters_by_execution_integrity_status() {
        let config = isolated_memory_config("approval-query-list-integrity-filter");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-not-started",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-incomplete",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-complete",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-in-progress",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );

        transition_request_status(
            &repo,
            "apr-incomplete",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("persist assistant turn via kernel failed: forced outcome persistence failure"),
        );
        transition_request_status(
            &repo,
            "apr-complete",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            None,
        );
        transition_request_status(
            &repo,
            "apr-in-progress",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executing,
            None,
        );

        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-incomplete",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append incomplete started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-incomplete",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": false,
                "replay_outcome_persist_error": "persist assistant turn via kernel failed: forced outcome persistence failure",
            }),
        })
        .expect("append incomplete finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-complete",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append complete started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-complete",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append complete finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-in-progress",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append in-progress started event");

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "integrity_status": "incomplete",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome");

        assert_eq!(outcome.payload["filter"]["integrity_status"], "incomplete");
        assert_eq!(outcome.payload["matched_count"], 1);
        assert_eq!(outcome.payload["returned_count"], 1);
        let requests = outcome.payload["requests"]
            .as_array()
            .expect("requests array");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["approval_request_id"], "apr-incomplete");
        assert_eq!(requests[0]["execution_integrity"]["status"], "incomplete");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_filters_by_attention_state() {
        let config = isolated_memory_config("approval-query-list-attention-filter");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-attention-not-started",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-attention-integrity-gap",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-attention-execution-failed",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-attention-in-progress",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );

        transition_request_status(
            &repo,
            "apr-attention-integrity-gap",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("persist assistant turn via kernel failed: forced outcome persistence failure"),
        );
        transition_request_status(
            &repo,
            "apr-attention-execution-failed",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("tool failed for domain reasons"),
        );
        transition_request_status(
            &repo,
            "apr-attention-in-progress",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executing,
            None,
        );

        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-attention-integrity-gap",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append integrity-gap started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-attention-integrity-gap",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": false,
                "replay_outcome_persist_error": "persist assistant turn via kernel failed: forced outcome persistence failure",
            }),
        })
        .expect("append integrity-gap finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-attention-execution-failed",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append execution-failed started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_failed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-attention-execution-failed",
                "error": "tool failed for domain reasons",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append execution-failed terminal event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-attention-in-progress",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append in-progress started event");
        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-attention-integrity-gap", now - 7_200);
        overwrite_request_requested_at(&config, "apr-attention-execution-failed", now - 300);

        let needs_attention_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "needs_attention": true,
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for needs_attention");

        assert_eq!(
            needs_attention_outcome.payload["filter"]["needs_attention"],
            true
        );
        assert_eq!(needs_attention_outcome.payload["matched_count"], 2);
        assert_eq!(needs_attention_outcome.payload["returned_count"], 2);
        let needs_attention_requests = needs_attention_outcome.payload["requests"]
            .as_array()
            .expect("needs attention requests array");
        let needs_attention_ids = needs_attention_requests
            .iter()
            .filter_map(|item| item["approval_request_id"].as_str())
            .collect::<Vec<_>>();
        assert!(needs_attention_ids.contains(&"apr-attention-integrity-gap"));
        assert!(needs_attention_ids.contains(&"apr-attention-execution-failed"));

        let execution_failed_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "attention_reason": "execution_failed",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for attention_reason");

        assert_eq!(
            execution_failed_outcome.payload["filter"]["attention_reason"],
            "execution_failed"
        );
        assert_eq!(execution_failed_outcome.payload["matched_count"], 1);
        assert_eq!(execution_failed_outcome.payload["returned_count"], 1);
        let execution_failed_requests = execution_failed_outcome.payload["requests"]
            .as_array()
            .expect("execution_failed requests array");
        assert_eq!(execution_failed_requests.len(), 1);
        assert_eq!(
            execution_failed_requests[0]["approval_request_id"],
            "apr-attention-execution-failed"
        );
        assert_eq!(
            execution_failed_requests[0]["execution_integrity"]["attention_reason"],
            "execution_failed"
        );
        assert_eq!(
            execution_failed_requests[0]["execution_integrity"]["recommended_action"],
            "inspect_tool_execution_failure"
        );

        let replay_persistence_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "recommended_action": "inspect_replay_persistence",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for recommended_action");

        assert_eq!(
            replay_persistence_outcome.payload["filter"]["recommended_action"],
            "inspect_replay_persistence"
        );
        assert_eq!(replay_persistence_outcome.payload["matched_count"], 1);
        assert_eq!(replay_persistence_outcome.payload["returned_count"], 1);
        let replay_persistence_requests = replay_persistence_outcome.payload["requests"]
            .as_array()
            .expect("recommended_action requests array");
        assert_eq!(replay_persistence_requests.len(), 1);
        assert_eq!(
            replay_persistence_requests[0]["approval_request_id"],
            "apr-attention-integrity-gap"
        );
        assert_eq!(
            replay_persistence_requests[0]["execution_integrity"]["recommended_action"],
            "inspect_replay_persistence"
        );

        let stale_age_bucket_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "attention_age_bucket": "stale",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for attention_age_bucket");

        assert_eq!(
            stale_age_bucket_outcome.payload["filter"]["attention_age_bucket"],
            "stale"
        );
        assert_eq!(stale_age_bucket_outcome.payload["matched_count"], 1);
        assert_eq!(stale_age_bucket_outcome.payload["returned_count"], 1);
        let stale_age_bucket_requests = stale_age_bucket_outcome.payload["requests"]
            .as_array()
            .expect("attention_age_bucket requests array");
        assert_eq!(stale_age_bucket_requests.len(), 1);
        assert_eq!(
            stale_age_bucket_requests[0]["approval_request_id"],
            "apr-attention-integrity-gap"
        );
        assert_eq!(
            stale_age_bucket_requests[0]["execution_integrity"]["attention_age_bucket"],
            "stale"
        );

        let elevated_escalation_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "escalation_level": "elevated",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for escalation_level");

        assert_eq!(
            elevated_escalation_outcome.payload["filter"]["escalation_level"],
            "elevated"
        );
        assert_eq!(elevated_escalation_outcome.payload["matched_count"], 2);
        assert_eq!(elevated_escalation_outcome.payload["returned_count"], 2);
        let elevated_escalation_requests = elevated_escalation_outcome.payload["requests"]
            .as_array()
            .expect("escalation_level requests array");
        let elevated_escalation_ids = elevated_escalation_requests
            .iter()
            .filter_map(|item| item["approval_request_id"].as_str())
            .collect::<Vec<_>>();
        assert!(elevated_escalation_ids.contains(&"apr-attention-integrity-gap"));
        assert!(elevated_escalation_ids.contains(&"apr-attention-execution-failed"));

        let completed_with_attention_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "replay_result": "completed_with_attention",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for replay_result");

        assert_eq!(
            completed_with_attention_outcome.payload["filter"]["replay_result"],
            "completed_with_attention"
        );
        assert_eq!(completed_with_attention_outcome.payload["matched_count"], 2);
        assert_eq!(
            completed_with_attention_outcome.payload["returned_count"],
            2
        );
        let completed_with_attention_requests = completed_with_attention_outcome.payload
            ["requests"]
            .as_array()
            .expect("replay_result requests array");
        let completed_with_attention_ids = completed_with_attention_requests
            .iter()
            .filter_map(|item| item["approval_request_id"].as_str())
            .collect::<Vec<_>>();
        assert!(completed_with_attention_ids.contains(&"apr-attention-integrity-gap"));
        assert!(completed_with_attention_ids.contains(&"apr-attention-execution-failed"));

        let in_progress_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "replay_result": "in_progress",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for in_progress replay_result");

        assert_eq!(
            in_progress_outcome.payload["filter"]["replay_result"],
            "in_progress"
        );
        assert_eq!(in_progress_outcome.payload["matched_count"], 1);
        assert_eq!(in_progress_outcome.payload["returned_count"], 1);
        let in_progress_requests = in_progress_outcome.payload["requests"]
            .as_array()
            .expect("in_progress replay_result requests array");
        assert_eq!(in_progress_requests.len(), 1);
        assert_eq!(
            in_progress_requests[0]["approval_request_id"],
            "apr-attention-in-progress"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_filters_by_decision() {
        let config = isolated_memory_config("approval-query-list-decision-filter");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-decision-pending",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-decision-denied",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-decision-approved",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-decision-once",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );

        repo.transition_approval_request_if_current(
            "apr-decision-denied",
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Denied,
                decision: Some(ApprovalDecision::Deny),
                resolved_by_session_id: Some("root-session".to_owned()),
                executed_at: None,
                last_error: None,
            },
        )
        .expect("transition denied request")
        .expect("denied request should transition");
        repo.transition_approval_request_if_current(
            "apr-decision-approved",
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Approved,
                decision: Some(ApprovalDecision::ApproveAlways),
                resolved_by_session_id: Some("root-session".to_owned()),
                executed_at: None,
                last_error: None,
            },
        )
        .expect("transition approved request")
        .expect("approved request should transition");
        repo.transition_approval_request_if_current(
            "apr-decision-once",
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Executed,
                decision: Some(ApprovalDecision::ApproveOnce),
                resolved_by_session_id: Some("root-session".to_owned()),
                executed_at: Some(1_773_000_000),
                last_error: None,
            },
        )
        .expect("transition approve_once request")
        .expect("approve_once request should transition");

        let approve_always_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "decision": "approve_always",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for approve_always decision");

        assert_eq!(
            approve_always_outcome.payload["filter"]["decision"],
            "approve_always"
        );
        assert_eq!(approve_always_outcome.payload["matched_count"], 1);
        assert_eq!(approve_always_outcome.payload["returned_count"], 1);
        let approve_always_requests = approve_always_outcome.payload["requests"]
            .as_array()
            .expect("approve_always decision requests array");
        assert_eq!(approve_always_requests.len(), 1);
        assert_eq!(
            approve_always_requests[0]["approval_request_id"],
            "apr-decision-approved"
        );

        let deny_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "decision": "deny",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for deny decision");

        assert_eq!(deny_outcome.payload["filter"]["decision"], "deny");
        assert_eq!(deny_outcome.payload["matched_count"], 1);
        assert_eq!(deny_outcome.payload["returned_count"], 1);
        let deny_requests = deny_outcome.payload["requests"]
            .as_array()
            .expect("deny decision requests array");
        assert_eq!(deny_requests.len(), 1);
        assert_eq!(
            deny_requests[0]["approval_request_id"],
            "apr-decision-denied"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_filters_and_summarizes_by_correlation_fields() {
        let config = isolated_memory_config("approval-query-list-correlation-filter");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-correlation-delegate-rule-a",
            "root-session",
            "delegate_async",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-correlation-delegate-rule-b",
            "root-session",
            "delegate_async",
            "governed_tool_requires_allowlist_grant",
        );
        seed_request(
            &repo,
            "apr-correlation-cancel-rule-a",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );

        let summary_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for correlation summary");

        let correlation_summary = &summary_outcome.payload["correlation_summary"];
        assert_eq!(correlation_summary["tool_name_counts"]["delegate_async"], 2);
        assert_eq!(correlation_summary["tool_name_counts"]["session_cancel"], 1);
        assert_eq!(
            correlation_summary["approval_key_counts"]["tool:delegate_async"],
            2
        );
        assert_eq!(
            correlation_summary["approval_key_counts"]["tool:session_cancel"],
            1
        );
        assert_eq!(
            correlation_summary["rule_id_counts"]["governed_tool_requires_per_call_approval"],
            2
        );
        assert_eq!(
            correlation_summary["rule_id_counts"]["governed_tool_requires_allowlist_grant"],
            1
        );

        let tool_name_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "tool_name": "delegate_async",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for tool_name filter");

        assert_eq!(
            tool_name_outcome.payload["filter"]["tool_name"],
            "delegate_async"
        );
        assert_eq!(tool_name_outcome.payload["matched_count"], 2);
        let tool_name_requests = tool_name_outcome.payload["requests"]
            .as_array()
            .expect("tool_name requests array");
        assert_eq!(tool_name_requests.len(), 2);
        assert!(tool_name_requests
            .iter()
            .all(|item| item["tool_name"] == "delegate_async"));

        let approval_key_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "approval_key": "tool:session_cancel",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for approval_key filter");

        assert_eq!(
            approval_key_outcome.payload["filter"]["approval_key"],
            "tool:session_cancel"
        );
        assert_eq!(approval_key_outcome.payload["matched_count"], 1);
        let approval_key_requests = approval_key_outcome.payload["requests"]
            .as_array()
            .expect("approval_key requests array");
        assert_eq!(approval_key_requests.len(), 1);
        assert_eq!(
            approval_key_requests[0]["approval_request_id"],
            "apr-correlation-cancel-rule-a"
        );

        let rule_id_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "rule_id": "governed_tool_requires_per_call_approval",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for rule_id filter");

        assert_eq!(
            rule_id_outcome.payload["filter"]["rule_id"],
            "governed_tool_requires_per_call_approval"
        );
        assert_eq!(rule_id_outcome.payload["matched_count"], 2);
        let rule_id_requests = rule_id_outcome.payload["requests"]
            .as_array()
            .expect("rule_id requests array");
        let rule_id_request_ids = rule_id_requests
            .iter()
            .filter_map(|item| item["approval_request_id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(rule_id_request_ids.len(), 2);
        assert!(rule_id_request_ids.contains(&"apr-correlation-delegate-rule-a"));
        assert!(rule_id_request_ids.contains(&"apr-correlation-cancel-rule-a"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_prioritizes_attention_when_requested() {
        let config = isolated_memory_config("approval-query-list-attention-priority");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-priority-not-started",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-priority-complete",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-priority-integrity-gap",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-priority-execution-failed",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-priority-missing-events",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );

        transition_request_status(
            &repo,
            "apr-priority-complete",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            None,
        );
        transition_request_status(
            &repo,
            "apr-priority-integrity-gap",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("persist assistant turn via kernel failed: forced outcome persistence failure"),
        );
        transition_request_status(
            &repo,
            "apr-priority-execution-failed",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("tool failed for domain reasons"),
        );
        transition_request_status(
            &repo,
            "apr-priority-missing-events",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("approval request executed without persisted execution events"),
        );

        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-priority-complete",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append complete started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-priority-complete",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append complete finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-priority-integrity-gap",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append integrity-gap started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-priority-integrity-gap",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": false,
                "replay_outcome_persist_error": "persist assistant turn via kernel failed: forced outcome persistence failure",
            }),
        })
        .expect("append integrity-gap finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-priority-execution-failed",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append execution-failed started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_failed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-priority-execution-failed",
                "error": "tool failed for domain reasons",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append execution-failed terminal event");
        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-priority-integrity-gap", now - 7_200);
        overwrite_request_requested_at(&config, "apr-priority-execution-failed", now - 300);
        overwrite_request_requested_at(&config, "apr-priority-missing-events", now - 172_800);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "prioritize_attention": true,
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list prioritized outcome");

        assert_eq!(outcome.payload["filter"]["prioritize_attention"], true);
        let requests = outcome.payload["requests"]
            .as_array()
            .expect("requests array");
        assert_eq!(requests.len(), 5);
        assert_eq!(
            requests[0]["approval_request_id"],
            "apr-priority-missing-events"
        );
        assert_eq!(
            requests[0]["execution_integrity"]["escalation_level"],
            "critical"
        );
        assert_eq!(
            requests[1]["approval_request_id"],
            "apr-priority-integrity-gap"
        );
        assert_eq!(
            requests[1]["execution_integrity"]["attention_reason"],
            "integrity_gap"
        );
        assert_eq!(
            requests[1]["execution_integrity"]["escalation_level"],
            "elevated"
        );
        assert_eq!(
            requests[2]["approval_request_id"],
            "apr-priority-execution-failed"
        );
        assert_eq!(
            requests[2]["execution_integrity"]["attention_reason"],
            "execution_failed"
        );
        assert_eq!(
            requests[2]["execution_integrity"]["escalation_level"],
            "elevated"
        );
        assert_eq!(requests[3]["execution_integrity"]["needs_attention"], false);
        assert_eq!(requests[4]["execution_integrity"]["needs_attention"], false);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_prioritizes_pending_queue_and_summarizes_age() {
        let config = isolated_memory_config("approval-query-list-pending-priority");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-pending-fresh",
            "root-session",
            "delegate_async",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-pending-stale",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-pending-overdue",
            "root-session",
            "shell.exec",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-pending-executed",
            "root-session",
            "memory_search",
            "governed_tool_requires_per_call_approval",
        );

        transition_request_status(
            &repo,
            "apr-pending-executed",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            None,
        );

        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-pending-fresh", now - 60);
        overwrite_request_requested_at(&config, "apr-pending-stale", now - 7_200);
        overwrite_request_requested_at(&config, "apr-pending-overdue", now - 172_800);
        overwrite_request_requested_at(&config, "apr-pending-executed", now - 30);

        let prioritized_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "prioritize_pending": true,
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list prioritized pending outcome");

        assert_eq!(
            prioritized_outcome.payload["filter"]["prioritize_pending"],
            true
        );
        let requests = prioritized_outcome.payload["requests"]
            .as_array()
            .expect("pending-priority requests array");
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0]["approval_request_id"], "apr-pending-overdue");
        assert_eq!(requests[0]["pending_queue"]["age_bucket"], "overdue");
        assert_eq!(requests[1]["approval_request_id"], "apr-pending-stale");
        assert_eq!(requests[1]["pending_queue"]["age_bucket"], "stale");
        assert_eq!(requests[2]["approval_request_id"], "apr-pending-fresh");
        assert_eq!(requests[2]["pending_queue"]["age_bucket"], "fresh");
        assert_eq!(requests[3]["approval_request_id"], "apr-pending-executed");
        assert_eq!(requests[3]["pending_queue"]["awaiting_decision"], false);
        assert_eq!(requests[3]["pending_queue"]["age_seconds"], Value::Null);
        assert_eq!(requests[3]["pending_queue"]["age_bucket"], Value::Null);

        let pending_summary = &prioritized_outcome.payload["pending_summary"];
        assert_eq!(pending_summary["pending_count"], 3);
        assert_eq!(pending_summary["counts_by_age_bucket"]["fresh"], 1);
        assert_eq!(pending_summary["counts_by_age_bucket"]["stale"], 1);
        assert_eq!(pending_summary["counts_by_age_bucket"]["overdue"], 1);
        assert_eq!(
            pending_summary["oldest_pending_requested_at"],
            now - 172_800
        );
        assert!(
            pending_summary["oldest_pending_age_seconds"]
                .as_i64()
                .expect("oldest pending age seconds")
                >= 172_800
        );

        let stale_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "pending_age_bucket": "stale",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list stale pending age outcome");

        assert_eq!(
            stale_outcome.payload["filter"]["pending_age_bucket"],
            "stale"
        );
        assert_eq!(stale_outcome.payload["matched_count"], 1);
        let stale_requests = stale_outcome.payload["requests"]
            .as_array()
            .expect("stale pending requests array");
        assert_eq!(stale_requests.len(), 1);
        assert_eq!(
            stale_requests[0]["approval_request_id"],
            "apr-pending-stale"
        );
        assert_eq!(stale_requests[0]["pending_queue"]["age_bucket"], "stale");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_summarizes_session_hotspots() {
        let config = isolated_memory_config("approval-query-list-session-summary");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-hot",
            SessionKind::DelegateChild,
            Some("root-session"),
        );
        seed_session(
            &repo,
            "child-busy",
            SessionKind::DelegateChild,
            Some("root-session"),
        );

        seed_request(
            &repo,
            "apr-root-complete",
            "root-session",
            "memory_search",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-child-hot-pending",
            "child-hot",
            "shell.exec",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-child-hot-attention",
            "child-hot",
            "delegate_async",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-child-busy-pending-a",
            "child-busy",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-child-busy-pending-b",
            "child-busy",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );

        transition_request_status(
            &repo,
            "apr-root-complete",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            None,
        );
        transition_request_status(
            &repo,
            "apr-child-hot-attention",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("tool failed for domain reasons"),
        );

        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-root-complete",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append root complete started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-root-complete",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append root complete finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "child-hot".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-child-hot-attention",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append child-hot started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "child-hot".to_owned(),
            event_kind: "tool_approval_execution_failed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-child-hot-attention",
                "error": "tool failed for domain reasons",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append child-hot failed event");

        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-child-hot-pending", now - 172_800);
        overwrite_request_requested_at(&config, "apr-child-hot-attention", now - 300);
        overwrite_request_requested_at(&config, "apr-child-busy-pending-a", now - 7_200);
        overwrite_request_requested_at(&config, "apr-child-busy-pending-b", now - 60);
        overwrite_request_requested_at(&config, "apr-root-complete", now - 30);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for session summary");

        let session_summary = &outcome.payload["session_summary"];
        assert_eq!(session_summary["session_count"], 3);
        let sessions = session_summary["sessions"]
            .as_array()
            .expect("session summary sessions array");
        assert_eq!(sessions.len(), 3);

        assert_eq!(sessions[0]["session_id"], "child-hot");
        assert_eq!(sessions[0]["request_count"], 2);
        assert_eq!(sessions[0]["pending_count"], 1);
        assert_eq!(sessions[0]["attention_count"], 1);
        assert_eq!(sessions[0]["oldest_pending_age_bucket"], "overdue");
        assert!(
            sessions[0]["oldest_pending_age_seconds"]
                .as_i64()
                .expect("child-hot pending age")
                >= 172_800
        );

        assert_eq!(sessions[1]["session_id"], "child-busy");
        assert_eq!(sessions[1]["request_count"], 2);
        assert_eq!(sessions[1]["pending_count"], 2);
        assert_eq!(sessions[1]["attention_count"], 0);
        assert_eq!(sessions[1]["oldest_pending_age_bucket"], "stale");

        assert_eq!(sessions[2]["session_id"], "root-session");
        assert_eq!(sessions[2]["request_count"], 1);
        assert_eq!(sessions[2]["pending_count"], 0);
        assert_eq!(sessions[2]["attention_count"], 0);
        assert_eq!(sessions[2]["oldest_pending_requested_at"], Value::Null);
        assert_eq!(sessions[2]["oldest_pending_age_seconds"], Value::Null);
        assert_eq!(sessions[2]["oldest_pending_age_bucket"], Value::Null);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_filters_and_summarizes_by_governance_dimensions() {
        let config = isolated_memory_config("approval-query-list-governance-summary");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        seed_request_with_governance(
            &repo,
            "apr-governance-high",
            "root-session",
            "delegate_async",
            "governed_tool_requires_per_call_approval",
            "Orchestration",
            "TopologyMutation",
            "High",
        );
        seed_request_with_governance(
            &repo,
            "apr-governance-elevated",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
            "App",
            "Routine",
            "Elevated",
        );
        seed_request_with_governance(
            &repo,
            "apr-governance-low",
            "root-session",
            "memory_search",
            "governed_tool_requires_per_call_approval",
            "App",
            "Routine",
            "Low",
        );

        let summary_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for governance summary");

        let governance_summary = &summary_outcome.payload["governance_summary"];
        assert_eq!(governance_summary["execution_plane_counts"]["App"], 2);
        assert_eq!(
            governance_summary["execution_plane_counts"]["Orchestration"],
            1
        );
        assert_eq!(governance_summary["governance_scope_counts"]["Routine"], 2);
        assert_eq!(
            governance_summary["governance_scope_counts"]["TopologyMutation"],
            1
        );
        assert_eq!(governance_summary["risk_class_counts"]["Low"], 1);
        assert_eq!(governance_summary["risk_class_counts"]["Elevated"], 1);
        assert_eq!(governance_summary["risk_class_counts"]["High"], 1);

        let orchestration_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "execution_plane": "Orchestration",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for execution_plane filter");

        assert_eq!(
            orchestration_outcome.payload["filter"]["execution_plane"],
            "Orchestration"
        );
        assert_eq!(orchestration_outcome.payload["matched_count"], 1);
        let orchestration_requests = orchestration_outcome.payload["requests"]
            .as_array()
            .expect("orchestration requests array");
        assert_eq!(orchestration_requests.len(), 1);
        assert_eq!(
            orchestration_requests[0]["approval_request_id"],
            "apr-governance-high"
        );
        assert_eq!(
            orchestration_requests[0]["execution_plane"],
            "Orchestration"
        );

        let high_risk_outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "risk_class": "High",
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome for risk_class filter");

        assert_eq!(high_risk_outcome.payload["filter"]["risk_class"], "High");
        assert_eq!(high_risk_outcome.payload["matched_count"], 1);
        let high_risk_requests = high_risk_outcome.payload["requests"]
            .as_array()
            .expect("high risk requests array");
        assert_eq!(high_risk_requests.len(), 1);
        assert_eq!(
            high_risk_requests[0]["approval_request_id"],
            "apr-governance-high"
        );
        assert_eq!(high_risk_requests[0]["risk_class"], "High");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_surfaces_integrity_summary_counts() {
        let config = isolated_memory_config("approval-query-list-integrity-summary");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-summary-not-started",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-summary-in-progress",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-summary-complete",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-summary-incomplete",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-summary-execution-failed",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        seed_request(
            &repo,
            "apr-summary-missing-events",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );

        transition_request_status(
            &repo,
            "apr-summary-in-progress",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executing,
            None,
        );
        transition_request_status(
            &repo,
            "apr-summary-complete",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            None,
        );
        transition_request_status(
            &repo,
            "apr-summary-incomplete",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("persist assistant turn via kernel failed: forced outcome persistence failure"),
        );
        transition_request_status(
            &repo,
            "apr-summary-execution-failed",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("tool failed for domain reasons"),
        );
        transition_request_status(
            &repo,
            "apr-summary-missing-events",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("approval request executed without persisted execution events"),
        );

        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-summary-in-progress",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append in-progress started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-summary-complete",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append complete started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-summary-complete",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append complete finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-summary-incomplete",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append incomplete started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-summary-incomplete",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": false,
                "replay_outcome_persist_error": "persist assistant turn via kernel failed: forced outcome persistence failure",
            }),
        })
        .expect("append incomplete finished event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-summary-execution-failed",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append execution-failed started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_failed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-summary-execution-failed",
                "error": "tool failed for domain reasons",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append execution-failed terminal event");
        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-summary-incomplete", now - 7_200);
        overwrite_request_requested_at(&config, "apr-summary-execution-failed", now - 300);
        overwrite_request_requested_at(&config, "apr-summary-missing-events", now - 172_800);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "limit": 10,
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome");

        let summary = &outcome.payload["integrity_summary"];
        assert_eq!(summary["counts_by_status"]["not_started"], 1);
        assert_eq!(summary["counts_by_status"]["in_progress"], 1);
        assert_eq!(summary["counts_by_status"]["complete"], 2);
        assert_eq!(summary["counts_by_status"]["incomplete"], 2);
        assert_eq!(
            summary["incomplete_gap_counts"]["execution_events_missing"],
            1
        );
        assert_eq!(
            summary["incomplete_gap_counts"]["replay_outcome_missing"],
            1
        );
        assert_eq!(summary["attention_summary"]["needs_attention_count"], 3);
        assert_eq!(
            summary["attention_summary"]["counts_by_reason"]["integrity_gap"],
            2
        );
        assert_eq!(
            summary["attention_summary"]["counts_by_reason"]["execution_failed"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["recommended_action_counts"]
                ["inspect_execution_event_stream"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["recommended_action_counts"]["inspect_replay_persistence"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["recommended_action_counts"]
                ["inspect_tool_execution_failure"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["age_bucket_counts"]["fresh"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["age_bucket_counts"]["stale"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["age_bucket_counts"]["overdue"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["counts_by_escalation"]["elevated"],
            2
        );
        assert_eq!(
            summary["attention_summary"]["counts_by_escalation"]["critical"],
            1
        );
        let resolution_summary = &outcome.payload["resolution_summary"];
        assert_eq!(resolution_summary["decision_counts"]["unresolved"], 6);
        assert_eq!(resolution_summary["decision_counts"]["approve_once"], 0);
        assert_eq!(resolution_summary["decision_counts"]["approve_always"], 0);
        assert_eq!(resolution_summary["decision_counts"]["deny"], 0);
        assert_eq!(resolution_summary["request_status_counts"]["pending"], 1);
        assert_eq!(resolution_summary["request_status_counts"]["approved"], 0);
        assert_eq!(resolution_summary["request_status_counts"]["executing"], 1);
        assert_eq!(resolution_summary["request_status_counts"]["executed"], 4);
        assert_eq!(resolution_summary["request_status_counts"]["denied"], 0);
        assert_eq!(resolution_summary["request_status_counts"]["expired"], 0);
        assert_eq!(resolution_summary["request_status_counts"]["cancelled"], 0);
        assert_eq!(
            resolution_summary["replay_result_counts"]["not_attempted"],
            1
        );
        assert_eq!(resolution_summary["replay_result_counts"]["in_progress"], 1);
        assert_eq!(
            resolution_summary["replay_result_counts"]["completed_cleanly"],
            1
        );
        assert_eq!(
            resolution_summary["replay_result_counts"]["completed_with_attention"],
            3
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_attention_reason() {
        let config = isolated_memory_config("approval-query-list-invalid-attention-reason");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "attention_reason": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown attention_reason should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown attention_reason"),
            "expected attention_reason validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_decision() {
        let config = isolated_memory_config("approval-query-list-invalid-decision");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "decision": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown decision should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown decision"),
            "expected decision validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_replay_result() {
        let config = isolated_memory_config("approval-query-list-invalid-replay-result");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "replay_result": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown replay_result should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown replay_result"),
            "expected replay_result validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_recommended_action() {
        let config = isolated_memory_config("approval-query-list-invalid-recommended-action");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "recommended_action": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown recommended_action should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown recommended_action"),
            "expected recommended_action validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_pending_age_bucket() {
        let config = isolated_memory_config("approval-query-list-invalid-pending-age-bucket");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "pending_age_bucket": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown pending_age_bucket should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown pending_age_bucket"),
            "expected pending_age_bucket validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_execution_plane() {
        let config = isolated_memory_config("approval-query-list-invalid-execution-plane");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "execution_plane": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown execution_plane should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown execution_plane"),
            "expected execution_plane validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_risk_class() {
        let config = isolated_memory_config("approval-query-list-invalid-risk-class");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "risk_class": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown risk_class should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown risk_class"),
            "expected risk_class validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_attention_age_bucket() {
        let config = isolated_memory_config("approval-query-list-invalid-attention-age-bucket");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "attention_age_bucket": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown attention_age_bucket should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown attention_age_bucket"),
            "expected attention_age_bucket validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_escalation_level() {
        let config = isolated_memory_config("approval-query-list-invalid-escalation-level");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "escalation_level": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown escalation_level should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown escalation_level"),
            "expected escalation_level validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_rejects_unknown_execution_integrity_status() {
        let config = isolated_memory_config("approval-query-list-invalid-integrity-status");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "integrity_status": "broken",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("unknown integrity_status should be rejected");

        assert!(
            error.contains("approval_requests_list_invalid_request: unknown integrity_status"),
            "expected integrity_status validation error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_status_returns_full_visible_request_detail() {
        let config = isolated_memory_config("approval-query-status-visible");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );
        seed_request(
            &repo,
            "apr-child-visible",
            "child-session",
            "delegate_async",
            "governed_tool_requires_per_call_approval",
        );

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-child-visible",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        assert_eq!(outcome.status, "ok");
        let request = &outcome.payload["approval_request"];
        assert_eq!(request["approval_request_id"], "apr-child-visible");
        assert_eq!(request["session_id"], "child-session");
        assert_eq!(request["tool_name"], "delegate_async");
        assert_eq!(request["approval_key"], "tool:delegate_async");
        assert_eq!(request["status"], "pending");
        assert_eq!(
            request["governance_snapshot"]["rule_id"],
            "governed_tool_requires_per_call_approval"
        );
        assert_eq!(request["request_payload"]["tool_name"], "delegate_async");
        assert_eq!(
            request["request_payload"]["args_json"]["task"],
            "run-apr-child-visible"
        );
        assert_eq!(
            request["execution_evidence"]["terminal_event_kind"],
            Value::Null
        );
        assert_eq!(
            request["execution_evidence"]["evidence_complete"],
            Value::Null
        );
        assert_eq!(request["execution_integrity"]["status"], "not_started");
        assert_eq!(request["execution_integrity"]["gap"], Value::Null);
        assert_eq!(request["execution_integrity"]["needs_attention"], false);
        assert_eq!(
            request["execution_integrity"]["attention_reason"],
            Value::Null
        );
        assert_eq!(
            request["execution_integrity"]["recommended_action"],
            Value::Null
        );
        assert_eq!(
            request["execution_integrity"]["attention_age_seconds"],
            Value::Null
        );
        assert_eq!(
            request["execution_integrity"]["attention_age_bucket"],
            Value::Null
        );
        assert_eq!(
            request["execution_integrity"]["escalation_level"],
            Value::Null
        );
        assert_eq!(request["resolution"]["decision"], Value::Null);
        assert_eq!(request["resolution"]["request_status"], "pending");
        assert_eq!(request["resolution"]["replay_attempted"], false);
        assert_eq!(request["resolution"]["replay_result"], "not_attempted");
        assert_eq!(request["resolution"]["integrity_status"], "not_started");
        assert_eq!(request["resolution"]["needs_attention"], false);
        assert_eq!(request["resolution"]["attention_reason"], Value::Null);
        assert_eq!(request["resolution"]["recommended_action"], Value::Null);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_status_surfaces_execution_evidence() {
        let config = isolated_memory_config("approval-query-status-evidence");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-visible-evidence",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-visible-evidence",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_finished".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-visible-evidence",
                "resumed_tool_status": "ok",
                "replay_outcome_persisted": false,
                "replay_outcome_persist_error": "persist assistant turn via kernel failed: forced outcome persistence failure",
            }),
        })
        .expect("append finished event");
        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-visible-evidence", now - 7_200);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-visible-evidence",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        let evidence = &outcome.payload["approval_request"]["execution_evidence"];
        assert_eq!(
            evidence["terminal_event_kind"],
            "tool_approval_execution_finished"
        );
        assert_eq!(evidence["replay_decision_persisted"], true);
        assert_eq!(evidence["replay_outcome_persisted"], false);
        assert_eq!(evidence["evidence_complete"], false);
        assert!(
            evidence["replay_outcome_persist_error"]
                .as_str()
                .is_some_and(|error| error.contains("persist assistant turn via kernel failed")),
            "expected replay outcome persistence error in evidence, got: {evidence}"
        );
        let integrity = &outcome.payload["approval_request"]["execution_integrity"];
        assert_eq!(integrity["status"], "incomplete");
        assert_eq!(integrity["gap"], "replay_outcome_missing");
        assert_eq!(integrity["execution_error"], Value::Null);
        assert_eq!(integrity["needs_attention"], true);
        assert_eq!(integrity["attention_reason"], "integrity_gap");
        assert_eq!(
            integrity["recommended_action"],
            "inspect_replay_persistence"
        );
        assert!(
            integrity["attention_age_seconds"]
                .as_i64()
                .is_some_and(|value| value >= 7_200),
            "expected stale attention age in integrity payload, got: {integrity}"
        );
        assert_eq!(integrity["attention_age_bucket"], "stale");
        assert_eq!(integrity["escalation_level"], "elevated");
        assert!(
            integrity["integrity_error"]
                .as_str()
                .is_some_and(|error| error.contains("persist assistant turn via kernel failed")),
            "expected replay outcome persistence integrity error, got: {integrity}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_status_recommends_event_stream_investigation_for_missing_execution_events(
    ) {
        let config = isolated_memory_config("approval-query-status-missing-execution-events");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-visible-missing-events",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        transition_request_status(
            &repo,
            "apr-visible-missing-events",
            ApprovalRequestStatus::Pending,
            ApprovalRequestStatus::Executed,
            Some("approval request executed without persisted execution events"),
        );
        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-visible-missing-events", now - 172_800);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-visible-missing-events",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        let integrity = &outcome.payload["approval_request"]["execution_integrity"];
        assert_eq!(integrity["status"], "incomplete");
        assert_eq!(integrity["gap"], "execution_events_missing");
        assert_eq!(
            integrity["recommended_action"],
            "inspect_execution_event_stream"
        );
        assert!(
            integrity["attention_age_seconds"]
                .as_i64()
                .is_some_and(|value| value >= 172_800),
            "expected overdue attention age in integrity payload, got: {integrity}"
        );
        assert_eq!(integrity["attention_age_bucket"], "overdue");
        assert_eq!(integrity["escalation_level"], "critical");
        let resolution = &outcome.payload["approval_request"]["resolution"];
        assert_eq!(resolution["decision"], Value::Null);
        assert_eq!(resolution["request_status"], "executed");
        assert_eq!(resolution["replay_attempted"], true);
        assert_eq!(resolution["replay_result"], "completed_with_attention");
        assert_eq!(resolution["integrity_status"], "incomplete");
        assert_eq!(resolution["needs_attention"], true);
        assert_eq!(resolution["attention_reason"], "integrity_gap");
        assert_eq!(
            resolution["recommended_action"],
            "inspect_execution_event_stream"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_status_distinguishes_execution_failure_from_integrity_gap() {
        let config = isolated_memory_config("approval-query-status-execution-failure");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-visible-execution-failure",
            "root-session",
            "session_cancel",
            "governed_tool_requires_per_call_approval",
        );
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-visible-execution-failure",
                "replay_decision_persisted": true,
                "replay_decision_persist_error": null,
            }),
        })
        .expect("append started event");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "tool_approval_execution_failed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "approval_request_id": "apr-visible-execution-failure",
                "error": "tool failed for domain reasons",
                "replay_outcome_persisted": true,
                "replay_outcome_persist_error": null,
            }),
        })
        .expect("append failed event");
        let now = test_unix_ts_now();
        overwrite_request_requested_at(&config, "apr-visible-execution-failure", now - 300);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-visible-execution-failure",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        let integrity = &outcome.payload["approval_request"]["execution_integrity"];
        assert_eq!(integrity["status"], "complete");
        assert_eq!(integrity["gap"], Value::Null);
        assert_eq!(integrity["integrity_error"], Value::Null);
        assert_eq!(
            integrity["execution_error"],
            "tool failed for domain reasons"
        );
        assert_eq!(integrity["needs_attention"], true);
        assert_eq!(integrity["attention_reason"], "execution_failed");
        assert_eq!(
            integrity["recommended_action"],
            "inspect_tool_execution_failure"
        );
        assert!(
            integrity["attention_age_seconds"]
                .as_i64()
                .is_some_and(|value| value >= 300),
            "expected fresh attention age in integrity payload, got: {integrity}"
        );
        assert_eq!(integrity["attention_age_bucket"], "fresh");
        assert_eq!(integrity["escalation_level"], "elevated");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_status_rejects_hidden_request() {
        let config = isolated_memory_config("approval-query-status-hidden");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(&repo, "hidden-root", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-hidden",
            "hidden-root",
            "delegate_async",
            "rule-hidden",
        );

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-hidden",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("hidden approval request should be rejected");

        assert!(
            error.contains("visibility_denied"),
            "expected visibility_denied, got: {error}"
        );
    }
}
