#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeMap;

use async_trait::async_trait;
use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{json, Value};

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
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalRequestsListRequest {
    session_id: Option<String>,
    status: Option<ApprovalRequestStatus>,
    integrity_status: Option<ApprovalExecutionIntegrityStatus>,
    needs_attention: Option<bool>,
    attention_reason: Option<ApprovalAttentionReason>,
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
    if request.prioritize_attention {
        request_summaries.sort_by(|left, right| {
            approval_request_attention_priority(left)
                .cmp(&approval_request_attention_priority(right))
                .then_with(|| {
                    approval_request_requested_at(right).cmp(&approval_request_requested_at(left))
                })
                .then_with(|| approval_request_id(left).cmp(approval_request_id(right)))
        });
    }
    let integrity_summary = approval_request_list_integrity_summary_json(&request_summaries);
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
                "integrity_status": request.integrity_status.map(ApprovalExecutionIntegrityStatus::as_str),
                "needs_attention": request.needs_attention,
                "attention_reason": request.attention_reason.map(ApprovalAttentionReason::as_str),
                "prioritize_attention": request.prioritize_attention,
                "limit": request.limit,
            },
            "visible_session_ids": target_session_ids,
            "matched_count": matched_count,
            "returned_count": returned_count,
            "integrity_summary": integrity_summary,
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

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
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
    json!({
        "approval_request_id": record.approval_request_id,
        "session_id": record.session_id,
        "turn_id": record.turn_id,
        "tool_call_id": record.tool_call_id,
        "tool_name": record.tool_name,
        "approval_key": record.approval_key,
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
        "execution_evidence": execution_evidence,
        "execution_integrity": execution_integrity,
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
        }
    }

    json!({
        "counts_by_status": counts_by_status,
        "incomplete_gap_counts": incomplete_gap_counts,
        "attention_summary": {
            "needs_attention_count": needs_attention_count,
            "counts_by_reason": counts_by_reason,
        },
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_detail_json_with_evidence(
    record: &ApprovalRequestRecord,
    execution_evidence: Value,
    execution_integrity: Value,
) -> Value {
    json!({
        "approval_request_id": record.approval_request_id,
        "session_id": record.session_id,
        "turn_id": record.turn_id,
        "tool_call_id": record.tool_call_id,
        "tool_name": record.tool_name,
        "approval_key": record.approval_key,
        "status": record.status.as_str(),
        "decision": record.decision.map(|decision| decision.as_str()),
        "requested_at": record.requested_at,
        "resolved_at": record.resolved_at,
        "resolved_by_session_id": record.resolved_by_session_id,
        "executed_at": record.executed_at,
        "last_error": record.last_error,
        "request_payload": record.request_payload_json,
        "governance_snapshot": record.governance_snapshot_json,
        "execution_evidence": execution_evidence,
        "execution_integrity": execution_integrity,
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

    json!({
        "status": status.as_str(),
        "gap": gap,
        "integrity_error": integrity_error,
        "execution_error": execution_error,
        "evidence_complete": evidence_complete,
        "needs_attention": needs_attention,
        "attention_reason": attention_reason,
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
fn approval_request_attention_reason_from_json(
    execution_integrity: &Value,
) -> Option<ApprovalAttentionReason> {
    execution_integrity
        .get("attention_reason")
        .and_then(Value::as_str)
        .and_then(|value| parse_approval_attention_reason(value).ok())
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
fn approval_request_requested_at(request: &Value) -> i64 {
    request
        .get("requested_at")
        .and_then(Value::as_i64)
        .unwrap_or(i64::MIN)
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
        integrity_status: optional_payload_approval_execution_integrity_status(
            payload,
            "integrity_status",
        )?,
        needs_attention: payload.get("needs_attention").and_then(Value::as_bool),
        attention_reason: optional_payload_approval_attention_reason(payload, "attention_reason")?,
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
fn optional_payload_approval_execution_integrity_status(
    payload: &Value,
    field: &str,
) -> Result<Option<ApprovalExecutionIntegrityStatus>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_approval_execution_integrity_status(value.as_str()))
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

    use loongclaw_contracts::ToolCoreRequest;
    use serde_json::json;
    use serde_json::Value;

    use crate::config::ToolConfig;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        ApprovalRequestStatus, NewApprovalRequestRecord, NewSessionRecord, SessionKind,
        SessionRepository, SessionState, TransitionApprovalRequestIfCurrentRequest,
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
        assert_eq!(requests.len(), 4);
        assert_eq!(
            requests[0]["approval_request_id"],
            "apr-priority-integrity-gap"
        );
        assert_eq!(
            requests[0]["execution_integrity"]["attention_reason"],
            "integrity_gap"
        );
        assert_eq!(
            requests[1]["approval_request_id"],
            "apr-priority-execution-failed"
        );
        assert_eq!(
            requests[1]["execution_integrity"]["attention_reason"],
            "execution_failed"
        );
        assert_eq!(requests[2]["execution_integrity"]["needs_attention"], false);
        assert_eq!(requests[3]["execution_integrity"]["needs_attention"], false);
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
        assert_eq!(summary["counts_by_status"]["incomplete"], 1);
        assert_eq!(
            summary["incomplete_gap_counts"]["replay_outcome_missing"],
            1
        );
        assert_eq!(summary["attention_summary"]["needs_attention_count"], 2);
        assert_eq!(
            summary["attention_summary"]["counts_by_reason"]["integrity_gap"],
            1
        );
        assert_eq!(
            summary["attention_summary"]["counts_by_reason"]["execution_failed"],
            1
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
        assert!(
            integrity["integrity_error"]
                .as_str()
                .is_some_and(|error| error.contains("persist assistant turn via kernel failed")),
            "expected replay outcome persistence integrity error, got: {integrity}"
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
