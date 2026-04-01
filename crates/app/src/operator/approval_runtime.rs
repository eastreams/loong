#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::ToolCoreRequest;
#[cfg(feature = "memory-sqlite")]
use serde_json::{Value, json};
#[cfg(feature = "memory-sqlite")]
use sha2::{Digest, Sha256};

#[cfg(feature = "memory-sqlite")]
use crate::operator::session_graph::OperatorSessionGraph;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalDecision, ApprovalGrantRecord, ApprovalRequestRecord, ApprovalRequestStatus,
    NewApprovalGrantRecord, NewApprovalRequestRecord, NewSessionRecord, SessionKind,
    SessionRepository, SessionState, TransitionApprovalRequestIfCurrentRequest,
};

#[cfg(feature = "memory-sqlite")]
pub(crate) struct GovernedToolApprovalRequest<'a> {
    pub session_id: &'a str,
    pub parent_session_id: Option<&'a str>,
    pub turn_id: &'a str,
    pub tool_call_id: &'a str,
    pub tool_name: &'a str,
    pub args_json: Value,
    pub source: &'a str,
    pub governance_scope: &'a str,
    pub risk_class: &'a str,
    pub approval_mode: &'a str,
    pub reason: &'a str,
    pub rule_id: &'a str,
}

#[cfg(feature = "memory-sqlite")]
pub(crate) struct OperatorApprovalRuntime<'a> {
    repo: &'a SessionRepository,
    session_graph: OperatorSessionGraph<'a>,
}

#[cfg(feature = "memory-sqlite")]
impl<'a> OperatorApprovalRuntime<'a> {
    pub(crate) fn new(repo: &'a SessionRepository) -> Self {
        let session_graph = OperatorSessionGraph::new(repo);

        Self {
            repo,
            session_graph,
        }
    }

    pub(crate) fn approval_key_for_tool_name(tool_name: &str) -> String {
        format!("tool:{tool_name}")
    }

    pub(crate) fn governed_approval_request_id(
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
    ) -> String {
        let mut hasher = Sha256::new();

        hasher.update(session_id.as_bytes());
        hasher.update([0]);
        hasher.update(turn_id.as_bytes());
        hasher.update([0]);
        hasher.update(tool_call_id.as_bytes());
        hasher.update([0]);
        hasher.update(tool_name.as_bytes());

        let digest = hasher.finalize();
        let request_id = format!("apr_{digest:x}");

        request_id
    }

    pub(crate) fn ensure_governed_tool_approval_request(
        &self,
        request: GovernedToolApprovalRequest<'_>,
    ) -> Result<ApprovalRequestRecord, String> {
        self.ensure_session_boundary(request.session_id, request.parent_session_id)?;

        let approval_key = Self::approval_key_for_tool_name(request.tool_name);
        let approval_request_id = Self::governed_approval_request_id(
            request.session_id,
            request.turn_id,
            request.tool_call_id,
            request.tool_name,
        );
        let request_payload_json = json!({
            "session_id": request.session_id,
            "parent_session_id": request.parent_session_id,
            "turn_id": request.turn_id,
            "tool_call_id": request.tool_call_id,
            "tool_name": request.tool_name,
            "args_json": request.args_json,
            "source": request.source,
            "execution_kind": "app",
        });
        let governance_snapshot_json = json!({
            "governance_scope": request.governance_scope,
            "risk_class": request.risk_class,
            "approval_mode": request.approval_mode,
            "rule_id": request.rule_id,
            "reason": request.reason,
        });
        let approval_request_record = NewApprovalRequestRecord {
            approval_request_id,
            session_id: request.session_id.to_owned(),
            turn_id: request.turn_id.to_owned(),
            tool_call_id: request.tool_call_id.to_owned(),
            tool_name: request.tool_name.to_owned(),
            approval_key,
            request_payload_json,
            governance_snapshot_json,
        };

        self.repo.ensure_approval_request(approval_request_record)
    }

