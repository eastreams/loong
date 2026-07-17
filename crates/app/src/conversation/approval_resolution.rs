use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use loong_contracts::Capabilities;
use serde_json::Value;

use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_coordinator::{execute_delegate_async_tool, execute_delegate_tool};
use super::turn_engine::{AppToolDispatcher, DefaultAppToolDispatcher, ToolDispatchKind};
use crate::config::LoongConfig;
use crate::session::repository::{ApprovalDecision, ApprovalRequestRecord};

#[cfg(feature = "memory-sqlite")]
pub(super) struct CoordinatorApprovalResolutionRuntime<'a, R: ?Sized> {
    config: &'a LoongConfig,
    app_ctx: &'a crate::AppContext,
    runtime: &'a R,
    fallback: &'a DefaultAppToolDispatcher,
    binding: ConversationRuntimeBinding<'a>,
}

#[cfg(feature = "memory-sqlite")]
struct ApprovalReplayRequest {
    request: loong_contracts::ToolCoreRequest,
    dispatch_kind: ToolDispatchKind,
    capabilities_override: Option<Capabilities>,
    trusted_internal_context: bool,
}

#[cfg(feature = "memory-sqlite")]
impl<'a, R> CoordinatorApprovalResolutionRuntime<'a, R>
where
    R: ConversationRuntime + ?Sized,
{
    pub(super) fn new(
        config: &'a LoongConfig,
        app_ctx: &'a crate::AppContext,
        runtime: &'a R,
        fallback: &'a DefaultAppToolDispatcher,
        binding: ConversationRuntimeBinding<'a>,
    ) -> Self {
        Self {
            config,
            app_ctx,
            runtime,
            fallback,
            binding,
        }
    }

    fn can_replay_approved_request(&self) -> bool {
        self.binding.is_context_bound()
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
                let error = format!(
                    "approval_request_invalid_execution_kind: expected `shell.exec`, got `{approved_tool_name}`"
                );
                return Err(error);
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
            request: loong_contracts::ToolCoreRequest {
                tool_name: crate::tools::SHELL_EXEC_TOOL_NAME.to_owned(),
                payload,
            },
            dispatch_kind: ToolDispatchKind::LegacyCore,
            capabilities_override: None,
            trusted_internal_context: true,
        })
    }

    fn replay_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<ApprovalReplayRequest, String> {
        let dispatch_kind = self.replay_dispatch_kind(approval_request)?;
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
        let capabilities_override = serde_json::from_value::<Option<Capabilities>>(
            approval_request
                .request_payload_json
                .get("capabilities_override")
                .cloned()
                .ok_or_else(|| {
                    "approval_request_invalid_payload: missing capabilities_override".to_owned()
                })?,
        )
        .map_err(|error| {
            format!("approval_request_invalid_payload: invalid capabilities_override: {error}")
        })?;
        let trusted_internal_context = approval_request
            .request_payload_json
            .get("trusted_internal_context")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "approval_request_invalid_payload: missing trusted_internal_context".to_owned()
            })?;
        if dispatch_kind != ToolDispatchKind::Typed && capabilities_override.is_some() {
            return Err(
                "approval_request_invalid_payload: capabilities_override requires a registered typed tool"
                    .to_owned(),
            );
        }

        match dispatch_kind {
            ToolDispatchKind::LegacyApp => Ok(ApprovalReplayRequest {
                request: loong_contracts::ToolCoreRequest {
                    tool_name: tool_name.to_owned(),
                    payload,
                },
                dispatch_kind,
                capabilities_override,
                trusted_internal_context,
            }),
            ToolDispatchKind::Typed | ToolDispatchKind::LegacyCore => {
                let canonical_tool_name = crate::tools::canonical_tool_name(tool_name);
                if dispatch_kind == ToolDispatchKind::LegacyCore
                    && canonical_tool_name == crate::tools::SHELL_EXEC_TOOL_NAME
                {
                    return self.replay_shell_request(approval_request, tool_name, &payload);
                }

                Ok(ApprovalReplayRequest {
                    request: loong_contracts::ToolCoreRequest {
                        tool_name: tool_name.to_owned(),
                        payload,
                    },
                    dispatch_kind,
                    capabilities_override,
                    trusted_internal_context,
                })
            }
        }
    }

    fn replay_dispatch_kind(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<ToolDispatchKind, String> {
        // Replay requires the owner persisted by typed preparation. Older
        // `execution_kind` values cannot prove typed ownership and fail closed.
        let dispatch_kind = approval_request
            .request_payload_json
            .get("dispatch_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval_request_invalid_payload: missing dispatch_kind".to_owned())?;

        match dispatch_kind {
            "typed" => Ok(ToolDispatchKind::Typed),
            "legacy_core" => Ok(ToolDispatchKind::LegacyCore),
            "legacy_app" => Ok(ToolDispatchKind::LegacyApp),
            _ => {
                let error = format!(
                    "approval_request_invalid_dispatch_kind: expected `typed`, `legacy_core`, or `legacy_app`, got `{dispatch_kind}`"
                );
                Err(error)
            }
        }
    }

    fn replay_requires_mutating_binding(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<bool, String> {
        let dispatch_kind = self.replay_dispatch_kind(approval_request)?;
        if matches!(
            dispatch_kind,
            ToolDispatchKind::Typed | ToolDispatchKind::LegacyCore
        ) {
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

    pub(super) async fn replay_approved_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<loong_contracts::ToolCoreOutcome, String> {
        let replay_request = self.replay_request(approval_request)?;
        let ApprovalReplayRequest {
            request,
            dispatch_kind,
            capabilities_override,
            trusted_internal_context,
        } = replay_request;

        match dispatch_kind {
            ToolDispatchKind::Typed => {
                let session_context = self
                    .runtime
                    .session_context(
                        self.config,
                        self.app_ctx,
                        &approval_request.session_id,
                        self.binding,
                    )
                    .map_err(|error| {
                        format!("load approval request session context failed: {error}")
                    })?;
                crate::tools::execute_registered_tool_request(
                    &session_context,
                    request,
                    capabilities_override,
                    trusted_internal_context,
                )
                .await
                .map_err(|error| error.to_string())
            }
            ToolDispatchKind::LegacyCore => crate::tools::execute_legacy_kernel_tool_request(
                self.app_ctx,
                request,
                trusted_internal_context,
            )
            .await
            .map_err(|error| error.to_string()),
            ToolDispatchKind::LegacyApp => {
                let session_context = self
                    .runtime
                    .session_context(
                        self.config,
                        self.app_ctx,
                        &approval_request.session_id,
                        self.binding,
                    )
                    .map_err(|error| {
                        format!("load approval request session context failed: {error}")
                    })?;
                match crate::tools::canonical_tool_name(request.tool_name.as_str()) {
                    "delegate" => {
                        execute_delegate_tool(
                            self.config,
                            self.runtime,
                            &session_context,
                            request.payload,
                            self.binding,
                        )
                        .await
                    }
                    "delegate_async" => {
                        execute_delegate_async_tool(
                            self.config,
                            self.runtime,
                            &session_context,
                            request.payload,
                            self.binding,
                        )
                        .await
                    }
                    _ => {
                        self.fallback
                            .execute_app_tool(&session_context, request, self.binding)
                            .await
                    }
                }
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
    fn can_replay_approved_request(&self) -> bool {
        CoordinatorApprovalResolutionRuntime::can_replay_approved_request(self)
    }

    fn ensure_resolution_binding_allows_decision(
        &self,
        approval_request: &ApprovalRequestRecord,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        CoordinatorApprovalResolutionRuntime::ensure_resolution_binding_allows_decision(
            self,
            approval_request,
            decision,
        )
    }

    async fn replay_approved_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<loong_contracts::ToolCoreOutcome, String> {
        CoordinatorApprovalResolutionRuntime::replay_approved_request(self, approval_request).await
    }
}

#[cfg(all(test, feature = "memory-sqlite"))]
#[path = "approval_resolution/tests.rs"]
mod tests;
