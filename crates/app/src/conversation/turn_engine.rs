use async_trait::async_trait;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(feature = "memory-sqlite")]
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use loongclaw_contracts::{Capability, KernelError, ToolCoreOutcome, ToolCoreRequest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{
    GovernedToolApprovalMode, GovernedToolApprovalStrategy, LoongClawConfig, SessionVisibility,
    ToolConfig,
};
use crate::context::KernelContext;
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::tools::{
    delegate_child_tool_view_for_config, delegate_child_tool_view_for_config_with_delegate,
    runtime_tool_view, runtime_tool_view_for_config, tool_catalog, ToolApprovalMode,
    ToolDescriptor, ToolExecutionPlane, ToolGovernanceScope, ToolRiskClass, ToolView,
};

use super::persistence::{
    persist_tool_decision, persist_tool_decision_with_memory_config, persist_tool_outcome,
    persist_tool_outcome_with_memory_config,
};
use super::runtime::{ConversationRuntime, SessionContext};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalDecision, ApprovalRequestStatus, NewApprovalGrantRecord, NewApprovalRequestRecord,
    NewSessionEvent, NewSessionRecord, SessionKind, SessionRepository, SessionState,
    TransitionApprovalRequestIfCurrentRequest,
};

pub use crate::tools::delegate::{AsyncDelegateSpawnRequest, AsyncDelegateSpawner};
#[cfg(feature = "memory-sqlite")]
const TOOL_APPROVAL_RESOLVED_EVENT_KIND: &str = "tool_approval_resolved";
#[cfg(feature = "memory-sqlite")]
const TOOL_APPROVAL_EXECUTION_STARTED_EVENT_KIND: &str = "tool_approval_execution_started";
#[cfg(feature = "memory-sqlite")]
const TOOL_APPROVAL_EXECUTION_FINISHED_EVENT_KIND: &str = "tool_approval_execution_finished";
#[cfg(feature = "memory-sqlite")]
const TOOL_APPROVAL_EXECUTION_FAILED_EVENT_KIND: &str = "tool_approval_execution_failed";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub assistant_text: String,
    pub tool_intents: Vec<ToolIntent>,
    pub raw_meta: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIntent {
    pub tool_name: String,
    pub args_json: serde_json::Value,
    pub source: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolGovernanceSnapshot {
    pub execution_plane: ToolExecutionPlane,
    pub governance_scope: ToolGovernanceScope,
    pub risk_class: ToolRiskClass,
    pub approval_mode: ToolApprovalMode,
    pub audit_label: String,
    pub reason: String,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecision {
    pub allow: bool,
    pub deny: bool,
    pub approval_required: bool,
    pub reason: String,
    pub rule_id: String,
    pub governance: ToolGovernanceSnapshot,
}

impl ToolDecision {
    fn from_governance(decision: &ToolGovernanceDecision) -> Self {
        Self {
            allow: decision.allow,
            deny: !decision.allow && !decision.approval_required,
            approval_required: decision.approval_required,
            reason: decision.reason.clone(),
            rule_id: decision.rule_id.clone(),
            governance: decision.snapshot(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: String,
    pub payload: serde_json::Value,
    pub error_code: Option<String>,
    pub human_reason: Option<String>,
    pub audit_event_id: Option<String>,
    pub governance_allowed: bool,
    pub governance: Option<ToolGovernanceSnapshot>,
}

impl ToolOutcome {
    fn governed(
        status: impl Into<String>,
        payload: serde_json::Value,
        error_code: Option<String>,
        human_reason: Option<String>,
        audit_event_id: Option<String>,
        governance_allowed: bool,
        decision: &ToolGovernanceDecision,
    ) -> Self {
        Self {
            status: status.into(),
            payload,
            error_code,
            human_reason,
            audit_event_id,
            governance_allowed,
            governance: Some(decision.snapshot()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolGovernanceDecision {
    pub allow: bool,
    pub approval_required: bool,
    pub reason: String,
    pub rule_id: String,
    pub execution_plane: ToolExecutionPlane,
    pub governance_scope: ToolGovernanceScope,
    pub risk_class: ToolRiskClass,
    pub approval_mode: ToolApprovalMode,
    pub audit_label: &'static str,
}

impl ToolGovernanceDecision {
    pub fn snapshot(&self) -> ToolGovernanceSnapshot {
        ToolGovernanceSnapshot {
            execution_plane: self.execution_plane,
            governance_scope: self.governance_scope,
            risk_class: self.risk_class,
            approval_mode: self.approval_mode,
            audit_label: self.audit_label.to_owned(),
            reason: self.reason.clone(),
            rule_id: self.rule_id.clone(),
        }
    }

    pub fn allow(descriptor: &ToolDescriptor) -> Self {
        Self {
            allow: true,
            approval_required: false,
            reason: "governance_allow".to_owned(),
            rule_id: "default_governance_allow".to_owned(),
            execution_plane: descriptor.execution_plane,
            governance_scope: descriptor.governance_scope,
            risk_class: descriptor.risk_class,
            approval_mode: descriptor.approval_mode,
            audit_label: descriptor.audit_label,
        }
    }

    pub fn allow_with_rule(
        descriptor: &ToolDescriptor,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            allow: true,
            approval_required: false,
            reason: reason.into(),
            rule_id: rule_id.into(),
            execution_plane: descriptor.execution_plane,
            governance_scope: descriptor.governance_scope,
            risk_class: descriptor.risk_class,
            approval_mode: descriptor.approval_mode,
            audit_label: descriptor.audit_label,
        }
    }

    pub fn deny(
        descriptor: &ToolDescriptor,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            allow: false,
            approval_required: false,
            reason: reason.into(),
            rule_id: rule_id.into(),
            execution_plane: descriptor.execution_plane,
            governance_scope: descriptor.governance_scope,
            risk_class: descriptor.risk_class,
            approval_mode: descriptor.approval_mode,
            audit_label: descriptor.audit_label,
        }
    }

    pub fn require_approval(
        descriptor: &ToolDescriptor,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            allow: false,
            approval_required: true,
            reason: reason.into(),
            rule_id: rule_id.into(),
            execution_plane: descriptor.execution_plane,
            governance_scope: descriptor.governance_scope,
            risk_class: descriptor.risk_class,
            approval_mode: descriptor.approval_mode,
            audit_label: descriptor.audit_label,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirementKind {
    KernelContextRequired,
    GovernedTool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequirement {
    pub kind: ApprovalRequirementKind,
    pub reason: String,
    pub rule_id: String,
    pub tool_name: Option<String>,
    pub approval_key: Option<String>,
    pub approval_request_id: Option<String>,
}

impl ApprovalRequirement {
    pub fn kernel_context_required() -> Self {
        Self {
            kind: ApprovalRequirementKind::KernelContextRequired,
            reason: "kernel_context_required".to_owned(),
            rule_id: "kernel_context_required".to_owned(),
            tool_name: None,
            approval_key: None,
            approval_request_id: None,
        }
    }

    pub fn governed_tool(
        tool_name: impl Into<String>,
        approval_key: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
        approval_request_id: Option<String>,
    ) -> Self {
        Self {
            kind: ApprovalRequirementKind::GovernedTool,
            reason: reason.into(),
            rule_id: rule_id.into(),
            tool_name: Some(tool_name.into()),
            approval_key: Some(approval_key.into()),
            approval_request_id,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TurnResult {
    FinalText(String),
    NeedsApproval(ApprovalRequirement),
    ToolDenied(String),
    ToolError(String),
    ProviderError(String),
}

#[async_trait]
pub trait AppToolDispatcher: Send + Sync {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String>;
}

#[async_trait]
pub trait OrchestrationToolDispatcher: Send + Sync {
    async fn execute_orchestration_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String>;
}

#[async_trait]
pub(crate) trait ApprovalOrchestrationReplayer: Send + Sync {
    async fn replay_orchestration_request(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String>;
}

#[async_trait]
pub trait ToolGovernanceEvaluator: Send + Sync {
    async fn evaluate_tool_governance(
        &self,
        descriptor: &ToolDescriptor,
        intent: &ToolIntent,
        session_context: &SessionContext,
        kernel_ctx: Option<&KernelContext>,
    ) -> ToolGovernanceDecision;
}

#[async_trait]
trait ToolLifecycleRecorder: Send + Sync {
    async fn persist_tool_decision(
        &self,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        decision: &ToolDecision,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<(), String>;

    async fn persist_tool_outcome(
        &self,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        outcome: &ToolOutcome,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<(), String>;
}

pub(crate) trait GovernedApprovalRequestStore: Send + Sync {
    fn ensure_governed_approval_request(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &ToolDescriptor,
        governance_decision: &ToolGovernanceDecision,
    ) -> Result<String, String>;
}

struct RuntimeToolLifecycleRecorder<'a, R: ConversationRuntime + ?Sized> {
    runtime: &'a R,
}

#[async_trait]
impl<R: ConversationRuntime + ?Sized> ToolLifecycleRecorder
    for RuntimeToolLifecycleRecorder<'_, R>
{
    async fn persist_tool_decision(
        &self,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        decision: &ToolDecision,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<(), String> {
        persist_tool_decision(
            self.runtime,
            session_id,
            turn_id,
            tool_call_id,
            decision,
            kernel_ctx,
        )
        .await
    }

    async fn persist_tool_outcome(
        &self,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        outcome: &ToolOutcome,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<(), String> {
        persist_tool_outcome(
            self.runtime,
            session_id,
            turn_id,
            tool_call_id,
            outcome,
            kernel_ctx,
        )
        .await
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub(crate) struct SessionRepositoryApprovalRequestStore {
    memory_config: MemoryRuntimeConfig,
}

#[cfg(feature = "memory-sqlite")]
impl SessionRepositoryApprovalRequestStore {
    pub(crate) fn new(memory_config: MemoryRuntimeConfig) -> Self {
        Self { memory_config }
    }
}

#[cfg(feature = "memory-sqlite")]
impl GovernedApprovalRequestStore for SessionRepositoryApprovalRequestStore {
    fn ensure_governed_approval_request(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &ToolDescriptor,
        governance_decision: &ToolGovernanceDecision,
    ) -> Result<String, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let kind = if session_context.parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        };
        let _ = repo.ensure_session(NewSessionRecord {
            session_id: session_context.session_id.clone(),
            kind,
            parent_session_id: session_context.parent_session_id.clone(),
            label: None,
            state: SessionState::Ready,
        })?;

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let request_payload_json = serde_json::json!({
            "session_id": session_context.session_id,
            "parent_session_id": session_context.parent_session_id,
            "turn_id": intent.turn_id,
            "tool_call_id": intent.tool_call_id,
            "tool_name": descriptor.name,
            "args_json": intent.args_json,
            "source": intent.source,
            "execution_plane": descriptor.execution_plane,
        });
        let governance_snapshot_json = serde_json::to_value(governance_decision.snapshot())
            .map_err(|error| format!("serialize governed approval snapshot failed: {error}"))?;
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: approval_request_id.clone(),
            session_id: session_context.session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
            tool_name: descriptor.name.to_owned(),
            approval_key: format!("tool:{}", descriptor.name),
            request_payload_json,
            governance_snapshot_json,
        })?;
        Ok(stored.approval_request_id)
    }
}

fn governed_approval_request_id(
    session_context: &SessionContext,
    tool_name: &str,
    intent: &ToolIntent,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(session_context.session_id.as_bytes());
    hasher.update([0]);
    hasher.update(intent.turn_id.as_bytes());
    hasher.update([0]);
    hasher.update(intent.tool_call_id.as_bytes());
    hasher.update([0]);
    hasher.update(tool_name.as_bytes());
    format!("apr_{:x}", hasher.finalize())
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct AsyncDelegateSubprocessPlan {
    program: PathBuf,
    args: Vec<String>,
}

#[cfg(feature = "memory-sqlite")]
fn build_async_delegate_subprocess_plan(
    executable_path: &Path,
    config_path: Option<&str>,
    request: &crate::tools::delegate::AsyncDelegateSpawnRequest,
) -> AsyncDelegateSubprocessPlan {
    let mut args = vec!["run-turn".to_owned()];
    if let Some(config_path) = config_path.map(str::trim).filter(|value| !value.is_empty()) {
        args.push("--config".to_owned());
        args.push(config_path.to_owned());
    }
    args.extend([
        "--session".to_owned(),
        request.child_session_id.clone(),
        "--input".to_owned(),
        request.task.clone(),
        "--timeout-seconds".to_owned(),
        request.timeout_seconds.to_string(),
        "--delegate-child".to_owned(),
    ]);
    AsyncDelegateSubprocessPlan {
        program: executable_path.to_path_buf(),
        args,
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
struct SubprocessAsyncDelegateSpawner {
    executable_path: PathBuf,
    config_path: Option<String>,
}

#[cfg(feature = "memory-sqlite")]
impl SubprocessAsyncDelegateSpawner {
    fn from_current_process() -> Result<Self, String> {
        let executable_path = std::env::current_exe().map_err(|error| {
            format!("resolve current executable for async delegate failed: {error}")
        })?;
        let config_path = std::env::var("LOONGCLAW_CONFIG_PATH")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Ok(Self {
            executable_path,
            config_path,
        })
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::tools::delegate::AsyncDelegateSpawner for SubprocessAsyncDelegateSpawner {
    async fn spawn(
        &self,
        request: crate::tools::delegate::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        let plan = build_async_delegate_subprocess_plan(
            &self.executable_path,
            self.config_path.as_deref(),
            &request,
        );
        let mut command = tokio::process::Command::new(&plan.program);
        command.args(&plan.args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|error| format!("spawn async delegate subprocess failed: {error}"))?;
        drop(child);
        Ok(())
    }
}

pub struct NoopAppToolDispatcher;

#[async_trait]
impl AppToolDispatcher for NoopAppToolDispatcher {
    async fn execute_app_tool(
        &self,
        _session_context: &SessionContext,
        request: ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        Err(format!("app_tool_not_implemented: {}", request.tool_name))
    }
}

pub struct NoopOrchestrationToolDispatcher;

#[async_trait]
impl OrchestrationToolDispatcher for NoopOrchestrationToolDispatcher {
    async fn execute_orchestration_tool(
        &self,
        _session_context: &SessionContext,
        request: ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        Err(format!(
            "orchestration_tool_not_implemented: {}",
            request.tool_name
        ))
    }
}

pub struct NoopToolGovernanceEvaluator;

#[async_trait]
impl ToolGovernanceEvaluator for NoopToolGovernanceEvaluator {
    async fn evaluate_tool_governance(
        &self,
        descriptor: &ToolDescriptor,
        _intent: &ToolIntent,
        _session_context: &SessionContext,
        _kernel_ctx: Option<&KernelContext>,
    ) -> ToolGovernanceDecision {
        ToolGovernanceDecision::allow(descriptor)
    }
}

#[derive(Debug, Clone)]
pub struct DefaultToolGovernanceEvaluator {
    tool_config: ToolConfig,
    memory_config: Option<MemoryRuntimeConfig>,
}

impl DefaultToolGovernanceEvaluator {
    pub fn new(tool_config: ToolConfig) -> Self {
        Self {
            tool_config,
            memory_config: None,
        }
    }

    pub fn with_memory_config(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        Self {
            tool_config,
            memory_config: Some(memory_config),
        }
    }

    fn approval_key(&self, descriptor: &ToolDescriptor) -> String {
        format!("tool:{}", descriptor.name)
    }

    fn one_time_full_access_state(&self, now_epoch_s: u64) -> (bool, Option<String>) {
        let approval = &self.tool_config.approval;
        if !approval.one_time_full_access_granted {
            return (false, None);
        }
        if let Some(deadline) = approval.one_time_full_access_expires_at_epoch_s {
            if now_epoch_s > deadline {
                return (
                    false,
                    Some(format!(
                        "one-time full access grant expired at {deadline}, now is {now_epoch_s}"
                    )),
                );
            }
        }
        if matches!(approval.one_time_full_access_remaining_uses, Some(0)) {
            return (
                false,
                Some("one-time full access grant has no remaining uses".to_owned()),
            );
        }
        (true, None)
    }

    #[cfg(feature = "memory-sqlite")]
    fn runtime_grant_scope_session_id(
        &self,
        session_id: &str,
        approval_key: &str,
    ) -> Result<Option<String>, String> {
        let memory_config = match &self.memory_config {
            Some(memory_config) => memory_config,
            None => return Ok(None),
        };
        let repo = SessionRepository::new(memory_config)?;
        let Some(scope_session_id) = repo.lineage_root_session_id(session_id)? else {
            return Ok(None);
        };
        let grant = repo.load_approval_grant(&scope_session_id, approval_key)?;
        Ok(grant.map(|_| scope_session_id))
    }
}

impl Default for DefaultToolGovernanceEvaluator {
    fn default() -> Self {
        Self::new(ToolConfig::default())
    }
}

#[async_trait]
impl ToolGovernanceEvaluator for DefaultToolGovernanceEvaluator {
    async fn evaluate_tool_governance(
        &self,
        descriptor: &ToolDescriptor,
        _intent: &ToolIntent,
        session_context: &SessionContext,
        _kernel_ctx: Option<&KernelContext>,
    ) -> ToolGovernanceDecision {
        if descriptor.execution_plane == ToolExecutionPlane::Core
            || descriptor.approval_mode != ToolApprovalMode::PolicyDriven
        {
            return ToolGovernanceDecision::allow(descriptor);
        }

        let approval = &self.tool_config.approval;
        let approval_key = self.approval_key(descriptor);
        if approval
            .denied_calls
            .iter()
            .any(|entry| entry == &approval_key)
        {
            return ToolGovernanceDecision::deny(
                descriptor,
                format!("governed tool {approval_key} is denied by approval policy"),
                "governed_tool_denied_call",
            );
        }

        if matches!(approval.mode, GovernedToolApprovalMode::Disabled) {
            return ToolGovernanceDecision::allow_with_rule(
                descriptor,
                format!(
                    "governed tool {approval_key} is allowed because approval mode is disabled"
                ),
                "governed_tool_approval_disabled",
            );
        }

        let approval_required = match approval.mode {
            GovernedToolApprovalMode::Disabled => false,
            GovernedToolApprovalMode::MediumBalanced => {
                matches!(descriptor.risk_class, ToolRiskClass::High)
            }
            GovernedToolApprovalMode::Strict => true,
        };

        if !approval_required {
            return ToolGovernanceDecision::allow_with_rule(
                descriptor,
                format!(
                    "governed tool {approval_key} is allowed by medium-balanced approval policy"
                ),
                "governed_tool_medium_balanced_allow",
            );
        }

        #[cfg(feature = "memory-sqlite")]
        match self.runtime_grant_scope_session_id(&session_context.session_id, &approval_key) {
            Ok(Some(scope_session_id)) => {
                return ToolGovernanceDecision::allow_with_rule(
                    descriptor,
                    format!(
                        "governed tool {approval_key} is allowed by runtime approval grant scoped to root session `{scope_session_id}`"
                    ),
                    "governed_tool_runtime_approval_grant",
                );
            }
            Ok(None) => {}
            Err(error) => {
                return ToolGovernanceDecision::require_approval(
                    descriptor,
                    format!(
                        "operator approval required for governed tool {approval_key}; runtime grant lookup failed: {error}"
                    ),
                    "governed_tool_runtime_grant_lookup_failed",
                );
            }
        }

        let now_epoch_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let (one_time_full_access_active, one_time_full_access_rejected_reason) =
            self.one_time_full_access_state(now_epoch_s);

        match approval.strategy {
            GovernedToolApprovalStrategy::PerCall
                if approval
                    .approved_calls
                    .iter()
                    .any(|entry| entry == &approval_key) =>
            {
                ToolGovernanceDecision::allow_with_rule(
                    descriptor,
                    format!("governed tool {approval_key} is pre-approved by approval policy"),
                    "governed_tool_preapproved_call",
                )
            }
            GovernedToolApprovalStrategy::OneTimeFullAccess if one_time_full_access_active => {
                ToolGovernanceDecision::allow_with_rule(
                    descriptor,
                    format!(
                        "governed tool {approval_key} is allowed by one-time full access grant"
                    ),
                    "governed_tool_one_time_full_access",
                )
            }
            GovernedToolApprovalStrategy::OneTimeFullAccess => {
                ToolGovernanceDecision::require_approval(
                    descriptor,
                    one_time_full_access_rejected_reason.unwrap_or_else(|| {
                        format!(
                            "operator approval required for governed tool {approval_key}; \
                             one-time full access is not granted"
                        )
                    }),
                    "governed_tool_requires_one_time_full_access",
                )
            }
            GovernedToolApprovalStrategy::PerCall => ToolGovernanceDecision::require_approval(
                descriptor,
                format!(
                    "operator approval required for governed tool {approval_key}; add it to \
                     tools.approval.approved_calls or enable one_time_full_access"
                ),
                "governed_tool_requires_per_call_approval",
            ),
        }
    }
}

pub(crate) fn effective_tool_config_for_session(
    tool_config: &ToolConfig,
    session_context: &SessionContext,
) -> ToolConfig {
    let mut tool_config = tool_config.clone();
    if session_context.parent_session_id.is_some() {
        tool_config.sessions.visibility = SessionVisibility::SelfOnly;
    }
    tool_config
}

#[cfg(feature = "memory-sqlite")]
fn effective_tool_view_for_session(
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    session_context: &SessionContext,
) -> Result<ToolView, String> {
    let repo = crate::session::repository::SessionRepository::new(memory_config)?;
    if let Some(session) = repo.load_session(&session_context.session_id)? {
        if session.parent_session_id.is_some() {
            let depth = repo
                .session_lineage_depth(&session_context.session_id)
                .map_err(|error| {
                    format!(
                        "compute session lineage depth for dispatcher tool view failed: {error}"
                    )
                })?;
            let allow_nested_delegate = depth < tool_config.delegate.max_depth;
            return Ok(delegate_child_tool_view_for_config_with_delegate(
                tool_config,
                allow_nested_delegate,
            ));
        }
        return Ok(runtime_tool_view_for_config(tool_config));
    }
    if repo
        .load_session_summary_with_legacy_fallback(&session_context.session_id)?
        .is_some_and(|session| {
            session.kind == crate::session::repository::SessionKind::DelegateChild
        })
    {
        return Ok(delegate_child_tool_view_for_config(tool_config));
    }
    Ok(runtime_tool_view_for_config(tool_config))
}

#[cfg(not(feature = "memory-sqlite"))]
fn effective_tool_view_for_session(
    _memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    _session_context: &SessionContext,
) -> Result<ToolView, String> {
    Ok(runtime_tool_view_for_config(tool_config))
}

#[derive(Clone)]
pub struct DefaultAppToolDispatcher {
    memory_config: MemoryRuntimeConfig,
    tool_config: ToolConfig,
    app_config: Option<Arc<LoongClawConfig>>,
    #[cfg(feature = "memory-sqlite")]
    approval_runtime: Option<Arc<dyn crate::tools::approval::ApprovalResolutionRuntime>>,
}

impl DefaultAppToolDispatcher {
    pub fn new(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        Self {
            #[cfg(feature = "memory-sqlite")]
            approval_runtime: default_approval_runtime(&memory_config, &tool_config, None, None),
            memory_config,
            tool_config,
            app_config: None,
        }
    }

    pub fn with_config(memory_config: MemoryRuntimeConfig, config: LoongClawConfig) -> Self {
        Self {
            #[cfg(feature = "memory-sqlite")]
            approval_runtime: default_approval_runtime(
                &memory_config,
                &config.tools,
                Some(Arc::new(config.clone())),
                None,
            ),
            memory_config,
            tool_config: config.tools.clone(),
            app_config: Some(Arc::new(config)),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    pub fn production(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        Self {
            approval_runtime: default_approval_runtime(&memory_config, &tool_config, None, None),
            memory_config,
            tool_config,
            app_config: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    pub fn production_with_config(
        memory_config: MemoryRuntimeConfig,
        config: LoongClawConfig,
    ) -> Self {
        Self {
            approval_runtime: default_approval_runtime(
                &memory_config,
                &config.tools,
                Some(Arc::new(config.clone())),
                None,
            ),
            memory_config,
            tool_config: config.tools.clone(),
            app_config: Some(Arc::new(config)),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    pub fn with_approval_resolution_orchestration_dispatcher(
        mut self,
        orchestration_dispatcher: Arc<dyn OrchestrationToolDispatcher>,
    ) -> Self {
        self.approval_runtime = default_approval_runtime(
            &self.memory_config,
            &self.tool_config,
            self.app_config.clone(),
            Some(orchestration_dispatcher),
        );
        self
    }

    pub fn runtime() -> Self {
        #[cfg(feature = "memory-sqlite")]
        {
            return Self::production(
                crate::memory::runtime_config::get_memory_runtime_config().clone(),
                ToolConfig::default(),
            );
        }
        #[cfg(not(feature = "memory-sqlite"))]
        Self::new(
            crate::memory::runtime_config::get_memory_runtime_config().clone(),
            ToolConfig::default(),
        )
    }
}

impl Default for DefaultAppToolDispatcher {
    fn default() -> Self {
        Self::runtime()
    }
}

#[async_trait]
impl AppToolDispatcher for DefaultAppToolDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        let canonical_tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
        let effective_tool_view = effective_tool_view_for_session(
            &self.memory_config,
            &self.tool_config,
            session_context,
        )?;
        if let Some(descriptor) = tool_catalog().descriptor(canonical_tool_name) {
            if descriptor.execution_plane == ToolExecutionPlane::App
                && (!session_context.tool_view.contains(descriptor.name)
                    || !effective_tool_view.contains(descriptor.name))
            {
                return Err(format!("tool_not_visible: {}", descriptor.name));
            }
        }
        let effective_tool_config =
            effective_tool_config_for_session(&self.tool_config, session_context);
        crate::tools::execute_app_tool_with_runtime_support(
            request,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
            crate::tools::AppToolRuntimeSupport {
                app_config: self.app_config.as_deref(),
                kernel_ctx,
                #[cfg(feature = "memory-sqlite")]
                approval_runtime: self.approval_runtime.clone(),
            },
        )
        .await
    }
}

#[cfg(feature = "memory-sqlite")]
fn default_approval_runtime(
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    app_config: Option<Arc<LoongClawConfig>>,
    orchestration_dispatcher: Option<Arc<dyn OrchestrationToolDispatcher>>,
) -> Option<Arc<dyn crate::tools::approval::ApprovalResolutionRuntime>> {
    Some(Arc::new(DefaultApprovalResolutionRuntime::new(
        memory_config.clone(),
        tool_config.clone(),
        app_config,
        orchestration_dispatcher,
    )))
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub(crate) struct DefaultApprovalResolutionRuntime {
    memory_config: MemoryRuntimeConfig,
    tool_config: ToolConfig,
    app_config: Option<Arc<LoongClawConfig>>,
    orchestration_dispatcher: Option<Arc<dyn OrchestrationToolDispatcher>>,
}

#[cfg(feature = "memory-sqlite")]
impl DefaultApprovalResolutionRuntime {
    pub(crate) fn new(
        memory_config: MemoryRuntimeConfig,
        tool_config: ToolConfig,
        app_config: Option<Arc<LoongClawConfig>>,
        orchestration_dispatcher: Option<Arc<dyn OrchestrationToolDispatcher>>,
    ) -> Self {
        Self {
            memory_config,
            tool_config,
            app_config,
            orchestration_dispatcher,
        }
    }

    fn append_approval_event(
        &self,
        repo: &SessionRepository,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
        actor_session_id: &str,
        event_kind: &str,
        extra_payload: serde_json::Value,
    ) -> Result<(), String> {
        let mut payload = serde_json::Map::new();
        payload.insert(
            "approval_request_id".to_owned(),
            serde_json::Value::String(approval_request.approval_request_id.clone()),
        );
        payload.insert(
            "tool_name".to_owned(),
            serde_json::Value::String(approval_request.tool_name.clone()),
        );
        payload.insert(
            "approval_key".to_owned(),
            serde_json::Value::String(approval_request.approval_key.clone()),
        );
        payload.insert(
            "status".to_owned(),
            serde_json::Value::String(approval_request.status.as_str().to_owned()),
        );
        if let Some(decision) = approval_request.decision {
            payload.insert(
                "decision".to_owned(),
                serde_json::Value::String(decision.as_str().to_owned()),
            );
        }
        payload.insert(
            "turn_id".to_owned(),
            serde_json::Value::String(approval_request.turn_id.clone()),
        );
        payload.insert(
            "tool_call_id".to_owned(),
            serde_json::Value::String(approval_request.tool_call_id.clone()),
        );
        if let Some(object) = extra_payload.as_object() {
            for (key, value) in object {
                payload.insert(key.clone(), value.clone());
            }
        }
        repo.append_event(NewSessionEvent {
            session_id: approval_request.session_id.clone(),
            event_kind: event_kind.to_owned(),
            actor_session_id: Some(actor_session_id.to_owned()),
            payload_json: serde_json::Value::Object(payload),
        })
        .map(|_| ())
    }

    fn request_lineage_root_session_id(
        &self,
        repo: &SessionRepository,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
    ) -> Result<String, String> {
        repo.lineage_root_session_id(&approval_request.session_id)?
            .ok_or_else(|| {
                format!(
                    "approval_request_session_not_found: `{}`",
                    approval_request.session_id
                )
            })
    }

    fn replay_request(
        &self,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
    ) -> Result<(ToolExecutionPlane, ToolCoreRequest), String> {
        let execution_plane = match approval_request
            .governance_snapshot_json
            .get("execution_plane")
            .and_then(serde_json::Value::as_str)
        {
            Some("App") => ToolExecutionPlane::App,
            Some("Orchestration") => ToolExecutionPlane::Orchestration,
            Some(other) => {
                return Err(format!(
                    "approval_request_invalid_execution_plane: `{other}`"
                ));
            }
            None => {
                return Err(
                    "approval_request_invalid_execution_plane: missing governance snapshot execution_plane"
                        .to_owned(),
                );
            }
        };
        let tool_name = approval_request
            .request_payload_json
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "approval_request_invalid_payload: missing tool_name".to_owned())?;
        let args_json = approval_request
            .request_payload_json
            .get("args_json")
            .cloned()
            .ok_or_else(|| "approval_request_invalid_payload: missing args_json".to_owned())?;

        Ok((
            execution_plane,
            ToolCoreRequest {
                tool_name: tool_name.to_owned(),
                payload: args_json,
            },
        ))
    }

    fn replay_session_context(
        &self,
        repo: &SessionRepository,
        session_id: &str,
    ) -> Result<SessionContext, String> {
        let root_context = SessionContext::root_with_tool_view(
            session_id.to_owned(),
            runtime_tool_view_for_config(&self.tool_config),
        );
        let tool_view =
            effective_tool_view_for_session(&self.memory_config, &self.tool_config, &root_context)?;
        let parent_session_id = repo
            .load_session(session_id)?
            .and_then(|session| session.parent_session_id);
        Ok(match parent_session_id {
            Some(parent_session_id) => {
                SessionContext::child(session_id.to_owned(), parent_session_id, tool_view)
            }
            None => SessionContext::root_with_tool_view(session_id.to_owned(), tool_view),
        })
    }

    async fn replay_approved_request(
        &self,
        repo: &SessionRepository,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
        orchestration_replayer: Option<&(dyn ApprovalOrchestrationReplayer + '_)>,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        let (execution_plane, replay_request) = self.replay_request(approval_request)?;

        match execution_plane {
            ToolExecutionPlane::App => {
                let session_context =
                    self.replay_session_context(repo, &approval_request.session_id)?;
                let app_dispatcher = match &self.app_config {
                    Some(config) => DefaultAppToolDispatcher::production_with_config(
                        self.memory_config.clone(),
                        config.as_ref().clone(),
                    ),
                    None => DefaultAppToolDispatcher::production(
                        self.memory_config.clone(),
                        self.tool_config.clone(),
                    ),
                };
                app_dispatcher
                    .execute_app_tool(&session_context, replay_request, kernel_ctx)
                    .await
            }
            ToolExecutionPlane::Orchestration => {
                let orchestration_replayer = orchestration_replayer.ok_or_else(|| {
                        "approval_request_resume_not_supported: orchestration replay dispatcher not configured"
                            .to_owned()
                    })?;
                let session_context =
                    self.replay_session_context(repo, &approval_request.session_id)?;
                orchestration_replayer
                    .replay_orchestration_request(&session_context, replay_request, kernel_ctx)
                    .await
            }
            ToolExecutionPlane::Core => Err(
                "approval_request_resume_not_supported: core-plane tools do not create approval requests"
                    .to_owned(),
            ),
        }
    }

    fn replay_governance_snapshot(
        &self,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
    ) -> Option<ToolGovernanceSnapshot> {
        serde_json::from_value(approval_request.governance_snapshot_json.clone()).ok()
    }

    fn replay_decision(
        &self,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
    ) -> Result<ToolDecision, String> {
        let governance = self
            .replay_governance_snapshot(approval_request)
            .ok_or_else(|| {
                "approval_request_invalid_governance_snapshot: failed to deserialize replay governance snapshot"
                    .to_owned()
            })?;
        let decision = approval_request.decision.ok_or_else(|| {
            "approval_request_missing_decision: approved replay is missing approval decision"
                .to_owned()
        })?;
        let reason = format!("approval_request_{}", decision.as_str());
        Ok(ToolDecision {
            allow: true,
            deny: false,
            approval_required: false,
            reason: reason.clone(),
            rule_id: reason,
            governance,
        })
    }

    fn replay_success_outcome(
        &self,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
        resumed_tool_output: &ToolCoreOutcome,
    ) -> ToolOutcome {
        ToolOutcome {
            status: resumed_tool_output.status.clone(),
            payload: resumed_tool_output.payload.clone(),
            error_code: None,
            human_reason: None,
            audit_event_id: None,
            governance_allowed: true,
            governance: self.replay_governance_snapshot(approval_request),
        }
    }

    fn replay_failure_outcome(
        &self,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
        error: &str,
    ) -> ToolOutcome {
        ToolOutcome {
            status: "error".to_owned(),
            payload: serde_json::Value::Null,
            error_code: Some("tool_error".to_owned()),
            human_reason: Some(error.to_owned()),
            audit_event_id: None,
            governance_allowed: true,
            governance: self.replay_governance_snapshot(approval_request),
        }
    }

    async fn persist_replayed_outcome(
        &self,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
        outcome: &ToolOutcome,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<(), String> {
        persist_tool_outcome_with_memory_config(
            &self.memory_config,
            &approval_request.session_id,
            &approval_request.turn_id,
            &approval_request.tool_call_id,
            outcome,
            kernel_ctx,
        )
        .await
    }

    async fn persist_replayed_decision(
        &self,
        approval_request: &crate::session::repository::ApprovalRequestRecord,
        decision: &ToolDecision,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<(), String> {
        persist_tool_decision_with_memory_config(
            &self.memory_config,
            &approval_request.session_id,
            &approval_request.turn_id,
            &approval_request.tool_call_id,
            decision,
            kernel_ctx,
        )
        .await
    }

    async fn execute_approved_request(
        &self,
        repo: &SessionRepository,
        current_session_id: &str,
        approval_request_id: &str,
        orchestration_replayer: Option<&(dyn ApprovalOrchestrationReplayer + '_)>,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let executing = repo
            .transition_approval_request_if_current(
                approval_request_id,
                TransitionApprovalRequestIfCurrentRequest {
                    expected_status: ApprovalRequestStatus::Approved,
                    next_status: ApprovalRequestStatus::Executing,
                    decision: None,
                    resolved_by_session_id: None,
                    executed_at: None,
                    last_error: None,
                },
            )?
            .ok_or_else(|| {
                format!(
                    "approval_request_not_approved: `{approval_request_id}` is no longer approved"
                )
            })?;
        let replay_decision_persist_error = match self.replay_decision(&executing) {
            Ok(replay_decision) => self
                .persist_replayed_decision(&executing, &replay_decision, kernel_ctx)
                .await
                .err(),
            Err(error) => Some(error),
        };
        self.append_approval_event(
            repo,
            &executing,
            current_session_id,
            TOOL_APPROVAL_EXECUTION_STARTED_EVENT_KIND,
            serde_json::json!({
                "replay_decision_persisted": replay_decision_persist_error.is_none(),
                "replay_decision_persist_error": replay_decision_persist_error.clone(),
            }),
        )?;
        if let Some(error) = replay_decision_persist_error {
            let executed_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            let executed = repo
                .transition_approval_request_if_current(
                    approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Executing,
                        next_status: ApprovalRequestStatus::Executed,
                        decision: None,
                        resolved_by_session_id: None,
                        executed_at: Some(executed_at),
                        last_error: Some(error.clone()),
                    },
                )?
                .ok_or_else(|| {
                    format!(
                        "approval_request_not_executing: `{approval_request_id}` is no longer executing"
                    )
                })?;
            self.append_approval_event(
                repo,
                &executed,
                current_session_id,
                TOOL_APPROVAL_EXECUTION_FAILED_EVENT_KIND,
                serde_json::json!({
                    "error": error.clone(),
                    "replay_decision_persisted": false,
                    "replay_decision_persist_error": error.clone(),
                }),
            )?;
            return Err(error);
        }

        match self
            .replay_approved_request(repo, &executing, orchestration_replayer, kernel_ctx)
            .await
        {
            Ok(resumed_tool_output) => {
                let executed_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or(0);
                let executed = repo
                    .transition_approval_request_if_current(
                        approval_request_id,
                        TransitionApprovalRequestIfCurrentRequest {
                            expected_status: ApprovalRequestStatus::Executing,
                            next_status: ApprovalRequestStatus::Executed,
                            decision: None,
                            resolved_by_session_id: None,
                            executed_at: Some(executed_at),
                            last_error: None,
                        },
                    )?
                    .ok_or_else(|| {
                        format!(
                            "approval_request_not_executing: `{approval_request_id}` is no longer executing"
                        )
                    })?;
                let replay_outcome = self.replay_success_outcome(&executing, &resumed_tool_output);
                let replay_outcome_persist_error = self
                    .persist_replayed_outcome(&executing, &replay_outcome, kernel_ctx)
                    .await
                    .err();
                let executed = if let Some(error) = replay_outcome_persist_error.as_ref() {
                    repo.transition_approval_request_if_current(
                        approval_request_id,
                        TransitionApprovalRequestIfCurrentRequest {
                            expected_status: ApprovalRequestStatus::Executed,
                            next_status: ApprovalRequestStatus::Executed,
                            decision: None,
                            resolved_by_session_id: None,
                            executed_at: executed.executed_at,
                            last_error: Some(error.clone()),
                        },
                    )?
                    .ok_or_else(|| {
                        format!(
                            "approval_request_not_executed: `{approval_request_id}` lost executed state before recording replay outcome persistence error"
                        )
                    })?
                } else {
                    executed
                };
                self.append_approval_event(
                    repo,
                    &executed,
                    current_session_id,
                    TOOL_APPROVAL_EXECUTION_FINISHED_EVENT_KIND,
                    serde_json::json!({
                        "resumed_tool_status": resumed_tool_output.status.clone(),
                        "replay_outcome_persisted": replay_outcome_persist_error.is_none(),
                        "replay_outcome_persist_error": replay_outcome_persist_error.clone(),
                    }),
                )?;
                Ok(crate::tools::approval::ApprovalResolutionOutcome {
                    approval_request: executed,
                    resumed_tool_output: Some(resumed_tool_output),
                })
            }
            Err(error) => {
                let executed_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or(0);
                let executed = repo
                    .transition_approval_request_if_current(
                        approval_request_id,
                        TransitionApprovalRequestIfCurrentRequest {
                            expected_status: ApprovalRequestStatus::Executing,
                            next_status: ApprovalRequestStatus::Executed,
                            decision: None,
                            resolved_by_session_id: None,
                            executed_at: Some(executed_at),
                            last_error: Some(error.clone()),
                        },
                    )?
                    .ok_or_else(|| {
                        format!(
                            "approval_request_not_executing: `{approval_request_id}` is no longer executing"
                        )
                    })?;
                let replay_outcome = self.replay_failure_outcome(&executing, &error);
                let replay_outcome_persist_error = self
                    .persist_replayed_outcome(&executing, &replay_outcome, kernel_ctx)
                    .await
                    .err();
                let error = if let Some(persist_error) = replay_outcome_persist_error.as_ref() {
                    format!("{error}; replay_outcome_persist_error: {persist_error}")
                } else {
                    error
                };
                let executed = if replay_outcome_persist_error.is_some() {
                    repo.transition_approval_request_if_current(
                        approval_request_id,
                        TransitionApprovalRequestIfCurrentRequest {
                            expected_status: ApprovalRequestStatus::Executed,
                            next_status: ApprovalRequestStatus::Executed,
                            decision: None,
                            resolved_by_session_id: None,
                            executed_at: executed.executed_at,
                            last_error: Some(error.clone()),
                        },
                    )?
                    .ok_or_else(|| {
                        format!(
                            "approval_request_not_executed: `{approval_request_id}` lost executed state before recording replay failure outcome persistence error"
                        )
                    })?
                } else {
                    executed
                };
                self.append_approval_event(
                    repo,
                    &executed,
                    current_session_id,
                    TOOL_APPROVAL_EXECUTION_FAILED_EVENT_KIND,
                    serde_json::json!({
                        "error": error.clone(),
                        "replay_outcome_persisted": replay_outcome_persist_error.is_none(),
                        "replay_outcome_persist_error": replay_outcome_persist_error.clone(),
                    }),
                )?;
                Err(error)
            }
        }
    }

    pub(crate) async fn resolve_approval_request_with_orchestration_replayer(
        &self,
        request: crate::tools::approval::ApprovalResolutionRequest,
        orchestration_replayer: Option<&(dyn ApprovalOrchestrationReplayer + '_)>,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let approval_request = repo
            .load_approval_request(&request.approval_request_id)?
            .ok_or_else(|| {
                format!(
                    "approval_request_not_found: `{}`",
                    request.approval_request_id
                )
            })?;
        let is_visible = match request.visibility {
            SessionVisibility::SelfOnly => {
                request.current_session_id == approval_request.session_id
            }
            SessionVisibility::Children => {
                request.current_session_id == approval_request.session_id
                    || repo.is_session_visible(
                        &request.current_session_id,
                        &approval_request.session_id,
                    )?
            }
        };
        if !is_visible {
            return Err(format!(
                "visibility_denied: session `{}` is not visible from `{}`",
                approval_request.session_id, request.current_session_id
            ));
        }

        match request.decision {
            ApprovalDecision::Deny => {
                let resolved = repo.transition_approval_request_if_current(
                    &request.approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Pending,
                        next_status: ApprovalRequestStatus::Denied,
                        decision: Some(ApprovalDecision::Deny),
                        resolved_by_session_id: Some(request.current_session_id.clone()),
                        executed_at: None,
                        last_error: None,
                    },
                )?;
                let resolved = match resolved {
                    Some(resolved) => resolved,
                    None => {
                        let latest = repo
                            .load_approval_request(&request.approval_request_id)?
                            .ok_or_else(|| {
                                format!(
                                    "approval_request_not_found: `{}`",
                                    request.approval_request_id
                                )
                            })?;
                        return Err(format!(
                            "approval_request_not_pending: `{}` is already {}",
                            request.approval_request_id,
                            latest.status.as_str()
                        ));
                    }
                };
                repo.append_event(NewSessionEvent {
                    session_id: resolved.session_id.clone(),
                    event_kind: TOOL_APPROVAL_RESOLVED_EVENT_KIND.to_owned(),
                    actor_session_id: Some(request.current_session_id.clone()),
                    payload_json: serde_json::json!({
                        "approval_request_id": resolved.approval_request_id,
                        "decision": ApprovalDecision::Deny.as_str(),
                        "status": resolved.status.as_str(),
                        "resolved_by_session_id": request.current_session_id,
                    }),
                })?;
                Ok(crate::tools::approval::ApprovalResolutionOutcome {
                    approval_request: resolved,
                    resumed_tool_output: None,
                })
            }
            ApprovalDecision::ApproveOnce => {
                let approved = match repo.transition_approval_request_if_current(
                    &request.approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Pending,
                        next_status: ApprovalRequestStatus::Approved,
                        decision: Some(ApprovalDecision::ApproveOnce),
                        resolved_by_session_id: Some(request.current_session_id.clone()),
                        executed_at: None,
                        last_error: None,
                    },
                )? {
                    Some(approved) => approved,
                    None => {
                        let latest = repo
                            .load_approval_request(&request.approval_request_id)?
                            .ok_or_else(|| {
                                format!(
                                    "approval_request_not_found: `{}`",
                                    request.approval_request_id
                                )
                            })?;
                        return Err(format!(
                            "approval_request_not_pending: `{}` is already {}",
                            request.approval_request_id,
                            latest.status.as_str()
                        ));
                    }
                };
                self.append_approval_event(
                    &repo,
                    &approved,
                    &request.current_session_id,
                    TOOL_APPROVAL_RESOLVED_EVENT_KIND,
                    serde_json::json!({
                        "resolved_by_session_id": request.current_session_id,
                    }),
                )?;
                self.execute_approved_request(
                    &repo,
                    &request.current_session_id,
                    &request.approval_request_id,
                    orchestration_replayer,
                    kernel_ctx,
                )
                .await
            }
            ApprovalDecision::ApproveAlways => {
                let approved = match repo.transition_approval_request_if_current(
                    &request.approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Pending,
                        next_status: ApprovalRequestStatus::Approved,
                        decision: Some(ApprovalDecision::ApproveAlways),
                        resolved_by_session_id: Some(request.current_session_id.clone()),
                        executed_at: None,
                        last_error: None,
                    },
                )? {
                    Some(approved) => approved,
                    None => {
                        let latest = repo
                            .load_approval_request(&request.approval_request_id)?
                            .ok_or_else(|| {
                                format!(
                                    "approval_request_not_found: `{}`",
                                    request.approval_request_id
                                )
                            })?;
                        return Err(format!(
                            "approval_request_not_pending: `{}` is already {}",
                            request.approval_request_id,
                            latest.status.as_str()
                        ));
                    }
                };
                let grant_scope_session_id =
                    self.request_lineage_root_session_id(&repo, &approved)?;
                repo.upsert_approval_grant(NewApprovalGrantRecord {
                    scope_session_id: grant_scope_session_id.clone(),
                    approval_key: approved.approval_key.clone(),
                    created_by_session_id: Some(request.current_session_id.clone()),
                })?;
                self.append_approval_event(
                    &repo,
                    &approved,
                    &request.current_session_id,
                    TOOL_APPROVAL_RESOLVED_EVENT_KIND,
                    serde_json::json!({
                        "resolved_by_session_id": request.current_session_id,
                        "grant_scope_session_id": grant_scope_session_id,
                    }),
                )?;
                self.execute_approved_request(
                    &repo,
                    &request.current_session_id,
                    &request.approval_request_id,
                    orchestration_replayer,
                    kernel_ctx,
                )
                .await
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
struct DispatcherApprovalOrchestrationReplayer<'a> {
    dispatcher: &'a (dyn OrchestrationToolDispatcher + 'a),
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ApprovalOrchestrationReplayer for DispatcherApprovalOrchestrationReplayer<'_> {
    async fn replay_orchestration_request(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        self.dispatcher
            .execute_orchestration_tool(session_context, request, kernel_ctx)
            .await
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::tools::approval::ApprovalResolutionRuntime for DefaultApprovalResolutionRuntime {
    async fn resolve_approval_request(
        &self,
        request: crate::tools::approval::ApprovalResolutionRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let dispatcher_replayer = self
            .orchestration_dispatcher
            .as_deref()
            .map(|dispatcher| DispatcherApprovalOrchestrationReplayer { dispatcher });
        self.resolve_approval_request_with_orchestration_replayer(
            request,
            dispatcher_replayer
                .as_ref()
                .map(|replayer| replayer as &(dyn ApprovalOrchestrationReplayer + '_)),
            kernel_ctx,
        )
        .await
    }
}

#[derive(Clone)]
pub struct DefaultOrchestrationToolDispatcher {
    memory_config: MemoryRuntimeConfig,
    tool_config: ToolConfig,
    async_delegate_spawner: Option<Arc<dyn crate::tools::delegate::AsyncDelegateSpawner>>,
}

impl DefaultOrchestrationToolDispatcher {
    pub fn new(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        Self {
            memory_config,
            tool_config,
            async_delegate_spawner: None,
        }
    }

    #[cfg(feature = "memory-sqlite")]
    pub fn production(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        let async_delegate_spawner = SubprocessAsyncDelegateSpawner::from_current_process()
            .ok()
            .map(|spawner| Arc::new(spawner) as Arc<dyn AsyncDelegateSpawner>);
        Self {
            memory_config,
            tool_config,
            async_delegate_spawner,
        }
    }

    pub fn with_async_delegate_spawner(
        memory_config: MemoryRuntimeConfig,
        tool_config: ToolConfig,
        async_delegate_spawner: Arc<dyn crate::tools::delegate::AsyncDelegateSpawner>,
    ) -> Self {
        Self {
            memory_config,
            tool_config,
            async_delegate_spawner: Some(async_delegate_spawner),
        }
    }

    pub fn runtime() -> Self {
        #[cfg(feature = "memory-sqlite")]
        {
            return Self::production(
                crate::memory::runtime_config::get_memory_runtime_config().clone(),
                ToolConfig::default(),
            );
        }
        #[cfg(not(feature = "memory-sqlite"))]
        Self::new(
            crate::memory::runtime_config::get_memory_runtime_config().clone(),
            ToolConfig::default(),
        )
    }
}

impl Default for DefaultOrchestrationToolDispatcher {
    fn default() -> Self {
        Self::runtime()
    }
}

#[async_trait]
impl OrchestrationToolDispatcher for DefaultOrchestrationToolDispatcher {
    async fn execute_orchestration_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        let canonical_tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
        let effective_tool_view = effective_tool_view_for_session(
            &self.memory_config,
            &self.tool_config,
            session_context,
        )?;
        if let Some(descriptor) = tool_catalog().descriptor(canonical_tool_name) {
            if descriptor.execution_plane == ToolExecutionPlane::Orchestration
                && (!session_context.tool_view.contains(descriptor.name)
                    || !effective_tool_view.contains(descriptor.name))
            {
                return Err(format!("tool_not_visible: {}", descriptor.name));
            }
        }
        let effective_tool_config =
            effective_tool_config_for_session(&self.tool_config, session_context);
        crate::tools::execute_orchestration_tool_with_runtime_support(
            request,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
            crate::tools::OrchestrationToolRuntimeSupport {
                async_delegate_spawner: self.async_delegate_spawner.clone(),
            },
        )
        .await
    }
}

/// Single orchestration boundary for tool-call evaluation and execution.
///
/// `evaluate_turn` performs synchronous validation (no execution).
/// `execute_turn` performs policy-gated tool execution through the kernel.
pub struct TurnEngine {
    max_tool_steps: usize,
}

impl TurnEngine {
    pub fn new(max_tool_steps: usize) -> Self {
        Self { max_tool_steps }
    }

    /// Evaluate a provider turn and produce a deterministic result.
    /// Does NOT execute tools — just validates and gates.
    pub fn evaluate_turn(&self, turn: &ProviderTurn) -> TurnResult {
        self.evaluate_turn_in_view(turn, &runtime_tool_view())
    }

    pub fn evaluate_turn_in_view(&self, turn: &ProviderTurn, tool_view: &ToolView) -> TurnResult {
        self.evaluate_turn_in_context(turn, &session_context_from_turn(turn, tool_view.clone()))
    }

    pub fn evaluate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
    ) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::ToolDenied("max_tool_steps_exceeded".to_owned());
        }

        // Check each tool intent
        let catalog = tool_catalog();
        for intent in &turn.tool_intents {
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                return TurnResult::ToolDenied(format!("tool_not_found: {}", intent.tool_name));
            };
            if !session_context.tool_view.contains(descriptor.name) {
                return TurnResult::ToolDenied(format!("tool_not_visible: {}", intent.tool_name));
            }
        }

        // All tools validated — execution requires a kernel context
        TurnResult::NeedsApproval(ApprovalRequirement::kernel_context_required())
    }

    /// Execute a provider turn with policy-gated tool execution through the kernel.
    ///
    /// Flow:
    /// 1. No tool intents → `FinalText`
    /// 2. Too many intents → `ToolDenied("max_tool_steps_exceeded")`
    /// 3. Unknown tool → `ToolDenied("tool_not_found: ...")`
    /// 4. No kernel context → `ToolDenied("no_kernel_context")`
    /// 5. Policy/capability check via kernel → `ToolDenied` with reason if denied
    /// 6. Execute tool → map result to `TurnResult`
    pub async fn execute_turn(
        &self,
        turn: &ProviderTurn,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        self.execute_turn_in_view(turn, &runtime_tool_view(), kernel_ctx)
            .await
    }

    pub async fn execute_turn_in_view(
        &self,
        turn: &ProviderTurn,
        tool_view: &ToolView,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        self.execute_turn_in_context(
            turn,
            &session_context_from_turn(turn, tool_view.clone()),
            &DefaultAppToolDispatcher::runtime(),
            &DefaultOrchestrationToolDispatcher::runtime(),
            kernel_ctx,
        )
        .await
    }

    pub async fn execute_turn_in_context<
        A: AppToolDispatcher + ?Sized,
        O: OrchestrationToolDispatcher + ?Sized,
    >(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        app_dispatcher: &A,
        orchestration_dispatcher: &O,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        let governance = DefaultToolGovernanceEvaluator::default();
        self.execute_turn_in_context_with_governance(
            turn,
            session_context,
            &governance,
            app_dispatcher,
            orchestration_dispatcher,
            kernel_ctx,
        )
        .await
    }

    pub async fn execute_turn_in_context_with_governance<
        G: ToolGovernanceEvaluator + ?Sized,
        A: AppToolDispatcher + ?Sized,
        O: OrchestrationToolDispatcher + ?Sized,
    >(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        governance_evaluator: &G,
        app_dispatcher: &A,
        orchestration_dispatcher: &O,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        self.execute_turn_in_context_internal(
            turn,
            session_context,
            None,
            None,
            governance_evaluator,
            app_dispatcher,
            orchestration_dispatcher,
            kernel_ctx,
        )
        .await
    }

    pub(crate) async fn execute_turn_in_context_with_governance_and_persistence<
        R: ConversationRuntime + ?Sized,
        G: ToolGovernanceEvaluator + ?Sized,
        A: AppToolDispatcher + ?Sized,
        O: OrchestrationToolDispatcher + ?Sized,
    >(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        runtime: &R,
        governance_evaluator: &G,
        app_dispatcher: &A,
        orchestration_dispatcher: &O,
        approval_request_store: Option<&(dyn GovernedApprovalRequestStore + '_)>,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        let recorder = RuntimeToolLifecycleRecorder { runtime };
        self.execute_turn_in_context_internal(
            turn,
            session_context,
            Some(&recorder),
            approval_request_store,
            governance_evaluator,
            app_dispatcher,
            orchestration_dispatcher,
            kernel_ctx,
        )
        .await
    }

    async fn execute_turn_in_context_internal<
        G: ToolGovernanceEvaluator + ?Sized,
        A: AppToolDispatcher + ?Sized,
        O: OrchestrationToolDispatcher + ?Sized,
    >(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        lifecycle_recorder: Option<&(dyn ToolLifecycleRecorder + '_)>,
        approval_request_store: Option<&(dyn GovernedApprovalRequestStore + '_)>,
        governance_evaluator: &G,
        app_dispatcher: &A,
        orchestration_dispatcher: &O,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::ToolDenied("max_tool_steps_exceeded".to_owned());
        }

        // Check each tool intent is known
        let catalog = tool_catalog();
        for intent in &turn.tool_intents {
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                return TurnResult::ToolDenied(format!("tool_not_found: {}", intent.tool_name));
            };
            if !session_context.tool_view.contains(descriptor.name) {
                return TurnResult::ToolDenied(format!("tool_not_visible: {}", intent.tool_name));
            }
        }

        // Execute each tool intent through the kernel
        let mut outputs = Vec::new();
        for intent in &turn.tool_intents {
            let descriptor = catalog
                .resolve(&intent.tool_name)
                .expect("tool descriptor should remain resolvable after validation");
            let request = ToolCoreRequest {
                tool_name: descriptor.name.to_owned(),
                payload: intent.args_json.clone(),
            };
            match descriptor.execution_plane {
                ToolExecutionPlane::Core => {
                    let ctx = match kernel_ctx {
                        Some(ctx) => ctx,
                        None => return TurnResult::ToolDenied("no_kernel_context".to_owned()),
                    };
                    let caps = BTreeSet::from([Capability::InvokeTool]);
                    match ctx
                        .kernel
                        .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                        .await
                    {
                        Ok(outcome) => {
                            outputs.push(format!("[{}] {}", outcome.status, outcome.payload));
                        }
                        Err(e) => {
                            return match &e {
                                KernelError::Policy(_)
                                | KernelError::PackCapabilityBoundary { .. } => {
                                    TurnResult::ToolDenied(format!("{e}"))
                                }
                                _ => TurnResult::ToolError(format!("{e}")),
                            };
                        }
                    }
                }
                ToolExecutionPlane::App | ToolExecutionPlane::Orchestration => {
                    let governance_decision = governance_evaluator
                        .evaluate_tool_governance(descriptor, intent, session_context, kernel_ctx)
                        .await;
                    let tool_decision = ToolDecision::from_governance(&governance_decision);
                    if let Some(recorder) = lifecycle_recorder {
                        if let Err(error) = recorder
                            .persist_tool_decision(
                                &session_context.session_id,
                                &intent.turn_id,
                                &intent.tool_call_id,
                                &tool_decision,
                                kernel_ctx,
                            )
                            .await
                        {
                            return TurnResult::ToolError(format!(
                                "persist tool decision failed: {error}"
                            ));
                        }
                    }
                    if governance_decision.approval_required {
                        if let Some(recorder) = lifecycle_recorder {
                            let outcome = ToolOutcome::governed(
                                "approval_required",
                                serde_json::Value::Null,
                                Some("approval_required".to_owned()),
                                Some(governance_decision.reason.clone()),
                                None,
                                false,
                                &governance_decision,
                            );
                            if let Err(error) = recorder
                                .persist_tool_outcome(
                                    &session_context.session_id,
                                    &intent.turn_id,
                                    &intent.tool_call_id,
                                    &outcome,
                                    kernel_ctx,
                                )
                                .await
                            {
                                return TurnResult::ToolError(format!(
                                    "persist tool outcome failed: {error}"
                                ));
                            }
                        }
                        let approval_key = format!("tool:{}", descriptor.name);
                        let approval_request_id = match approval_request_store {
                            Some(store) => match store.ensure_governed_approval_request(
                                session_context,
                                intent,
                                descriptor,
                                &governance_decision,
                            ) {
                                Ok(approval_request_id) => Some(approval_request_id),
                                Err(error) => {
                                    return TurnResult::ToolError(format!(
                                        "persist approval request failed: {error}"
                                    ));
                                }
                            },
                            None => None,
                        };
                        return TurnResult::NeedsApproval(ApprovalRequirement::governed_tool(
                            descriptor.name,
                            approval_key,
                            governance_decision.reason.clone(),
                            governance_decision.rule_id.clone(),
                            approval_request_id,
                        ));
                    }
                    if !governance_decision.allow {
                        if let Some(recorder) = lifecycle_recorder {
                            let outcome = ToolOutcome::governed(
                                "denied",
                                serde_json::Value::Null,
                                Some("tool_denied".to_owned()),
                                Some(governance_decision.reason.clone()),
                                None,
                                false,
                                &governance_decision,
                            );
                            if let Err(error) = recorder
                                .persist_tool_outcome(
                                    &session_context.session_id,
                                    &intent.turn_id,
                                    &intent.tool_call_id,
                                    &outcome,
                                    kernel_ctx,
                                )
                                .await
                            {
                                return TurnResult::ToolError(format!(
                                    "persist tool outcome failed: {error}"
                                ));
                            }
                        }
                        return TurnResult::ToolDenied(governance_decision.reason);
                    }
                    match descriptor.execution_plane {
                        ToolExecutionPlane::App => match app_dispatcher
                            .execute_app_tool(session_context, request, kernel_ctx)
                            .await
                        {
                            Ok(outcome) => {
                                if let Some(recorder) = lifecycle_recorder {
                                    let tool_outcome = ToolOutcome::governed(
                                        outcome.status.clone(),
                                        outcome.payload.clone(),
                                        None,
                                        None,
                                        None,
                                        true,
                                        &governance_decision,
                                    );
                                    if let Err(error) = recorder
                                        .persist_tool_outcome(
                                            &session_context.session_id,
                                            &intent.turn_id,
                                            &intent.tool_call_id,
                                            &tool_outcome,
                                            kernel_ctx,
                                        )
                                        .await
                                    {
                                        return TurnResult::ToolError(format!(
                                            "persist tool outcome failed: {error}"
                                        ));
                                    }
                                }
                                outputs.push(format!("[{}] {}", outcome.status, outcome.payload));
                            }
                            Err(error) => {
                                if let Some(recorder) = lifecycle_recorder {
                                    let tool_outcome = ToolOutcome::governed(
                                        "error",
                                        serde_json::Value::Null,
                                        Some("tool_error".to_owned()),
                                        Some(error.clone()),
                                        None,
                                        true,
                                        &governance_decision,
                                    );
                                    if let Err(persist_error) = recorder
                                        .persist_tool_outcome(
                                            &session_context.session_id,
                                            &intent.turn_id,
                                            &intent.tool_call_id,
                                            &tool_outcome,
                                            kernel_ctx,
                                        )
                                        .await
                                    {
                                        return TurnResult::ToolError(format!(
                                            "persist tool outcome failed: {persist_error}"
                                        ));
                                    }
                                }
                                return TurnResult::ToolError(error);
                            }
                        },
                        ToolExecutionPlane::Orchestration => match orchestration_dispatcher
                            .execute_orchestration_tool(session_context, request, kernel_ctx)
                            .await
                        {
                            Ok(outcome) => {
                                if let Some(recorder) = lifecycle_recorder {
                                    let tool_outcome = ToolOutcome::governed(
                                        outcome.status.clone(),
                                        outcome.payload.clone(),
                                        None,
                                        None,
                                        None,
                                        true,
                                        &governance_decision,
                                    );
                                    if let Err(error) = recorder
                                        .persist_tool_outcome(
                                            &session_context.session_id,
                                            &intent.turn_id,
                                            &intent.tool_call_id,
                                            &tool_outcome,
                                            kernel_ctx,
                                        )
                                        .await
                                    {
                                        return TurnResult::ToolError(format!(
                                            "persist tool outcome failed: {error}"
                                        ));
                                    }
                                }
                                outputs.push(format!("[{}] {}", outcome.status, outcome.payload));
                            }
                            Err(error) => {
                                if let Some(recorder) = lifecycle_recorder {
                                    let tool_outcome = ToolOutcome::governed(
                                        "error",
                                        serde_json::Value::Null,
                                        Some("tool_error".to_owned()),
                                        Some(error.clone()),
                                        None,
                                        true,
                                        &governance_decision,
                                    );
                                    if let Err(persist_error) = recorder
                                        .persist_tool_outcome(
                                            &session_context.session_id,
                                            &intent.turn_id,
                                            &intent.tool_call_id,
                                            &tool_outcome,
                                            kernel_ctx,
                                        )
                                        .await
                                    {
                                        return TurnResult::ToolError(format!(
                                            "persist tool outcome failed: {persist_error}"
                                        ));
                                    }
                                }
                                return TurnResult::ToolError(error);
                            }
                        },
                        ToolExecutionPlane::Core => unreachable!(
                            "core tools should not enter app/orchestration governance branch"
                        ),
                    }
                }
            }
        }

        TurnResult::FinalText(outputs.join("\n"))
    }
}

#[cfg(all(test, feature = "memory-sqlite"))]
mod tests {
    use super::*;

    #[test]
    fn delegate_async_subprocess_plan_includes_config_path_and_delegate_child_flag() {
        let request = crate::tools::delegate::AsyncDelegateSpawnRequest {
            child_session_id: "delegate:child-1".to_owned(),
            parent_session_id: "root-session".to_owned(),
            task: "child task".to_owned(),
            label: Some("research".to_owned()),
            timeout_seconds: 19,
        };

        let plan = build_async_delegate_subprocess_plan(
            std::path::Path::new("/tmp/loongclawd"),
            Some("/tmp/loongclaw.toml"),
            &request,
        );

        assert_eq!(plan.program, std::path::PathBuf::from("/tmp/loongclawd"));
        assert_eq!(
            plan.args,
            vec![
                "run-turn",
                "--config",
                "/tmp/loongclaw.toml",
                "--session",
                "delegate:child-1",
                "--input",
                "child task",
                "--timeout-seconds",
                "19",
                "--delegate-child",
            ]
        );
    }
}

fn session_context_from_turn(turn: &ProviderTurn, tool_view: ToolView) -> SessionContext {
    let session_id = turn
        .tool_intents
        .first()
        .map(|intent| intent.session_id.as_str())
        .unwrap_or("default");
    SessionContext::root_with_tool_view(session_id, tool_view)
}