    pub(crate) fn ensure_request_session(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<(), String> {
        let parent_session_id = Self::request_parent_session_id(approval_request);

        self.ensure_session_boundary(&approval_request.session_id, parent_session_id)
    }

    pub(crate) fn load_runtime_grant_for_context(
        &self,
        session_id: &str,
        parent_session_id: Option<&str>,
        approval_key: &str,
    ) -> Result<Option<ApprovalGrantRecord>, String> {
        let grant_scope_session_id = self.grant_scope_session_id(session_id, parent_session_id)?;
        let runtime_grant = self
            .repo
            .load_approval_grant(&grant_scope_session_id, approval_key)?;

        Ok(runtime_grant)
    }

    pub(crate) fn grant_scope_session_id(
        &self,
        session_id: &str,
        parent_session_id: Option<&str>,
    ) -> Result<String, String> {
        self.session_graph
            .effective_lineage_root_session_id(session_id, parent_session_id)
    }

    pub(crate) fn load_runtime_grant_for_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<(String, Option<ApprovalGrantRecord>), String> {
        let parent_session_id = Self::request_parent_session_id(approval_request);
        let grant_scope_session_id =
            self.grant_scope_session_id(&approval_request.session_id, parent_session_id)?;
        let runtime_grant = self
            .repo
            .load_approval_grant(&grant_scope_session_id, &approval_request.approval_key)?;

        Ok((grant_scope_session_id, runtime_grant))
    }

    pub(crate) fn upsert_runtime_grant_for_request(
        &self,
        approval_request: &ApprovalRequestRecord,
        created_by_session_id: Option<&str>,
    ) -> Result<ApprovalGrantRecord, String> {
        let parent_session_id = Self::request_parent_session_id(approval_request);
        let scope_session_id =
            self.grant_scope_session_id(&approval_request.session_id, parent_session_id)?;
        let grant_record = NewApprovalGrantRecord {
            scope_session_id,
            approval_key: approval_request.approval_key.clone(),
            created_by_session_id: created_by_session_id.map(str::to_owned),
        };

        self.repo.upsert_approval_grant(grant_record)
    }

    pub(crate) fn resolve_pending_request(
        &self,
        approval_request_id: &str,
        decision: ApprovalDecision,
        current_session_id: &str,
    ) -> Result<ApprovalRequestRecord, String> {
        let next_status = Self::next_status_for_decision(decision);
        let transition_request = TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Pending,
            next_status,
            decision: Some(decision),
            resolved_by_session_id: Some(current_session_id.to_owned()),
            executed_at: None,
            last_error: None,
        };

        let maybe_resolved = self
            .repo
            .transition_approval_request_if_current(approval_request_id, transition_request)?;
        let resolved = match maybe_resolved {
            Some(resolved) => resolved,
            None => {
                return Err(self.pending_resolution_error(approval_request_id)?);
            }
        };

        if decision == ApprovalDecision::ApproveAlways {
            let _ = self.upsert_runtime_grant_for_request(&resolved, Some(current_session_id))?;
        }

        Ok(resolved)
    }

    pub(crate) fn replayable_tool_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<ToolCoreRequest, String> {
        let execution_kind = approval_request
            .request_payload_json
            .get("execution_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval_request_invalid_payload: missing execution_kind".to_owned())?;

        if execution_kind != "app" {
            return Err(format!(
                "approval_request_invalid_execution_kind: expected `app`, got `{execution_kind}`"
            ));
        }

        let tool_name_value = approval_request
            .request_payload_json
            .get("tool_name")
            .and_then(Value::as_str);
        let tool_name_value = tool_name_value.map(str::trim);
        let tool_name = tool_name_value
            .filter(|tool_name| !tool_name.is_empty())
            .ok_or_else(|| "approval_request_invalid_payload: missing tool_name".to_owned())?;
        let payload = approval_request
            .request_payload_json
            .get("args_json")
            .cloned()
            .ok_or_else(|| "approval_request_invalid_payload: missing args_json".to_owned())?;

        Ok(ToolCoreRequest {
            tool_name: tool_name.to_owned(),
            payload,
        })
    }

