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
    limit: usize,
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

    let matched_count = requests.len();
    requests.truncate(request.limit);
    let returned_count = requests.len();
    let recent_events_by_session_id =
        approval_request_recent_events_by_session_id(&repo, &requests)?;
    let request_summaries = requests
        .iter()
        .map(|record| {
            let session_events = recent_events_by_session_id
                .get(&record.session_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            approval_request_summary_json_with_evidence(
                record,
                approval_request_execution_evidence_json_from_events(
                    session_events,
                    &record.approval_request_id,
                ),
            )
        })
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "filter": {
                "session_id": request.session_id,
                "status": request.status.map(ApprovalRequestStatus::as_str),
                "limit": request.limit,
            },
            "visible_session_ids": target_session_ids,
            "matched_count": matched_count,
            "returned_count": returned_count,
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

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "approval_request": approval_request_detail_json_with_evidence(
                &request,
                approval_request_execution_evidence_json(&repo, &request)?,
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

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "approval_request": approval_request_detail_json_with_evidence(
                &outcome.approval_request,
                approval_request_execution_evidence_json(&repo, &outcome.approval_request)?,
            ),
            "resumed_tool_output": outcome.resumed_tool_output,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_summary_json_with_evidence(
    record: &ApprovalRequestRecord,
    execution_evidence: Value,
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
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_detail_json_with_evidence(
    record: &ApprovalRequestRecord,
    execution_evidence: Value,
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
fn parse_approval_requests_list_request(
    payload: &Value,
    tool_config: &ToolConfig,
) -> Result<ApprovalRequestsListRequest, String> {
    Ok(ApprovalRequestsListRequest {
        session_id: optional_payload_string(payload, "session_id"),
        status: optional_payload_approval_request_status(payload, "status")?,
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
        NewApprovalRequestRecord, NewSessionRecord, SessionKind, SessionRepository, SessionState,
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
