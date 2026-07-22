use super::runtime::ConversationRuntime;
use super::turn_coordinator::{execute_delegate_async_tool, execute_delegate_tool};
use super::turn_engine::{
    DefaultLegacyToolDispatcher, LegacyToolDispatchKind, LegacyToolDispatcher,
};
use crate::Context;
use crate::config::LoongConfig;
use crate::session::repository::{ApprovalDecision, ApprovalRequestRecord};
use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use loong_contracts::Capability;
#[cfg(feature = "memory-sqlite")]
use loong_core::policy::context::PolicyContext;
use serde_json::Value;

#[cfg(feature = "memory-sqlite")]
pub(super) struct CoordinatorApprovalResolutionRuntime<'owner, 'context, R: ?Sized> {
    config: &'owner LoongConfig,
    ctx: &'owner Context<'context>,
    runtime: &'owner R,
    fallback: &'owner DefaultLegacyToolDispatcher,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug)]
enum ApprovalReplayRequest {
    LegacyCore {
        request: loong_contracts::ToolCoreRequest,
        trusted_internal_context: bool,
    },
    LegacyApp {
        request: loong_contracts::ToolCoreRequest,
    },
}

#[cfg(feature = "memory-sqlite")]
impl<'owner, 'context, R> CoordinatorApprovalResolutionRuntime<'owner, 'context, R>
where
    R: ConversationRuntime + ?Sized,
{
    pub(super) fn new(
        config: &'owner LoongConfig,
        ctx: &'owner Context<'context>,
        runtime: &'owner R,
        fallback: &'owner DefaultLegacyToolDispatcher,
    ) -> Self {
        Self {
            config,
            ctx,
            runtime,
            fallback,
        }
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

        Ok(ApprovalReplayRequest::LegacyCore {
            request: loong_contracts::ToolCoreRequest {
                tool_name: crate::tools::SHELL_EXEC_TOOL_NAME.to_owned(),
                payload,
            },
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
        match dispatch_kind {
            LegacyToolDispatchKind::LegacyApp => Ok(ApprovalReplayRequest::LegacyApp {
                request: loong_contracts::ToolCoreRequest {
                    tool_name: tool_name.to_owned(),
                    payload,
                },
            }),
            LegacyToolDispatchKind::LegacyCore => {
                let canonical_tool_name = crate::tools::canonical_tool_name(tool_name);
                if canonical_tool_name == crate::tools::SHELL_EXEC_TOOL_NAME {
                    return self.replay_shell_request(approval_request, tool_name, &payload);
                }
                let trusted_internal_context = approval_request
                    .request_payload_json
                    .get("trusted_internal_context")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| {
                        "approval_request_invalid_payload: missing trusted_internal_context"
                            .to_owned()
                    })?;

                Ok(ApprovalReplayRequest::LegacyCore {
                    request: loong_contracts::ToolCoreRequest {
                        tool_name: tool_name.to_owned(),
                        payload,
                    },
                    trusted_internal_context,
                })
            }
        }
    }

    fn replay_dispatch_kind(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<LegacyToolDispatchKind, String> {
        // Replay is an explicitly legacy protocol. Persisted typed requests
        // cannot be reinterpreted after registrations or policies have changed.
        let dispatch_kind = approval_request
            .request_payload_json
            .get("dispatch_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval_request_invalid_payload: missing dispatch_kind".to_owned())?;

        match dispatch_kind {
            "legacy_core" => Ok(LegacyToolDispatchKind::LegacyCore),
            "legacy_app" => Ok(LegacyToolDispatchKind::LegacyApp),
            "typed" => Err(
                "approval_request_unsupported_dispatch_kind: typed approval replay is not supported"
                    .to_owned(),
            ),
            _ => {
                let error = format!(
                    "approval_request_invalid_dispatch_kind: expected `legacy_core` or `legacy_app`, got `{dispatch_kind}`"
                );
                Err(error)
            }
        }
    }

    /// Re-evaluate legacy descriptor authority when persisted approval is replayed.
    ///
    /// The stored request records consent, not a permanent capability grant.
    /// Current Session visibility and descriptor governance therefore remain
    /// mandatory even when the operator previously approved the payload.
    fn replay_requires_legacy_authority(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<bool, String> {
        let dispatch_kind = self.replay_dispatch_kind(approval_request)?;
        if dispatch_kind == LegacyToolDispatchKind::LegacyCore {
            return Ok(true);
        }

        let tool_name = approval_request
            .request_payload_json
            .get("tool_name")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval_request_invalid_payload: missing tool_name".to_owned())?;
        let descriptor = crate::tools::tool_catalog()
            .resolve(tool_name)
            .ok_or_else(|| format!("approval_request_tool_not_found: {tool_name}"))?;
        if descriptor.owner != crate::tools::ToolOwner::LegacyApp {
            return Err(format!(
                "approval_request_owner_mismatch: expected legacy_app tool, got {}",
                descriptor.name
            ));
        }
        if !self.ctx.session().tool_view.contains(descriptor.name) {
            return Err(format!("tool_not_visible: {}", descriptor.name));
        }

        Ok(descriptor.requires_kernel_binding())
    }

    fn ensure_resolution_allowed(
        &self,
        approval_request: &ApprovalRequestRecord,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        // Validate the persisted execution owner before approval resolution may
        // write consent or grants. Typed records belong to PolicyEngine
        // permission handling and must never enter this legacy replay protocol.
        self.replay_dispatch_kind(approval_request)?;
        let mutating_resolution_requested = matches!(
            decision,
            ApprovalDecision::ApproveOnce | ApprovalDecision::ApproveAlways
        );
        if !mutating_resolution_requested {
            return Ok(());
        }

        if self
            .ctx
            .allowed_capabilities()
            .contains(Capability::InvokeTool)
        {
            return Ok(());
        }

        let replay_requires_legacy_authority =
            self.replay_requires_legacy_authority(approval_request)?;
        if !replay_requires_legacy_authority {
            return Ok(());
        }

        Err("app_tool_denied: session lacks invoke_tool capability".to_owned())
    }

    pub(super) async fn replay_approved_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<loong_contracts::ToolCoreOutcome, String> {
        match self.replay_request(approval_request)? {
            ApprovalReplayRequest::LegacyCore {
                request,
                trusted_internal_context,
            } => self
                .fallback
                .execute_core_tool(self.ctx, request, trusted_internal_context)
                .await
                .map_err(|error| error.to_string()),
            ApprovalReplayRequest::LegacyApp { request } => {
                match crate::tools::canonical_tool_name(request.tool_name.as_str()) {
                    "delegate" => {
                        execute_delegate_tool(
                            self.config,
                            self.runtime,
                            self.ctx,
                            request.payload,
                            self.fallback,
                        )
                        .await
                    }
                    "delegate_async" => {
                        execute_delegate_async_tool(
                            self.config,
                            self.runtime,
                            self.ctx,
                            request.payload,
                            self.fallback,
                        )
                        .await
                    }
                    _ => self.fallback.execute_app_tool(self.ctx, request).await,
                }
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl<R> crate::tools::approval::ApprovalResolutionRuntime
    for CoordinatorApprovalResolutionRuntime<'_, '_, R>
where
    R: ConversationRuntime + ?Sized,
{
    fn ensure_resolution_allowed(
        &self,
        approval_request: &ApprovalRequestRecord,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        CoordinatorApprovalResolutionRuntime::ensure_resolution_allowed(
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