    pub(crate) fn begin_approved_request_execution(
        &self,
        approval_request_id: &str,
    ) -> Result<ApprovalRequestRecord, String> {
        let transition_request = TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Approved,
            next_status: ApprovalRequestStatus::Executing,
            decision: None,
            resolved_by_session_id: None,
            executed_at: None,
            last_error: None,
        };
        let maybe_executing = self
            .repo
            .transition_approval_request_if_current(approval_request_id, transition_request)?;
        let executing = maybe_executing.ok_or_else(|| {
            format!("approval_request_not_approved: `{approval_request_id}` is no longer approved")
        })?;

        Ok(executing)
    }

    pub(crate) fn finish_executing_request(
        &self,
        approval_request_id: &str,
        last_error: Option<&str>,
    ) -> Result<Option<ApprovalRequestRecord>, String> {
        let executed_at = Self::current_epoch_s();
        let transition_request = TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Executing,
            next_status: ApprovalRequestStatus::Executed,
            decision: None,
            resolved_by_session_id: None,
            executed_at: Some(executed_at),
            last_error: last_error.map(str::to_owned),
        };

        self.repo
            .transition_approval_request_if_current(approval_request_id, transition_request)
    }

    fn session_kind_for_parent(parent_session_id: Option<&str>) -> SessionKind {
        if parent_session_id.is_some() {
            return SessionKind::DelegateChild;
        }

        SessionKind::Root
    }

    fn ensure_session_boundary(
        &self,
        session_id: &str,
        parent_session_id: Option<&str>,
    ) -> Result<(), String> {
        let session_kind = Self::session_kind_for_parent(parent_session_id);
        let session_record = NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: session_kind,
            parent_session_id: parent_session_id.map(str::to_owned),
            label: None,
            state: SessionState::Ready,
        };

        let _ = self.repo.ensure_session(session_record)?;

        Ok(())
    }

    fn next_status_for_decision(decision: ApprovalDecision) -> ApprovalRequestStatus {
        match decision {
            ApprovalDecision::Deny => ApprovalRequestStatus::Denied,
            ApprovalDecision::ApproveOnce => ApprovalRequestStatus::Approved,
            ApprovalDecision::ApproveAlways => ApprovalRequestStatus::Approved,
        }
    }

    fn pending_resolution_error(&self, approval_request_id: &str) -> Result<String, String> {
        let latest_request = self.repo.load_approval_request(approval_request_id)?;
        let latest_request = latest_request
            .ok_or_else(|| format!("approval_request_not_found: `{approval_request_id}`"))?;
        let latest_status = latest_request.status.as_str();
        let error = format!(
            "approval_request_not_pending: `{approval_request_id}` is already {latest_status}"
        );

        Ok(error)
    }

    fn request_parent_session_id(approval_request: &ApprovalRequestRecord) -> Option<&str> {
        let parent_session_value = approval_request
            .request_payload_json
            .get("parent_session_id")
            .and_then(Value::as_str);
        let parent_session_value = parent_session_value.map(str::trim);

        parent_session_value.filter(|parent_session_id| !parent_session_id.is_empty())
    }

    fn current_epoch_s() -> i64 {
        let now = std::time::SystemTime::now();
        let unix_epoch = std::time::UNIX_EPOCH;
        let duration = now.duration_since(unix_epoch);
        let duration = duration.unwrap_or_default();
        let seconds = duration.as_secs();

        seconds as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{GovernedToolApprovalRequest, OperatorApprovalRuntime};

    use serde_json::json;

    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        ApprovalDecision, ApprovalRequestStatus, NewApprovalGrantRecord, NewSessionRecord,
        SessionKind, SessionRepository, SessionState,
    };

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let process_id = std::process::id();
        let temp_dir = std::env::temp_dir();
        let directory_name =
            format!("loongclaw-operator-approval-runtime-{test_name}-{process_id}");
        let base_dir = temp_dir.join(directory_name);
        let _ = std::fs::create_dir_all(&base_dir);

        let db_path = base_dir.join("memory.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    fn seed_session(
        repo: &SessionRepository,
        session_id: &str,
        kind: SessionKind,
        parent_session_id: Option<&str>,
    ) {
        let session_record = NewSessionRecord {
            session_id: session_id.to_owned(),
            kind,
            parent_session_id: parent_session_id.map(str::to_owned),
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        };

        repo.create_session(session_record).expect("create session");
    }

    fn delete_session_row(memory_config: &MemoryRuntimeConfig, session_id: &str) {
        let db_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .to_path_buf();
        let conn = rusqlite::Connection::open(db_path).expect("open sqlite connection");

        conn.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            rusqlite::params![session_id],
        )
        .expect("delete session row");
    }

    fn seed_runtime_grant(
        repo: &SessionRepository,
        scope_session_id: &str,
        approval_key: &str,
        created_by_session_id: Option<&str>,
    ) {
        let grant_record = NewApprovalGrantRecord {
            scope_session_id: scope_session_id.to_owned(),
            approval_key: approval_key.to_owned(),
            created_by_session_id: created_by_session_id.map(str::to_owned),
        };

        let _ = repo
            .upsert_approval_grant(grant_record)
            .expect("upsert runtime grant");
    }

    #[test]
    fn operator_approval_runtime_persists_governed_tool_request() {
        let memory_config = isolated_memory_config("persist-request");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");
        let approval_runtime = OperatorApprovalRuntime::new(&repo);

        let request = GovernedToolApprovalRequest {
            session_id: "root-session",
            parent_session_id: None,
            turn_id: "turn-1",
            tool_call_id: "call-1",
            tool_name: "delegate",
            args_json: json!({
                "task": "run delegate task"
            }),
            source: "assistant",
            governance_scope: "session",
            risk_class: "high",
            approval_mode: "policy_driven",
            reason: "operator approval required before running `delegate`",
            rule_id: "governed_tool_requires_approval",
        };

        let stored = approval_runtime
            .ensure_governed_tool_approval_request(request)
            .expect("persist approval request");

        assert_eq!(stored.session_id, "root-session");
        assert_eq!(stored.tool_name, "delegate");
        assert_eq!(stored.status, ApprovalRequestStatus::Pending);
        assert_eq!(stored.approval_key, "tool:delegate");
    }

    #[test]
    fn operator_approval_runtime_loads_runtime_grant_from_lineage_root_scope() {
        let memory_config = isolated_memory_config("grant-scope");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );

        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        seed_runtime_grant(&repo, "root-session", "tool:delegate", Some("root-session"));

        let runtime_grant = approval_runtime
            .load_runtime_grant_for_context("child-session", Some("root-session"), "tool:delegate")
            .expect("load runtime grant")
            .expect("runtime grant");

        assert_eq!(runtime_grant.scope_session_id, "root-session");
        assert_eq!(runtime_grant.approval_key, "tool:delegate");
    }

    #[test]
    fn operator_approval_runtime_approve_always_creates_runtime_grant() {
        let memory_config = isolated_memory_config("approve-always");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );

        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let request = GovernedToolApprovalRequest {
            session_id: "child-session",
            parent_session_id: Some("root-session"),
            turn_id: "turn-1",
            tool_call_id: "call-1",
            tool_name: "delegate",
            args_json: json!({
                "task": "run delegate task"
            }),
            source: "assistant",
            governance_scope: "session",
            risk_class: "high",
            approval_mode: "policy_driven",
            reason: "operator approval required before running `delegate`",
            rule_id: "governed_tool_requires_approval",
        };

        let stored = approval_runtime
            .ensure_governed_tool_approval_request(request)
            .expect("persist approval request");
        let resolved = approval_runtime
            .resolve_pending_request(
                &stored.approval_request_id,
                ApprovalDecision::ApproveAlways,
                "root-session",
            )
            .expect("resolve approval request");

        assert_eq!(resolved.status, ApprovalRequestStatus::Approved);

        let runtime_grant = approval_runtime
            .load_runtime_grant_for_context("child-session", Some("root-session"), "tool:delegate")
            .expect("load runtime grant")
            .expect("runtime grant");

        assert_eq!(runtime_grant.scope_session_id, "root-session");
    }

    #[test]
    fn operator_approval_runtime_uses_request_session_scope_when_session_row_is_missing() {
        let memory_config = isolated_memory_config("missing-session-row");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);

        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        seed_runtime_grant(&repo, "root-session", "tool:delegate", Some("root-session"));

        delete_session_row(&memory_config, "root-session");

        let grant_scope_session_id = approval_runtime
            .grant_scope_session_id("root-session", None)
            .expect("load grant scope session id");
        let runtime_grant = approval_runtime
            .load_runtime_grant_for_context("root-session", None, "tool:delegate")
            .expect("load runtime grant")
            .expect("runtime grant");

        assert_eq!(grant_scope_session_id, "root-session");
        assert_eq!(runtime_grant.scope_session_id, "root-session");
    }

    #[test]
    fn operator_approval_runtime_uses_request_parent_scope_when_child_row_is_missing() {
        let memory_config = isolated_memory_config("missing-child-session-row");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);
        seed_session(
            &repo,
            "child-session",
            SessionKind::DelegateChild,
            Some("root-session"),
        );

        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let request = GovernedToolApprovalRequest {
            session_id: "child-session",
            parent_session_id: Some("root-session"),
            turn_id: "turn-1",
            tool_call_id: "call-1",
            tool_name: "delegate",
            args_json: json!({
                "task": "run delegate task"
            }),
            source: "assistant",
            governance_scope: "session",
            risk_class: "high",
            approval_mode: "policy_driven",
            reason: "operator approval required before running `delegate`",
            rule_id: "governed_tool_requires_approval",
        };
        let stored = approval_runtime
            .ensure_governed_tool_approval_request(request)
            .expect("persist approval request");

        delete_session_row(&memory_config, "child-session");

        let resolved = approval_runtime
            .resolve_pending_request(
                &stored.approval_request_id,
                ApprovalDecision::ApproveAlways,
                "root-session",
            )
            .expect("resolve approval request");

        assert_eq!(resolved.status, ApprovalRequestStatus::Approved);

        let grant_lookup = approval_runtime
            .load_runtime_grant_for_request(&resolved)
            .expect("load runtime grant for request");
        let runtime_grant = grant_lookup.1.expect("runtime grant");

        assert_eq!(runtime_grant.scope_session_id, "root-session");
    }

    #[test]
    fn operator_approval_runtime_persists_grant_when_scope_session_row_is_missing() {
        let memory_config = isolated_memory_config("missing-scope-session-row");
        let repo = SessionRepository::new(&memory_config).expect("create session repository");

        seed_session(&repo, "root-session", SessionKind::Root, None);

        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let request = GovernedToolApprovalRequest {
            session_id: "root-session",
            parent_session_id: None,
            turn_id: "turn-1",
            tool_call_id: "call-1",
            tool_name: "delegate",
            args_json: json!({
                "task": "run delegate task"
            }),
            source: "assistant",
            governance_scope: "session",
            risk_class: "high",
            approval_mode: "policy_driven",
            reason: "operator approval required before running `delegate`",
            rule_id: "governed_tool_requires_approval",
        };
        let stored = approval_runtime
            .ensure_governed_tool_approval_request(request)
            .expect("persist approval request");

        delete_session_row(&memory_config, "root-session");

        let resolved = approval_runtime
            .resolve_pending_request(
                &stored.approval_request_id,
                ApprovalDecision::ApproveAlways,
                "root-session",
            )
            .expect("resolve approval request");
        let runtime_grant = approval_runtime
            .load_runtime_grant_for_request(&resolved)
            .expect("load runtime grant for request")
            .1
            .expect("runtime grant");

        assert_eq!(runtime_grant.scope_session_id, "root-session");
    }
}
