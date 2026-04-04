use async_trait::async_trait;
use serde_json::Value;

use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_coordinator::{execute_delegate_async_tool, execute_delegate_tool};
use super::turn_engine::{AppToolDispatcher, DefaultAppToolDispatcher};
use crate::config::{LoongClawConfig, ToolConsentMode};
use crate::session::repository::{
    ApprovalDecision, ApprovalRequestRecord, ApprovalRequestStatus, NewApprovalGrantRecord,
    NewSessionToolConsentRecord, SessionRepository, TransitionApprovalRequestIfCurrentRequest,
};
use crate::tools::ToolExecutionKind;

#[cfg(feature = "memory-sqlite")]
pub(super) struct CoordinatorApprovalResolutionRuntime<'a, R: ?Sized> {
    config: &'a LoongClawConfig,
    runtime: &'a R,
    fallback: &'a DefaultAppToolDispatcher,
    binding: ConversationRuntimeBinding<'a>,
}

#[cfg(feature = "memory-sqlite")]
struct ApprovalReplayRequest {
    request: loongclaw_contracts::ToolCoreRequest,
    execution_kind: crate::tools::ToolExecutionKind,
    trusted_internal_context: bool,
}

#[cfg(feature = "memory-sqlite")]
impl<'a, R> CoordinatorApprovalResolutionRuntime<'a, R>
where
    R: ConversationRuntime + ?Sized,
{
    pub(super) fn new(
        config: &'a LoongClawConfig,
        runtime: &'a R,
        fallback: &'a DefaultAppToolDispatcher,
        binding: ConversationRuntimeBinding<'a>,
    ) -> Self {
        Self {
            config,
            runtime,
            fallback,
            binding,
        }
    }

    fn can_replay_approved_request(&self) -> bool {
        self.binding.is_kernel_bound()
    }

    fn current_epoch_s() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0)
    }

    fn load_approval_request_or_not_found(
        repo: &SessionRepository,
        approval_request_id: &str,
    ) -> Result<ApprovalRequestRecord, String> {
        let latest_request = repo.load_approval_request(approval_request_id)?;
        let approval_request = latest_request
            .ok_or_else(|| format!("approval_request_not_found: `{approval_request_id}`"))?;

        Ok(approval_request)
    }

    fn ensure_approve_always_grant(
        repo: &SessionRepository,
        approval_request: &ApprovalRequestRecord,
        current_session_id: &str,
    ) -> Result<(), String> {
        let root_session_id = repo
            .lineage_root_session_id(&approval_request.session_id)?
            .ok_or_else(|| {
                format!(
                    "approval_request_session_not_found: `{}`",
                    approval_request.session_id
                )
            })?;

        let grant_record = NewApprovalGrantRecord {
            scope_session_id: root_session_id,
            approval_key: approval_request.approval_key.clone(),
            created_by_session_id: Some(current_session_id.to_owned()),
        };

        repo.upsert_approval_grant(grant_record)?;

        Ok(())
    }

    fn persist_session_consent_if_requested(
        repo: &SessionRepository,
        approval_request: &ApprovalRequestRecord,
        current_session_id: &str,
        session_consent_mode: Option<ToolConsentMode>,
    ) -> Result<(), String> {
        let Some(session_consent_mode) = session_consent_mode else {
            return Ok(());
        };

        let scope_session_id = repo
            .lineage_root_session_id(&approval_request.session_id)?
            .ok_or_else(|| {
                format!(
                    "approval_request_session_not_found: `{}`",
                    approval_request.session_id
                )
            })?;

        let consent_record = NewSessionToolConsentRecord {
            scope_session_id,
            mode: session_consent_mode,
            updated_by_session_id: Some(current_session_id.to_owned()),
        };

        repo.upsert_session_tool_consent(consent_record)?;

        Ok(())
    }

    fn replay_shell_request(
        &self,
        approval_request: &ApprovalRequestRecord,
        tool_name: &str,
        args_json: &Value,
    ) -> Result<ApprovalReplayRequest, String> {
        let canonical_tool_name = crate::tools::canonical_tool_name(tool_name);
        let mut payload = if canonical_tool_name == crate::tools::SHELL_EXEC_TOOL_NAME {
            args_json.clone()
        } else {
            let approved_tool_name = approval_request
                .request_payload_json
                .get("approved_tool_name")
                .and_then(Value::as_str)
                .map(crate::tools::canonical_tool_name)
                .unwrap_or(canonical_tool_name);
            if approved_tool_name != crate::tools::SHELL_EXEC_TOOL_NAME {
                return Err(format!(
                    "approval_request_invalid_execution_kind: expected `shell.exec`, got `{approved_tool_name}`"
                ));
            }
            args_json.get("arguments").cloned().ok_or_else(|| {
                "approval_request_invalid_payload: missing shell.exec arguments".to_owned()
            })?
        };
        let payload_object = payload.as_object_mut().ok_or_else(|| {
            "approval_request_invalid_payload: shell.exec args_json must be an object".to_owned()
        })?;
        let internal_context = crate::tools::shell_policy_ext::shell_exec_internal_approval_context(
            approval_request.approval_key.as_str(),
        );
        crate::tools::merge_trusted_internal_tool_context_into_arguments(
            payload_object,
            &internal_context,
        )?;

        Ok(ApprovalReplayRequest {
            request: loongclaw_contracts::ToolCoreRequest {
                tool_name: crate::tools::SHELL_EXEC_TOOL_NAME.to_owned(),
                payload,
            },
            execution_kind: crate::tools::ToolExecutionKind::Core,
            trusted_internal_context: true,
        })
    }

    fn replay_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<ApprovalReplayRequest, String> {
        let execution_kind = self.replay_execution_kind(approval_request)?;
        let tool_name = approval_request
            .request_payload_json
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "approval_request_invalid_payload: missing tool_name".to_owned())?;
        let payload = approval_request
            .request_payload_json
            .get("args_json")
            .cloned()
            .ok_or_else(|| "approval_request_invalid_payload: missing args_json".to_owned())?;

        match execution_kind {
            ToolExecutionKind::App => Ok(ApprovalReplayRequest {
                request: loongclaw_contracts::ToolCoreRequest {
                    tool_name: tool_name.to_owned(),
                    payload,
                },
                execution_kind: crate::tools::ToolExecutionKind::App,
                trusted_internal_context: false,
            }),
            ToolExecutionKind::Core => {
                let canonical_tool_name = crate::tools::canonical_tool_name(tool_name);
                if canonical_tool_name == crate::tools::SHELL_EXEC_TOOL_NAME {
                    return self.replay_shell_request(approval_request, tool_name, &payload);
                }

                Ok(ApprovalReplayRequest {
                    request: loongclaw_contracts::ToolCoreRequest {
                        tool_name: tool_name.to_owned(),
                        payload,
                    },
                    execution_kind: crate::tools::ToolExecutionKind::Core,
                    trusted_internal_context: false,
                })
            }
        }
    }

    fn replay_execution_kind(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<ToolExecutionKind, String> {
        let execution_kind = approval_request
            .request_payload_json
            .get("execution_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval_request_invalid_payload: missing execution_kind".to_owned())?;
        match execution_kind {
            "core" => Ok(ToolExecutionKind::Core),
            "app" => Ok(ToolExecutionKind::App),
            _ => Err(format!(
                "approval_request_invalid_execution_kind: expected `core` or `app`, got `{execution_kind}`"
            )),
        }
    }

    fn replay_requires_mutating_binding(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<bool, String> {
        let execution_kind = self.replay_execution_kind(approval_request)?;
        if execution_kind == ToolExecutionKind::Core {
            return Ok(true);
        }

        let tool_name = approval_request
            .request_payload_json
            .get("tool_name")
            .and_then(Value::as_str)
            .map(crate::tools::canonical_tool_name)
            .ok_or_else(|| "approval_request_invalid_payload: missing tool_name".to_owned())?;

        Ok(tool_name == "delegate_async")
    }

    fn ensure_resolution_binding_allows_decision(
        &self,
        approval_request: &ApprovalRequestRecord,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        let mutating_resolution_requested = matches!(
            decision,
            ApprovalDecision::ApproveOnce | ApprovalDecision::ApproveAlways
        );
        if !mutating_resolution_requested {
            return Ok(());
        }

        if self.binding.allows_mutation() {
            return Ok(());
        }

        let replay_requires_mutation = self.replay_requires_mutating_binding(approval_request)?;
        if !replay_requires_mutation {
            return Ok(());
        }

        Err("app_tool_denied: governed_runtime_binding_required".to_owned())
    }

    fn approval_request_not_pending_error(approval_request: &ApprovalRequestRecord) -> String {
        let approval_request_id = approval_request.approval_request_id.as_str();
        let status = approval_request.status.as_str();
        format!("approval_request_not_pending: `{approval_request_id}` is already {status}")
    }

    fn ensure_resolution_request_is_pending(
        approval_request: &ApprovalRequestRecord,
    ) -> Result<(), String> {
        if approval_request.status == ApprovalRequestStatus::Pending {
            return Ok(());
        }

        Err(Self::approval_request_not_pending_error(approval_request))
    }

    async fn finish_approved_resolution(
        &self,
        repo: &SessionRepository,
        approved: ApprovalRequestRecord,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let replay_is_allowed = self.can_replay_approved_request();
        if !replay_is_allowed {
            return Ok(crate::tools::approval::ApprovalResolutionOutcome {
                approval_request: approved,
                resumed_tool_output: None,
            });
        }

        let approval_request_id = approved.approval_request_id;
        self.execute_approved_request(repo, approval_request_id.as_str())
            .await
    }

    async fn resume_existing_approved_request(
        &self,
        repo: &SessionRepository,
        request: &crate::tools::approval::ApprovalResolutionRequest,
        approval_request: ApprovalRequestRecord,
        expected_decision: ApprovalDecision,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let status = approval_request.status;
        if status != ApprovalRequestStatus::Approved {
            return Err(Self::approval_request_not_pending_error(&approval_request));
        }

        let recorded_decision = approval_request.decision.ok_or_else(|| {
            let approval_request_id = request.approval_request_id.as_str();
            format!("approval_request_missing_decision: `{approval_request_id}` is approved")
        })?;

        if recorded_decision != expected_decision {
            let approval_request_id = request.approval_request_id.as_str();
            let recorded_decision_name = recorded_decision.as_str();
            let expected_decision_name = expected_decision.as_str();

            return Err(format!(
                "approval_request_decision_mismatch: `{approval_request_id}` is already `{recorded_decision_name}`, expected `{expected_decision_name}`"
            ));
        }

        if expected_decision == ApprovalDecision::ApproveAlways {
            Self::ensure_approve_always_grant(
                repo,
                &approval_request,
                &request.current_session_id,
            )?;
        }

        Self::persist_session_consent_if_requested(
            repo,
            &approval_request,
            &request.current_session_id,
            request.session_consent_mode,
        )?;

        self.finish_approved_resolution(repo, approval_request)
            .await
    }

    pub(super) async fn replay_approved_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
        let replay_request = self.replay_request(approval_request)?;
        match replay_request.execution_kind {
            crate::tools::ToolExecutionKind::Core => {
                let kernel_ctx = self
                    .binding
                    .kernel_context()
                    .ok_or_else(|| "no_kernel_context".to_owned())?;
                crate::tools::execute_kernel_tool_request(
                    kernel_ctx,
                    replay_request.request,
                    replay_request.trusted_internal_context,
                )
                .await
                .map_err(|error| error.to_string())
            }
            crate::tools::ToolExecutionKind::App => {
                let session_context = self
                    .runtime
                    .session_context(self.config, &approval_request.session_id, self.binding)
                    .map_err(|error| {
                        format!("load approval request session context failed: {error}")
                    })?;
                match crate::tools::canonical_tool_name(replay_request.request.tool_name.as_str()) {
                    "delegate" => {
                        execute_delegate_tool(
                            self.config,
                            self.runtime,
                            &session_context,
                            replay_request.request.payload,
                            self.binding,
                        )
                        .await
                    }
                    "delegate_async" => {
                        execute_delegate_async_tool(
                            self.config,
                            self.runtime,
                            &session_context,
                            replay_request.request.payload,
                            self.binding,
                        )
                        .await
                    }
                    _ => {
                        self.fallback
                            .execute_app_tool(
                                &session_context,
                                replay_request.request,
                                self.binding,
                            )
                            .await
                    }
                }
            }
        }
    }

    async fn execute_approved_request(
        &self,
        repo: &SessionRepository,
        approval_request_id: &str,
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

        match self.replay_approved_request(&executing).await {
            Ok(resumed_tool_output) => {
                let executed = repo
                    .transition_approval_request_if_current(
                        approval_request_id,
                        TransitionApprovalRequestIfCurrentRequest {
                            expected_status: ApprovalRequestStatus::Executing,
                            next_status: ApprovalRequestStatus::Executed,
                            decision: None,
                            resolved_by_session_id: None,
                            executed_at: Some(Self::current_epoch_s()),
                            last_error: None,
                        },
                    )?
                    .ok_or_else(|| {
                        format!(
                            "approval_request_not_executing: `{approval_request_id}` is no longer executing"
                        )
                    })?;
                Ok(crate::tools::approval::ApprovalResolutionOutcome {
                    approval_request: executed,
                    resumed_tool_output: Some(resumed_tool_output),
                })
            }
            Err(error) => {
                let maybe_executed = repo.transition_approval_request_if_current(
                    approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Executing,
                        next_status: ApprovalRequestStatus::Executed,
                        decision: None,
                        resolved_by_session_id: None,
                        executed_at: Some(Self::current_epoch_s()),
                        last_error: Some(error.clone()),
                    },
                )?;

                if maybe_executed.is_none() {
                    return Err(format!(
                        "approval_request_not_executing: `{approval_request_id}` is no longer executing; original replay error: {error}"
                    ));
                }

                Err(error)
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl<R> crate::tools::approval::ApprovalResolutionRuntime
    for CoordinatorApprovalResolutionRuntime<'_, R>
where
    R: ConversationRuntime + ?Sized,
{
    async fn resolve_approval_request(
        &self,
        request: crate::tools::approval::ApprovalResolutionRequest,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
            &self.config.memory,
        );
        let repo = SessionRepository::new(&memory_config)?;
        let approval_request = repo
            .load_approval_request(&request.approval_request_id)?
            .ok_or_else(|| {
                format!(
                    "approval_request_not_found: `{}`",
                    request.approval_request_id
                )
            })?;

        let is_visible = match request.visibility {
            crate::config::SessionVisibility::SelfOnly => {
                request.current_session_id == approval_request.session_id
            }
            crate::config::SessionVisibility::Children => {
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

        self.ensure_resolution_binding_allows_decision(&approval_request, request.decision)?;

        match request.decision {
            ApprovalDecision::Deny => {
                Self::ensure_resolution_request_is_pending(&approval_request)?;
                let resolved = match repo.transition_approval_request_if_current(
                    &request.approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Pending,
                        next_status: ApprovalRequestStatus::Denied,
                        decision: Some(ApprovalDecision::Deny),
                        resolved_by_session_id: Some(request.current_session_id.clone()),
                        executed_at: None,
                        last_error: None,
                    },
                )? {
                    Some(resolved) => resolved,
                    None => {
                        let latest = Self::load_approval_request_or_not_found(
                            &repo,
                            &request.approval_request_id,
                        )?;
                        return Err(Self::approval_request_not_pending_error(&latest));
                    }
                };
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
                        let latest = Self::load_approval_request_or_not_found(
                            &repo,
                            &request.approval_request_id,
                        )?;
                        return self
                            .resume_existing_approved_request(
                                &repo,
                                &request,
                                latest,
                                ApprovalDecision::ApproveOnce,
                            )
                            .await;
                    }
                };
                Self::persist_session_consent_if_requested(
                    &repo,
                    &approved,
                    &request.current_session_id,
                    request.session_consent_mode,
                )?;
                self.finish_approved_resolution(&repo, approved).await
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
                        let latest = Self::load_approval_request_or_not_found(
                            &repo,
                            &request.approval_request_id,
                        )?;
                        return self
                            .resume_existing_approved_request(
                                &repo,
                                &request,
                                latest,
                                ApprovalDecision::ApproveAlways,
                            )
                            .await;
                    }
                };
                Self::ensure_approve_always_grant(&repo, &approved, &request.current_session_id)?;
                self.finish_approved_resolution(&repo, approved).await
            }
        }
    }
}
