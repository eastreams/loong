use async_trait::async_trait;
use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};
#[cfg(feature = "memory-sqlite")]
use std::time::{SystemTime, UNIX_EPOCH};

use super::payload::{optional_payload_limit, optional_payload_string, required_payload_string};

use crate::config::ToolConfig;
#[cfg(feature = "memory-sqlite")]
use crate::config::{SessionVisibility, ToolConsentMode};
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::operator::approval_runtime::OperatorApprovalRuntime;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalDecision, ApprovalGrantRecord, ApprovalRequestRecord, ApprovalRequestStatus,
    NewApprovalGrantRecord, NewSessionToolConsentRecord, SessionRepository,
    TransitionApprovalRequestIfCurrentRequest,
};

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalRequestsListRequest {
    session_id: Option<String>,
    status: Option<ApprovalRequestStatus>,
    grant_attention: Option<GrantAttentionFilter>,
    limit: usize,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalRequestResolveRequest {
    approval_request_id: String,
    decision: ApprovalDecision,
    session_consent_mode: Option<ToolConsentMode>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
pub(crate) struct ApprovalResolutionRequest {
    pub current_session_id: String,
    pub approval_request_id: String,
    pub decision: ApprovalDecision,
    pub session_consent_mode: Option<ToolConsentMode>,
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
    fn can_replay_approved_request(&self) -> bool {
        true
    }

    fn ensure_resolution_binding_allows_decision(
        &self,
        approval_request: &ApprovalRequestRecord,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        let _ = approval_request;
        let _ = decision;
        Ok(())
    }

    async fn replay_approved_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<ToolCoreOutcome, String>;
}

#[cfg(feature = "memory-sqlite")]
const APPROVAL_GRANT_REVIEW_STALE_AFTER_SECONDS: i64 = 60 * 60 * 24 * 30;

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AttentionSeverity {
    Medium,
    High,
}

#[cfg(feature = "memory-sqlite")]
impl AttentionSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
struct AttentionSignal {
    source: &'static str,
    kind: &'static str,
    severity: AttentionSeverity,
    action: &'static str,
    detail: Option<String>,
}

#[cfg(feature = "memory-sqlite")]
impl AttentionSignal {
    fn to_json(&self) -> Value {
        json!({
            "source": self.source,
            "kind": self.kind,
            "severity": self.severity.as_str(),
            "action": self.action,
            "detail": self.detail,
        })
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrantAttentionState {
    NotApplicable,
    Clean,
    MissingGrant,
    ReviewStale,
}

#[cfg(feature = "memory-sqlite")]
impl GrantAttentionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::Clean => "clean",
            Self::MissingGrant => "missing_grant",
            Self::ReviewStale => "review_stale",
        }
    }

    fn needs_attention(self) -> bool {
        matches!(self, Self::MissingGrant | Self::ReviewStale)
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrantAttentionFilter {
    NeedsAttention,
    MissingGrant,
    ReviewStale,
    Clean,
    NotApplicable,
}

#[cfg(feature = "memory-sqlite")]
impl GrantAttentionFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::NeedsAttention => "needs_attention",
            Self::MissingGrant => "missing_grant",
            Self::ReviewStale => "review_stale",
            Self::Clean => "clean",
            Self::NotApplicable => "not_applicable",
        }
    }

    fn matches(self, state: GrantAttentionState) -> bool {
        match self {
            Self::NeedsAttention => state.needs_attention(),
            Self::MissingGrant => state == GrantAttentionState::MissingGrant,
            Self::ReviewStale => state == GrantAttentionState::ReviewStale,
            Self::Clean => state == GrantAttentionState::Clean,
            Self::NotApplicable => state == GrantAttentionState::NotApplicable,
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
struct DerivedAttentionView {
    execution_signals: Vec<AttentionSignal>,
    grant_signals: Vec<AttentionSignal>,
    grant_state: GrantAttentionState,
    grant_scope_session_id: Option<String>,
    grant_record: Option<ApprovalGrantRecord>,
    grant_age_seconds: Option<i64>,
}

#[cfg(feature = "memory-sqlite")]
impl DerivedAttentionView {
    fn combined_signals(&self) -> Vec<AttentionSignal> {
        let mut signals = self.execution_signals.clone();
        signals.extend(self.grant_signals.clone());
        signals
    }

    fn needs_attention(&self) -> bool {
        !self.execution_signals.is_empty() || !self.grant_signals.is_empty()
    }

    fn highest_severity(&self) -> Option<AttentionSeverity> {
        self.combined_signals()
            .into_iter()
            .map(|signal| signal.severity)
            .max()
    }

    fn highest_severity_str(&self) -> &'static str {
        self.highest_severity()
            .map(AttentionSeverity::as_str)
            .unwrap_or("none")
    }

    fn primary_action(&self) -> Option<&'static str> {
        let mut signals = self.combined_signals();
        signals.sort_by(|left, right| {
            right
                .severity
                .cmp(&left.severity)
                .then_with(|| left.kind.cmp(right.kind))
        });
        signals.first().map(|signal| signal.action)
    }

    fn source_category(&self) -> &'static str {
        match (
            self.execution_signals.is_empty(),
            self.grant_signals.is_empty(),
        ) {
            (false, false) => "combined",
            (false, true) => "execution_only",
            (true, false) => "grant_only",
            (true, true) => "none",
        }
    }

    fn reason_kinds(&self) -> Vec<&'static str> {
        self.combined_signals()
            .into_iter()
            .map(|signal| signal.kind)
            .collect()
    }

    fn execution_state(&self) -> &'static str {
        if self
            .execution_signals
            .iter()
            .any(|signal| signal.kind == "resumed_execution_failed")
        {
            "resume_failed"
        } else if self
            .execution_signals
            .iter()
            .any(|signal| signal.kind == "resume_incomplete")
        {
            "resume_incomplete"
        } else if self
            .execution_signals
            .iter()
            .any(|signal| signal.kind == "pending_operator_decision")
        {
            "pending_decision"
        } else {
            "clean"
        }
    }

    fn execution_integrity_json(&self) -> Value {
        json!({
            "state": self.execution_state(),
            "needs_attention": !self.execution_signals.is_empty(),
            "signals": self.execution_signals.iter().map(AttentionSignal::to_json).collect::<Vec<_>>(),
            "highest_escalation_level": self
                .execution_signals
                .iter()
                .map(|signal| signal.severity)
                .max()
                .map(AttentionSeverity::as_str)
                .unwrap_or("none"),
        })
    }

    fn grant_review_json(&self) -> Value {
        json!({
            "state": self.grant_state.as_str(),
            "needs_attention": self.grant_state.needs_attention(),
            "scope_session_id": self.grant_scope_session_id,
            "grant_exists": self.grant_record.is_some(),
            "grant_created_by_session_id": self
                .grant_record
                .as_ref()
                .and_then(|grant| grant.created_by_session_id.clone()),
            "grant_created_at": self.grant_record.as_ref().map(|grant| grant.created_at),
            "grant_updated_at": self.grant_record.as_ref().map(|grant| grant.updated_at),
            "grant_age_seconds": self.grant_age_seconds,
            "review_stale_after_seconds": self
                .grant_record
                .as_ref()
                .map(|_| APPROVAL_GRANT_REVIEW_STALE_AFTER_SECONDS),
            "signals": self.grant_signals.iter().map(AttentionSignal::to_json).collect::<Vec<_>>(),
        })
    }

    fn grant_attention_json(&self) -> Value {
        json!({
            "state": self.grant_state.as_str(),
            "needs_attention": self.grant_state.needs_attention(),
            "signals": self.grant_signals.iter().map(AttentionSignal::to_json).collect::<Vec<_>>(),
            "highest_escalation_level": self
                .grant_signals
                .iter()
                .map(|signal| signal.severity)
                .max()
                .map(AttentionSeverity::as_str)
                .unwrap_or("none"),
        })
    }

    fn attention_json(&self) -> Value {
        let signals = self.combined_signals();
        let mut sources = Vec::new();
        for source in ["execution", "grant"] {
            if signals.iter().any(|signal| signal.source == source) {
                sources.push(source);
            }
        }
        json!({
            "needs_attention": self.needs_attention(),
            "sources": sources,
            "signals": signals.iter().map(AttentionSignal::to_json).collect::<Vec<_>>(),
            "highest_escalation_level": self.highest_severity_str(),
            "primary_action": self.primary_action(),
        })
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
struct ApprovalRequestView {
    record: ApprovalRequestRecord,
    attention: DerivedAttentionView,
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
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, current_session_id, config, tool_config, runtime);
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
            "approval_request_resolve" => {
                let runtime =
                    runtime.ok_or_else(|| "approval_request_runtime_not_configured".to_owned())?;
                execute_approval_request_resolve(
                    request.payload,
                    current_session_id,
                    config,
                    tool_config,
                    runtime,
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

    let mut request_views = Vec::new();
    for record in requests {
        let attention = derive_attention_view(&repo, &record)?;
        if request
            .grant_attention
            .is_some_and(|filter| !filter.matches(attention.grant_state))
        {
            continue;
        }
        request_views.push(ApprovalRequestView { record, attention });
    }

    request_views.sort_by(|left, right| {
        right
            .record
            .requested_at
            .cmp(&left.record.requested_at)
            .then_with(|| {
                left.record
                    .approval_request_id
                    .cmp(&right.record.approval_request_id)
            })
    });

    let matched_count = request_views.len();
    let attention_summary = approval_attention_summary_json(&request_views);
    request_views.truncate(request.limit);
    let returned_count = request_views.len();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "filter": {
                "session_id": request.session_id,
                "status": request.status.map(ApprovalRequestStatus::as_str),
                "grant_attention": request.grant_attention.map(GrantAttentionFilter::as_str),
                "limit": request.limit,
            },
            "visible_session_ids": target_session_ids,
            "matched_count": matched_count,
            "returned_count": returned_count,
            "attention_summary": attention_summary,
            "requests": request_views
                .iter()
                .map(approval_request_summary_json)
                .collect::<Vec<_>>(),
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
    let approval_request_id =
        required_payload_string(&payload, "approval_request_id", "approval tool")?;
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
    let attention = derive_attention_view(&repo, &request)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "approval_request": approval_request_detail_json(&request, &attention),
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
) -> Result<ToolCoreOutcome, String> {
    let request = parse_approval_request_resolve_request(&payload)?;
    let resolution_request = ApprovalResolutionRequest {
        current_session_id: current_session_id.to_owned(),
        approval_request_id: request.approval_request_id,
        decision: request.decision,
        session_consent_mode: request.session_consent_mode,
        visibility: tool_config.sessions.visibility,
    };
    let outcome =
        resolve_approval_request_with_runtime(config, runtime, resolution_request).await?;
    let repo = SessionRepository::new(config)?;
    let attention = derive_attention_view(&repo, &outcome.approval_request)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "approval_request": approval_request_detail_json(&outcome.approval_request, &attention),
            "resumed_tool_output": outcome.resumed_tool_output,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
async fn resolve_approval_request_with_runtime(
    config: &MemoryRuntimeConfig,
    runtime: &(dyn ApprovalResolutionRuntime + '_),
    request: ApprovalResolutionRequest,
) -> Result<ApprovalResolutionOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let approval_request = load_visible_approval_request(&repo, &request)?;

    runtime.ensure_resolution_binding_allows_decision(&approval_request, request.decision)?;

    match request.decision {
        ApprovalDecision::Deny => {
            resolve_denied_approval_request(&repo, &request, &approval_request)
        }
        ApprovalDecision::ApproveOnce => {
            let approved = transition_approval_request_to_approved(
                &repo,
                &request,
                ApprovalDecision::ApproveOnce,
                approval_request,
            )?;
            persist_session_consent_if_requested(
                &repo,
                &approved,
                &request.current_session_id,
                request.session_consent_mode,
            )?;
            finish_approved_resolution(&repo, runtime, approved).await
        }
        ApprovalDecision::ApproveAlways => {
            let approved = transition_approval_request_to_approved(
                &repo,
                &request,
                ApprovalDecision::ApproveAlways,
                approval_request,
            )?;
            persist_runtime_grant_for_approved_request(
                &repo,
                &approved,
                &request.current_session_id,
            )?;
            finish_approved_resolution(&repo, runtime, approved).await
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn load_visible_approval_request(
    repo: &SessionRepository,
    request: &ApprovalResolutionRequest,
) -> Result<ApprovalRequestRecord, String> {
    let approval_request = repo
        .load_approval_request(&request.approval_request_id)?
        .ok_or_else(|| {
            format!(
                "approval_request_not_found: `{}`",
                request.approval_request_id
            )
        })?;

    let is_visible = match request.visibility {
        SessionVisibility::SelfOnly => request.current_session_id == approval_request.session_id,
        SessionVisibility::Children => {
            let current_session_id = request.current_session_id.as_str();
            let target_session_id = approval_request.session_id.as_str();
            let same_session = current_session_id == target_session_id;
            if same_session {
                true
            } else {
                repo.is_session_visible(current_session_id, target_session_id)?
            }
        }
    };

    if !is_visible {
        let current_session_id = request.current_session_id.as_str();
        let target_session_id = approval_request.session_id.as_str();
        let error = format!(
            "visibility_denied: session `{target_session_id}` is not visible from `{current_session_id}`"
        );
        return Err(error);
    }

    Ok(approval_request)
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_not_pending_error(approval_request: &ApprovalRequestRecord) -> String {
    let approval_request_id = approval_request.approval_request_id.as_str();
    let status = approval_request.status.as_str();
    format!("approval_request_not_pending: `{approval_request_id}` is already {status}")
}

#[cfg(feature = "memory-sqlite")]
fn transition_approval_request_to_approved(
    repo: &SessionRepository,
    request: &ApprovalResolutionRequest,
    expected_decision: ApprovalDecision,
    approval_request: ApprovalRequestRecord,
) -> Result<ApprovalRequestRecord, String> {
    let approval_request_id = request.approval_request_id.as_str();
    let resolved_by_session_id = request.current_session_id.clone();
    let updated = repo.transition_approval_request_if_current(
        approval_request_id,
        TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Pending,
            next_status: ApprovalRequestStatus::Approved,
            decision: Some(expected_decision),
            resolved_by_session_id: Some(resolved_by_session_id),
            executed_at: None,
            last_error: None,
        },
    )?;

    let Some(approved) = updated else {
        return resume_existing_approved_request(
            repo,
            request,
            approval_request,
            expected_decision,
        );
    };

    Ok(approved)
}

#[cfg(feature = "memory-sqlite")]
fn resume_existing_approved_request(
    repo: &SessionRepository,
    request: &ApprovalResolutionRequest,
    approval_request: ApprovalRequestRecord,
    expected_decision: ApprovalDecision,
) -> Result<ApprovalRequestRecord, String> {
    if approval_request.status != ApprovalRequestStatus::Approved {
        let error = approval_request_not_pending_error(&approval_request);
        return Err(error);
    }

    let recorded_decision = approval_request.decision.ok_or_else(|| {
        let approval_request_id = request.approval_request_id.as_str();
        format!("approval_request_missing_decision: `{approval_request_id}` is approved")
    })?;

    if recorded_decision != expected_decision {
        let approval_request_id = request.approval_request_id.as_str();
        let recorded_decision_name = recorded_decision.as_str();
        let expected_decision_name = expected_decision.as_str();
        let error = format!(
            "approval_request_decision_mismatch: `{approval_request_id}` is already `{recorded_decision_name}`, expected `{expected_decision_name}`"
        );
        return Err(error);
    }

    if expected_decision == ApprovalDecision::ApproveAlways {
        persist_runtime_grant_for_approved_request(
            repo,
            &approval_request,
            &request.current_session_id,
        )?;
    }

    persist_session_consent_if_requested(
        repo,
        &approval_request,
        &request.current_session_id,
        request.session_consent_mode,
    )?;

    Ok(approval_request)
}

#[cfg(feature = "memory-sqlite")]
fn resolve_denied_approval_request(
    repo: &SessionRepository,
    request: &ApprovalResolutionRequest,
    approval_request: &ApprovalRequestRecord,
) -> Result<ApprovalResolutionOutcome, String> {
    if approval_request.status != ApprovalRequestStatus::Pending {
        let error = approval_request_not_pending_error(approval_request);
        return Err(error);
    }

    let approval_request_id = request.approval_request_id.as_str();
    let resolved_by_session_id = request.current_session_id.clone();
    let denied = repo.transition_approval_request_if_current(
        approval_request_id,
        TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Pending,
            next_status: ApprovalRequestStatus::Denied,
            decision: Some(ApprovalDecision::Deny),
            resolved_by_session_id: Some(resolved_by_session_id),
            executed_at: None,
            last_error: None,
        },
    )?;

    let Some(denied) = denied else {
        let latest = repo
            .load_approval_request(approval_request_id)?
            .ok_or_else(|| {
                format!(
                    "approval_request_not_found: `{}`",
                    request.approval_request_id
                )
            })?;
        let error = approval_request_not_pending_error(&latest);
        return Err(error);
    };

    Ok(ApprovalResolutionOutcome {
        approval_request: denied,
        resumed_tool_output: None,
    })
}

#[cfg(feature = "memory-sqlite")]
fn persist_runtime_grant_for_approved_request(
    repo: &SessionRepository,
    approval_request: &ApprovalRequestRecord,
    current_session_id: &str,
) -> Result<(), String> {
    let scope_session_id = approval_request_scope_session_id(repo, approval_request)?;

    let grant_record = NewApprovalGrantRecord {
        scope_session_id,
        approval_key: approval_request.approval_key.clone(),
        created_by_session_id: Some(current_session_id.to_owned()),
    };

    repo.upsert_approval_grant(grant_record)?;

    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn persist_session_consent_if_requested(
    repo: &SessionRepository,
    approval_request: &ApprovalRequestRecord,
    current_session_id: &str,
    session_consent_mode: Option<ToolConsentMode>,
) -> Result<(), String> {
    let Some(session_consent_mode) = session_consent_mode else {
        return Ok(());
    };

    let scope_session_id = approval_request_scope_session_id(repo, approval_request)?;

    let consent_record = NewSessionToolConsentRecord {
        scope_session_id,
        mode: session_consent_mode,
        updated_by_session_id: Some(current_session_id.to_owned()),
    };

    repo.upsert_session_tool_consent(consent_record)?;

    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_parent_session_id(approval_request: &ApprovalRequestRecord) -> Option<&str> {
    let parent_session_value = approval_request
        .request_payload_json
        .get("parent_session_id")
        .and_then(Value::as_str);
    let parent_session_value = parent_session_value.map(str::trim);

    parent_session_value.filter(|parent_session_id| !parent_session_id.is_empty())
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_scope_session_id(
    repo: &SessionRepository,
    approval_request: &ApprovalRequestRecord,
) -> Result<String, String> {
    let approval_runtime = OperatorApprovalRuntime::new(repo);
    let parent_session_id = approval_request_parent_session_id(approval_request);
    approval_runtime.grant_scope_session_id(&approval_request.session_id, parent_session_id)
}

#[cfg(feature = "memory-sqlite")]
async fn finish_approved_resolution(
    repo: &SessionRepository,
    runtime: &(dyn ApprovalResolutionRuntime + '_),
    approved: ApprovalRequestRecord,
) -> Result<ApprovalResolutionOutcome, String> {
    if !runtime.can_replay_approved_request() {
        return Ok(ApprovalResolutionOutcome {
            approval_request: approved,
            resumed_tool_output: None,
        });
    }

    let approval_request_id = approved.approval_request_id;
    execute_approved_request(repo, runtime, approval_request_id.as_str()).await
}

#[cfg(feature = "memory-sqlite")]
async fn execute_approved_request(
    repo: &SessionRepository,
    runtime: &(dyn ApprovalResolutionRuntime + '_),
    approval_request_id: &str,
) -> Result<ApprovalResolutionOutcome, String> {
    let executing = repo.transition_approval_request_if_current(
        approval_request_id,
        TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Approved,
            next_status: ApprovalRequestStatus::Executing,
            decision: None,
            resolved_by_session_id: None,
            executed_at: None,
            last_error: None,
        },
    )?;

    let Some(executing) = executing else {
        let error =
            format!("approval_request_not_approved: `{approval_request_id}` is no longer approved");
        return Err(error);
    };

    let replay_result = runtime.replay_approved_request(&executing).await;
    match replay_result {
        Ok(resumed_tool_output) => {
            let executed = repo.transition_approval_request_if_current(
                approval_request_id,
                TransitionApprovalRequestIfCurrentRequest {
                    expected_status: ApprovalRequestStatus::Executing,
                    next_status: ApprovalRequestStatus::Executed,
                    decision: None,
                    resolved_by_session_id: None,
                    executed_at: Some(unix_ts_now()),
                    last_error: None,
                },
            )?;

            let Some(executed) = executed else {
                let error = format!(
                    "approval_request_not_executing: `{approval_request_id}` is no longer executing"
                );
                return Err(error);
            };

            Ok(ApprovalResolutionOutcome {
                approval_request: executed,
                resumed_tool_output: Some(resumed_tool_output),
            })
        }
        Err(error) => {
            let executed = repo.transition_approval_request_if_current(
                approval_request_id,
                TransitionApprovalRequestIfCurrentRequest {
                    expected_status: ApprovalRequestStatus::Executing,
                    next_status: ApprovalRequestStatus::Executed,
                    decision: None,
                    resolved_by_session_id: None,
                    executed_at: Some(unix_ts_now()),
                    last_error: Some(error.clone()),
                },
            )?;

            if executed.is_none() {
                let combined_error = format!(
                    "approval_request_not_executing: `{approval_request_id}` is no longer executing; original replay error: {error}"
                );
                return Err(combined_error);
            }

            Err(error)
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn unix_ts_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(feature = "memory-sqlite")]
fn derive_attention_view(
    repo: &SessionRepository,
    record: &ApprovalRequestRecord,
) -> Result<DerivedAttentionView, String> {
    let approval_runtime = OperatorApprovalRuntime::new(repo);
    let mut execution_signals = Vec::new();
    match record.status {
        ApprovalRequestStatus::Pending => execution_signals.push(AttentionSignal {
            source: "execution",
            kind: "pending_operator_decision",
            severity: AttentionSeverity::Medium,
            action: "resolve_request",
            detail: Some("approval request is waiting for an operator decision".to_owned()),
        }),
        ApprovalRequestStatus::Approved | ApprovalRequestStatus::Executing => {
            execution_signals.push(AttentionSignal {
                source: "execution",
                kind: "resume_incomplete",
                severity: AttentionSeverity::High,
                action: "inspect_replay_state",
                detail: Some(
                    "approval request left the queue without reaching a terminal execution state"
                        .to_owned(),
                ),
            });
        }
        ApprovalRequestStatus::Executed if record.last_error.is_some() => {
            execution_signals.push(AttentionSignal {
                source: "execution",
                kind: "resumed_execution_failed",
                severity: AttentionSeverity::High,
                action: "inspect_failed_replay",
                detail: record.last_error.clone(),
            });
        }
        ApprovalRequestStatus::Executed
        | ApprovalRequestStatus::Denied
        | ApprovalRequestStatus::Expired
        | ApprovalRequestStatus::Cancelled => {}
    }

    let (grant_scope_session_id, grant_record) =
        approval_runtime.load_runtime_grant_for_request(record)?;
    let grant_age_seconds = grant_record
        .as_ref()
        .map(|grant| unix_ts_now().saturating_sub(grant.updated_at).max(0));

    let (grant_state, grant_signals) = match record.decision {
        Some(ApprovalDecision::ApproveAlways) => match (&grant_record, grant_age_seconds) {
            (None, _) => (
                GrantAttentionState::MissingGrant,
                vec![AttentionSignal {
                    source: "grant",
                    kind: "missing_runtime_grant",
                    severity: AttentionSeverity::High,
                    action: "repair_runtime_grant",
                    detail: Some(
                        "approve_always was recorded without a durable runtime grant".to_owned(),
                    ),
                }],
            ),
            (Some(_), Some(age_seconds))
                if age_seconds > APPROVAL_GRANT_REVIEW_STALE_AFTER_SECONDS =>
            {
                (
                    GrantAttentionState::ReviewStale,
                    vec![AttentionSignal {
                        source: "grant",
                        kind: "stale_runtime_grant_review",
                        severity: AttentionSeverity::Medium,
                        action: "review_runtime_grant",
                        detail: Some(format!(
                            "runtime grant review age {age_seconds}s exceeds {}s",
                            APPROVAL_GRANT_REVIEW_STALE_AFTER_SECONDS
                        )),
                    }],
                )
            }
            _ => (GrantAttentionState::Clean, Vec::new()),
        },
        _ => (GrantAttentionState::NotApplicable, Vec::new()),
    };

    Ok(DerivedAttentionView {
        execution_signals,
        grant_signals,
        grant_state,
        grant_scope_session_id: Some(grant_scope_session_id),
        grant_record,
        grant_age_seconds,
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_attention_summary_json(requests: &[ApprovalRequestView]) -> Value {
    let mut execution_only = 0usize;
    let mut grant_only = 0usize;
    let mut combined = 0usize;
    let mut none = 0usize;
    let mut reasons = std::collections::BTreeMap::<String, usize>::new();
    let mut actions = std::collections::BTreeMap::<String, usize>::new();
    let mut tools = std::collections::BTreeMap::<String, usize>::new();
    let mut sessions = std::collections::BTreeMap::<String, usize>::new();

    for request in requests {
        match request.attention.source_category() {
            "execution_only" => execution_only += 1,
            "grant_only" => grant_only += 1,
            "combined" => combined += 1,
            _ => none += 1,
        }
        if request.attention.needs_attention() {
            *tools.entry(request.record.tool_name.clone()).or_default() += 1;
            *sessions
                .entry(request.record.session_id.clone())
                .or_default() += 1;
        }
        for reason in request.attention.reason_kinds() {
            *reasons.entry(reason.to_owned()).or_default() += 1;
        }
        let mut request_actions = request
            .attention
            .combined_signals()
            .into_iter()
            .map(|signal| signal.action)
            .collect::<Vec<_>>();
        request_actions.sort_unstable();
        request_actions.dedup();
        for action in request_actions {
            *actions.entry(action.to_owned()).or_default() += 1;
        }
    }

    fn sorted_counts(counts: std::collections::BTreeMap<String, usize>, label: &str) -> Vec<Value> {
        let mut items: Vec<(String, usize)> = counts.into_iter().collect();
        items.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        items
            .into_iter()
            .map(|(value, count)| json!({ label: value, "count": count }))
            .collect()
    }

    json!({
        "needs_attention_count": requests
            .iter()
            .filter(|request| request.attention.needs_attention())
            .count(),
        "source_breakdown": {
            "execution_only": execution_only,
            "grant_only": grant_only,
            "combined": combined,
            "none": none,
        },
        "hotspots": {
            "by_reason": sorted_counts(reasons, "reason"),
            "by_action": sorted_counts(actions, "action"),
            "by_tool": sorted_counts(tools, "tool_name"),
            "by_session": sorted_counts(sessions, "session_id"),
        },
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_summary_json(view: &ApprovalRequestView) -> Value {
    let record = &view.record;
    let snapshot = &record.governance_snapshot_json;
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
        "policy_source": snapshot.get("policy_source").and_then(Value::as_str),
        "autonomy_profile": snapshot.get("autonomy_profile").and_then(Value::as_str),
        "capability_action_class": snapshot
            .get("capability_action_class")
            .and_then(Value::as_str),
        "decision_kind": snapshot.get("decision_kind").and_then(Value::as_str),
        "rule_id": snapshot.get("rule_id").and_then(Value::as_str),
        "reason_code": snapshot.get("reason_code").and_then(Value::as_str),
        "execution_integrity": view.attention.execution_integrity_json(),
        "grant_review": view.attention.grant_review_json(),
        "grant_attention": view.attention.grant_attention_json(),
        "attention": view.attention.attention_json(),
    })
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_detail_json(
    record: &ApprovalRequestRecord,
    attention: &DerivedAttentionView,
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
        "execution_integrity": attention.execution_integrity_json(),
        "grant_review": attention.grant_review_json(),
        "grant_attention": attention.grant_attention_json(),
        "attention": attention.attention_json(),
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
        grant_attention: optional_payload_grant_attention_filter(payload, "grant_attention")?,
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
    let approval_request_id =
        required_payload_string(payload, "approval_request_id", "approval tool")?;
    let decision_value = required_payload_string(payload, "decision", "approval tool")?;
    let decision = parse_approval_decision(&decision_value)?;
    let session_consent_mode = optional_payload_string(payload, "session_consent_mode")
        .map(|value| parse_session_consent_mode(value.as_str()))
        .transpose()?;

    if session_consent_mode.is_some() && decision != ApprovalDecision::ApproveOnce {
        return Err(
            "approval_request_resolve_invalid_request: session_consent_mode requires decision `approve_once`"
                .to_owned(),
        );
    }

    Ok(ApprovalRequestResolveRequest {
        approval_request_id,
        decision,
        session_consent_mode,
    })
}

#[cfg(feature = "memory-sqlite")]
fn parse_session_consent_mode(value: &str) -> Result<ToolConsentMode, String> {
    match value {
        "auto" => Ok(ToolConsentMode::Auto),
        "full" => Ok(ToolConsentMode::Full),
        _ => Err(format!(
            "approval_request_resolve_invalid_request: unknown session_consent_mode `{value}`"
        )),
    }
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
            current_session_id == target_session_id
                || repo.is_session_visible(current_session_id, target_session_id)?
        }
    };
    if is_visible {
        return Ok(());
    }
    Err(format!(
        "visibility_denied: session `{target_session_id}` is not visible from `{current_session_id}`"
    ))
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
fn optional_payload_grant_attention_filter(
    payload: &Value,
    field: &str,
) -> Result<Option<GrantAttentionFilter>, String> {
    optional_payload_string(payload, field)
        .map(|value| parse_grant_attention_filter(value.as_str()))
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
fn parse_grant_attention_filter(value: &str) -> Result<GrantAttentionFilter, String> {
    match value {
        "needs_attention" => Ok(GrantAttentionFilter::NeedsAttention),
        "missing_grant" => Ok(GrantAttentionFilter::MissingGrant),
        "review_stale" => Ok(GrantAttentionFilter::ReviewStale),
        "clean" => Ok(GrantAttentionFilter::Clean),
        "not_applicable" => Ok(GrantAttentionFilter::NotApplicable),
        _ => Err(format!(
            "approval_requests_list_invalid_request: unknown grant_attention `{value}`"
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
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use loong_contracts::ToolCoreOutcome;
    use loong_contracts::ToolCoreRequest;
    #[cfg(feature = "memory-sqlite")]
    use rusqlite::{Connection, params};
    use serde_json::Value;
    use serde_json::json;

    use super::*;
    use crate::config::ToolConfig;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        ApprovalDecision, ApprovalRequestStatus, NewApprovalGrantRecord, NewApprovalRequestRecord,
        NewSessionRecord, SessionKind, SessionRepository, SessionState,
        TransitionApprovalRequestIfCurrentRequest,
    };

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loong-approval-tools-{test_name}-{}",
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
            }),
        })
        .expect("seed approval request");
    }

    #[cfg(feature = "memory-sqlite")]
    fn approve_request(
        repo: &SessionRepository,
        approval_request_id: &str,
        decision: ApprovalDecision,
        resolved_by_session_id: &str,
    ) {
        repo.transition_approval_request_if_current(
            approval_request_id,
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Approved,
                decision: Some(decision),
                resolved_by_session_id: Some(resolved_by_session_id.to_owned()),
                executed_at: None,
                last_error: None,
            },
        )
        .expect("approve request")
        .expect("approval request should be pending");
    }

    #[cfg(feature = "memory-sqlite")]
    fn mark_request_executed(
        repo: &SessionRepository,
        approval_request_id: &str,
        last_error: Option<&str>,
    ) {
        repo.transition_approval_request_if_current(
            approval_request_id,
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Approved,
                next_status: ApprovalRequestStatus::Executed,
                decision: None,
                resolved_by_session_id: None,
                executed_at: Some(1),
                last_error: last_error.map(str::to_owned),
            },
        )
        .expect("mark request executed")
        .expect("approval request should be approved");
    }

    #[cfg(feature = "memory-sqlite")]
    fn seed_runtime_grant(repo: &SessionRepository, scope_session_id: &str, approval_key: &str) {
        repo.upsert_approval_grant(NewApprovalGrantRecord {
            scope_session_id: scope_session_id.to_owned(),
            approval_key: approval_key.to_owned(),
            created_by_session_id: Some(scope_session_id.to_owned()),
        })
        .expect("seed runtime grant");
    }

    #[cfg(feature = "memory-sqlite")]
    fn age_runtime_grant(
        config: &MemoryRuntimeConfig,
        scope_session_id: &str,
        approval_key: &str,
        updated_at: i64,
    ) {
        let db_path = config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .to_path_buf();
        let conn = Connection::open(db_path).expect("open sqlite connection");
        conn.execute(
            "UPDATE approval_grants
             SET created_at = ?1, updated_at = ?1
             WHERE scope_session_id = ?2 AND approval_key = ?3",
            params![updated_at, scope_session_id, approval_key],
        )
        .expect("age runtime grant");
    }

    #[cfg(feature = "memory-sqlite")]
    fn delete_session_row(config: &MemoryRuntimeConfig, session_id: &str) {
        let db_path = config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .to_path_buf();
        let conn = Connection::open(db_path).expect("open sqlite connection");

        conn.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id],
        )
        .expect("delete session row");
    }

    #[cfg(feature = "memory-sqlite")]
    #[derive(Clone)]
    struct MockApprovalResolutionRuntime {
        binding_error: Option<String>,
        can_replay: bool,
        replay_result: Result<ToolCoreOutcome, String>,
        replayed_request_ids: Arc<Mutex<Vec<String>>>,
    }

    #[cfg(feature = "memory-sqlite")]
    impl MockApprovalResolutionRuntime {
        fn succeeds_with(payload: Value) -> Self {
            let outcome = ToolCoreOutcome {
                status: "ok".to_owned(),
                payload,
            };
            Self {
                binding_error: None,
                can_replay: true,
                replay_result: Ok(outcome),
                replayed_request_ids: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn without_replay(mut self) -> Self {
            self.can_replay = false;
            self
        }

        fn replayed_request_ids(&self) -> Vec<String> {
            let guard = self
                .replayed_request_ids
                .lock()
                .expect("replayed request ids lock");
            guard.clone()
        }
    }

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl ApprovalResolutionRuntime for MockApprovalResolutionRuntime {
        fn can_replay_approved_request(&self) -> bool {
            self.can_replay
        }

        fn ensure_resolution_binding_allows_decision(
            &self,
            _approval_request: &ApprovalRequestRecord,
            _decision: ApprovalDecision,
        ) -> Result<(), String> {
            match &self.binding_error {
                Some(binding_error) => Err(binding_error.clone()),
                None => Ok(()),
            }
        }

        async fn replay_approved_request(
            &self,
            approval_request: &ApprovalRequestRecord,
        ) -> Result<ToolCoreOutcome, String> {
            let approval_request_id = approval_request.approval_request_id.clone();
            let mut guard = self
                .replayed_request_ids
                .lock()
                .expect("replayed request ids lock");
            guard.push(approval_request_id);
            drop(guard);

            self.replay_result.clone()
        }
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
        assert_eq!(
            outcome.payload["attention_summary"]["needs_attention_count"],
            2
        );
        assert_eq!(
            outcome.payload["attention_summary"]["source_breakdown"]["execution_only"],
            2
        );
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
        assert_eq!(
            requests[0]["attention"]["signals"][0]["source"],
            "execution"
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
            "governed_tool_requires_approval",
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
            "governed_tool_requires_approval"
        );
        assert_eq!(request["request_payload"]["tool_name"], "delegate_async");
        assert_eq!(
            request["request_payload"]["args_json"]["task"],
            "run-apr-child-visible"
        );
        assert_eq!(request["execution_integrity"]["state"], "pending_decision");
        assert_eq!(request["grant_review"]["state"], "not_applicable");
        assert_eq!(request["attention"]["sources"], json!(["execution"]));
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

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_attention_status_exposes_source_tagged_signals() {
        let config = isolated_memory_config("approval-attention-status");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-attention-status",
            "root-session",
            "delegate",
            "governed_tool_requires_approval",
        );
        approve_request(
            &repo,
            "apr-attention-status",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(
            &repo,
            "apr-attention-status",
            Some("delegate replay failed"),
        );

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-attention-status",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        let request = &outcome.payload["approval_request"];
        assert_eq!(request["execution_integrity"]["state"], "resume_failed");
        assert_eq!(request["grant_review"]["state"], "missing_grant");
        assert_eq!(request["grant_attention"]["needs_attention"], true);
        assert_eq!(request["attention"]["needs_attention"], true);
        assert_eq!(
            request["attention"]["sources"],
            json!(["execution", "grant"])
        );
        let sources = request["attention"]["signals"]
            .as_array()
            .expect("attention signals");
        assert!(sources.iter().any(|signal| signal["source"] == "execution"));
        assert!(sources.iter().any(|signal| signal["source"] == "grant"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_attention_list_summarizes_execution_grant_and_combined_hotspots() {
        let config = isolated_memory_config("approval-attention-summary");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        seed_request(
            &repo,
            "apr-execution-only",
            "root-session",
            "delegate",
            "rule-execution",
        );

        seed_request(
            &repo,
            "apr-grant-only",
            "root-session",
            "delegate_async",
            "rule-grant-only",
        );
        approve_request(
            &repo,
            "apr-grant-only",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(&repo, "apr-grant-only", None);

        seed_request(
            &repo,
            "apr-combined",
            "root-session",
            "session_cancel",
            "rule-combined",
        );
        approve_request(
            &repo,
            "apr-combined",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(&repo, "apr-combined", Some("delegate replay failed"));

        seed_request(
            &repo,
            "apr-clean-grant",
            "root-session",
            "session_recover",
            "rule-clean",
        );
        approve_request(
            &repo,
            "apr-clean-grant",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(&repo, "apr-clean-grant", None);
        seed_runtime_grant(&repo, "root-session", "tool:session_recover");

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

        assert_eq!(
            outcome.payload["attention_summary"]["needs_attention_count"],
            3
        );
        assert_eq!(
            outcome.payload["attention_summary"]["source_breakdown"]["execution_only"],
            1
        );
        assert_eq!(
            outcome.payload["attention_summary"]["source_breakdown"]["grant_only"],
            1
        );
        assert_eq!(
            outcome.payload["attention_summary"]["source_breakdown"]["combined"],
            1
        );
        let reasons = outcome.payload["attention_summary"]["hotspots"]["by_reason"]
            .as_array()
            .expect("reason hotspots");
        assert!(
            reasons
                .iter()
                .any(|item| item["reason"] == "pending_operator_decision")
        );
        assert!(
            reasons
                .iter()
                .any(|item| item["reason"] == "missing_runtime_grant")
        );
        assert!(
            reasons
                .iter()
                .any(|item| item["reason"] == "resumed_execution_failed")
        );
        let actions = outcome.payload["attention_summary"]["hotspots"]["by_action"]
            .as_array()
            .expect("action hotspots");
        assert!(
            actions
                .iter()
                .any(|item| item["action"] == "resolve_request")
        );
        assert!(
            actions
                .iter()
                .any(|item| item["action"] == "repair_runtime_grant")
        );
        assert!(
            actions
                .iter()
                .any(|item| item["action"] == "inspect_failed_replay")
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_grant_attention_filter_selects_grant_side_requests() {
        let config = isolated_memory_config("approval-grant-filter");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        seed_request(
            &repo,
            "apr-execution-only",
            "root-session",
            "delegate",
            "rule-execution",
        );

        seed_request(
            &repo,
            "apr-grant-attention",
            "root-session",
            "delegate",
            "rule-grant-attention",
        );
        approve_request(
            &repo,
            "apr-grant-attention",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(&repo, "apr-grant-attention", None);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "grant_attention": "needs_attention"
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_requests_list outcome");

        assert_eq!(outcome.payload["matched_count"], 1);
        assert_eq!(
            outcome.payload["filter"]["grant_attention"],
            "needs_attention"
        );
        let requests = outcome.payload["requests"]
            .as_array()
            .expect("requests array");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["approval_request_id"], "apr-grant-attention");
        assert_eq!(requests[0]["grant_review"]["state"], "missing_grant");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_grant_attention_filter_rejects_unknown_values() {
        let config = isolated_memory_config("approval-grant-filter-invalid");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-invalid-filter",
            "root-session",
            "delegate",
            "rule-invalid-filter",
        );

        let error = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_requests_list".to_owned(),
                payload: json!({
                    "grant_attention": "unknown"
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect_err("invalid grant attention filter should fail");

        assert!(
            error.contains("unknown grant_attention `unknown`"),
            "expected invalid grant attention error, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_tool_query_list_surfaces_autonomy_fields_in_summary() {
        let config = isolated_memory_config("approval-summary-autonomy-fields");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);

        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-autonomy-summary".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-autonomy-summary".to_owned(),
            tool_call_id: "call-autonomy-summary".to_owned(),
            tool_name: "external_skills.install".to_owned(),
            approval_key: "tool:external_skills.install".to_owned(),
            request_payload_json: json!({
                "session_id": "root-session",
                "tool_name": "external_skills.install",
                "args_json": {
                    "path": "source/demo-skill"
                },
            }),
            governance_snapshot_json: json!({
                "policy_source": "autonomy_policy",
                "autonomy_profile": "guided_acquisition",
                "capability_action_class": "capability_install",
                "decision_kind": "approval_required",
                "rule_id": "autonomy_policy_capability_acquisition_requires_approval",
                "reason_code": "autonomy_policy_capability_acquisition_requires_approval",
                "reason": "operator approval required before running `external_skills.install` under `guided_acquisition` product mode",
            }),
        })
        .expect("seed approval request");

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
        assert_eq!(requests.len(), 1);

        let request = &requests[0];
        assert_eq!(request["policy_source"], "autonomy_policy");
        assert_eq!(request["autonomy_profile"], "guided_acquisition");
        assert_eq!(request["capability_action_class"], "capability_install");
        assert_eq!(request["decision_kind"], "approval_required");
        assert_eq!(
            request["rule_id"],
            "autonomy_policy_capability_acquisition_requires_approval"
        );
        assert_eq!(
            request["reason_code"],
            "autonomy_policy_capability_acquisition_requires_approval"
        );
        assert_eq!(
            request["reason"],
            "operator approval required before running `external_skills.install` under `guided_acquisition` product mode"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_attention_grant_review_marks_stale_runtime_grants() {
        let config = isolated_memory_config("approval-grant-review-stale");
        let repo = SessionRepository::new(&config).expect("repository");
        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-stale-grant",
            "root-session",
            "delegate",
            "rule-stale-grant",
        );
        approve_request(
            &repo,
            "apr-stale-grant",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(&repo, "apr-stale-grant", None);
        seed_runtime_grant(&repo, "root-session", "tool:delegate");
        age_runtime_grant(&config, "root-session", "tool:delegate", 0);

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-stale-grant",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        assert_eq!(
            outcome.payload["approval_request"]["grant_review"]["state"],
            "review_stale"
        );
        assert_eq!(
            outcome.payload["approval_request"]["grant_attention"]["needs_attention"],
            true
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_status_uses_request_session_scope_when_session_row_is_missing() {
        let config = isolated_memory_config("approval-grant-missing-session-row");
        let repo = SessionRepository::new(&config).expect("repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-missing-session-row",
            "root-session",
            "delegate",
            "rule-missing-session-row",
        );
        approve_request(
            &repo,
            "apr-missing-session-row",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(&repo, "apr-missing-session-row", None);
        seed_runtime_grant(&repo, "root-session", "tool:delegate");
        delete_session_row(&config, "root-session");

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-missing-session-row",
                }),
            },
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        let approval_request = &outcome.payload["approval_request"];
        let grant_review = &approval_request["grant_review"];
        let grant_attention = &approval_request["grant_attention"];

        assert_eq!(grant_review["state"], "clean");
        assert_eq!(grant_review["scope_session_id"], "root-session");
        assert_eq!(grant_review["grant_exists"], true);
        assert_eq!(grant_attention["needs_attention"], false);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn approval_request_status_uses_root_scope_for_child_request_when_root_row_is_missing() {
        let config = isolated_memory_config("approval-grant-missing-root-row-child-request");
        let repo = SessionRepository::new(&config).expect("repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );
        repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr-missing-root-row-child-request".to_owned(),
            session_id: "child-session".to_owned(),
            turn_id: "turn-apr-missing-root-row-child-request".to_owned(),
            tool_call_id: "call-apr-missing-root-row-child-request".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: json!({
                "session_id": "child-session",
                "parent_session_id": "root-session",
                "tool_name": "delegate",
                "args_json": {
                    "task": "run-apr-missing-root-row-child-request"
                },
            }),
            governance_snapshot_json: json!({
                "reason": "approval required for delegate",
                "rule_id": "rule-missing-root-row-child-request",
            }),
        })
        .expect("seed child approval request");
        approve_request(
            &repo,
            "apr-missing-root-row-child-request",
            ApprovalDecision::ApproveAlways,
            "root-session",
        );
        mark_request_executed(&repo, "apr-missing-root-row-child-request", None);
        seed_runtime_grant(&repo, "root-session", "tool:delegate");
        delete_session_row(&config, "root-session");

        let outcome = crate::tools::execute_app_tool_with_config(
            ToolCoreRequest {
                tool_name: "approval_request_status".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-missing-root-row-child-request",
                }),
            },
            "child-session",
            &config,
            &ToolConfig::default(),
        )
        .expect("approval_request_status outcome");

        let approval_request = &outcome.payload["approval_request"];
        let grant_review = &approval_request["grant_review"];
        let grant_attention = &approval_request["grant_attention"];

        assert_eq!(grant_review["state"], "clean");
        assert_eq!(grant_review["scope_session_id"], "root-session");
        assert_eq!(grant_review["grant_exists"], true);
        assert_eq!(grant_attention["needs_attention"], false);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_request_resolve_approve_once_transitions_and_replays_in_tools_runtime() {
        let config = isolated_memory_config("approval-resolve-once");
        let repo = SessionRepository::new(&config).expect("repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-resolve-once",
            "root-session",
            "delegate_async",
            "governed_tool_requires_approval",
        );

        let runtime = MockApprovalResolutionRuntime::succeeds_with(json!({
            "tool": "delegate_async",
            "ok": true,
        }));
        let outcome = resolve_approval_request_with_runtime(
            &config,
            &runtime,
            ApprovalResolutionRequest {
                current_session_id: "root-session".to_owned(),
                approval_request_id: "apr-resolve-once".to_owned(),
                decision: ApprovalDecision::ApproveOnce,
                session_consent_mode: None,
                visibility: SessionVisibility::Children,
            },
        )
        .await
        .expect("approval request resolve outcome");

        assert_eq!(
            outcome.approval_request.status,
            ApprovalRequestStatus::Executed
        );
        assert_eq!(
            outcome
                .resumed_tool_output
                .as_ref()
                .map(|outcome| outcome.payload["tool"].clone()),
            Some(json!("delegate_async"))
        );
        assert_eq!(
            runtime.replayed_request_ids(),
            vec!["apr-resolve-once".to_owned()]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_request_resolve_approve_always_persists_runtime_grant_without_session_row() {
        let config = isolated_memory_config("approval-resolve-always-missing-session-row");
        let repo = SessionRepository::new(&config).expect("repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-resolve-always",
            "root-session",
            "delegate",
            "governed_tool_requires_approval",
        );
        delete_session_row(&config, "root-session");

        let runtime = MockApprovalResolutionRuntime::succeeds_with(json!({
            "tool": "delegate",
            "ok": true,
        }))
        .without_replay();
        let outcome = resolve_approval_request_with_runtime(
            &config,
            &runtime,
            ApprovalResolutionRequest {
                current_session_id: "root-session".to_owned(),
                approval_request_id: "apr-resolve-always".to_owned(),
                decision: ApprovalDecision::ApproveAlways,
                session_consent_mode: None,
                visibility: SessionVisibility::Children,
            },
        )
        .await
        .expect("approval request resolve outcome");

        let grant = repo
            .load_approval_grant("root-session", "tool:delegate")
            .expect("load approval grant");

        assert_eq!(
            outcome.approval_request.status,
            ApprovalRequestStatus::Approved
        );
        assert!(
            grant.is_some(),
            "expected root-session grant to be persisted"
        );
        assert!(runtime.replayed_request_ids().is_empty());
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_request_resolve_approve_once_retries_existing_approved_request_and_persists_consent()
     {
        let config = isolated_memory_config("approval-resolve-existing-approved");
        let repo = SessionRepository::new(&config).expect("repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-resolve-retry",
            "root-session",
            "sessions_list",
            "governed_tool_requires_approval",
        );
        repo.transition_approval_request_if_current(
            "apr-resolve-retry",
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
        .expect("approval request should be pending");

        let runtime = MockApprovalResolutionRuntime::succeeds_with(json!({
            "tool": "sessions_list",
            "ok": true,
        }))
        .without_replay();
        let outcome = resolve_approval_request_with_runtime(
            &config,
            &runtime,
            ApprovalResolutionRequest {
                current_session_id: "root-session".to_owned(),
                approval_request_id: "apr-resolve-retry".to_owned(),
                decision: ApprovalDecision::ApproveOnce,
                session_consent_mode: Some(ToolConsentMode::Auto),
                visibility: SessionVisibility::Children,
            },
        )
        .await
        .expect("approval request resolve retry outcome");

        let stored = repo
            .load_session_tool_consent("root-session")
            .expect("load session tool consent")
            .expect("session tool consent row");

        assert_eq!(
            outcome.approval_request.status,
            ApprovalRequestStatus::Approved
        );
        assert_eq!(stored.mode, ToolConsentMode::Auto);
        assert!(runtime.replayed_request_ids().is_empty());
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_request_resolve_deny_stays_terminal_without_replay() {
        let config = isolated_memory_config("approval-resolve-deny");
        let repo = SessionRepository::new(&config).expect("repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_request(
            &repo,
            "apr-resolve-deny",
            "root-session",
            "delegate_async",
            "governed_tool_requires_approval",
        );

        let runtime = MockApprovalResolutionRuntime::succeeds_with(json!({
            "tool": "delegate_async",
            "ok": true,
        }));
        let outcome = resolve_approval_request_with_runtime(
            &config,
            &runtime,
            ApprovalResolutionRequest {
                current_session_id: "root-session".to_owned(),
                approval_request_id: "apr-resolve-deny".to_owned(),
                decision: ApprovalDecision::Deny,
                session_consent_mode: None,
                visibility: SessionVisibility::Children,
            },
        )
        .await
        .expect("approval request resolve outcome");

        assert_eq!(
            outcome.approval_request.status,
            ApprovalRequestStatus::Denied
        );
        assert!(outcome.resumed_tool_output.is_none());
        assert!(runtime.replayed_request_ids().is_empty());
    }
}
