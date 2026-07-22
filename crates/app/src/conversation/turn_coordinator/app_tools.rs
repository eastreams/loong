use super::*;
use crate::conversation::autonomy_policy::AutonomyTurnBudgetState;
use crate::conversation::turn_engine::{ApprovalRequirement, LegacyToolPreflightOutcome};

fn effective_tool_config_for_session(
    tool_config: &crate::config::ToolConfig,
    session_context: &Context<'_>,
) -> crate::config::ToolConfig {
    let mut tool_config = tool_config.clone();
    if session_context.session().parent_session_id.is_some() {
        tool_config.sessions.visibility = crate::config::SessionVisibility::SelfOnly;
    }
    tool_config
}

pub(super) struct CoordinatorLegacyToolDispatcher<'a, R: ?Sized> {
    pub(super) config: &'a LoongConfig,
    pub(super) runtime: &'a R,
    pub(super) fallback: &'a DefaultLegacyToolDispatcher,
}

#[async_trait::async_trait]
impl<R> LegacyToolDispatcher for CoordinatorLegacyToolDispatcher<'_, R>
where
    R: ConversationRuntime + ?Sized,
{
    fn memory_config(&self) -> Option<&SessionStoreConfig> {
        self.fallback.memory_config()
    }

    async fn preflight_tool_intent(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &loong_contracts::ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: crate::conversation::turn_engine::LegacyToolDispatchKind,
        budget_state: &AutonomyTurnBudgetState,
    ) -> Result<LegacyToolPreflightOutcome, String> {
        self.fallback
            .preflight_tool_intent(
                session_context,
                intent,
                execution_request,
                trusted_internal_context,
                descriptor,
                dispatch_kind,
                budget_state,
            )
            .await
    }

    async fn maybe_require_approval(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &loong_contracts::ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: crate::conversation::turn_engine::LegacyToolDispatchKind,
    ) -> Result<Option<ApprovalRequirement>, String> {
        self.fallback
            .maybe_require_approval(
                session_context,
                intent,
                execution_request,
                trusted_internal_context,
                descriptor,
                dispatch_kind,
            )
            .await
    }

    async fn preflight_tool_execution(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        request: loong_contracts::ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
    ) -> Result<LegacyToolExecutionPreflight, String> {
        self.fallback
            .preflight_tool_execution(session_context, intent, request, descriptor)
            .await
    }

    async fn execute_core_tool(
        &self,
        session_context: &Context<'_>,
        request: loong_contracts::ToolCoreRequest,
        trusted_internal_context: bool,
    ) -> Result<loong_contracts::ToolCoreOutcome, crate::tools::LegacyToolRequestError> {
        self.fallback
            .execute_core_tool(session_context, request, trusted_internal_context)
            .await
    }

    async fn execute_app_tool(
        &self,
        session_context: &Context<'_>,
        request: loong_contracts::ToolCoreRequest,
    ) -> Result<loong_contracts::ToolCoreOutcome, String> {
        match crate::tools::canonical_tool_name(request.tool_name.as_str()) {
            "approval_request_resolve" => {
                #[cfg(not(feature = "memory-sqlite"))]
                {
                    let _ = session_context;
                    Err("approval tools require sqlite memory support (enable feature `memory-sqlite`)"
                        .to_owned())
                }

                #[cfg(feature = "memory-sqlite")]
                {
                    let memory_config = SessionStoreConfig::from_memory_config(&self.config.memory);
                    let effective_tool_config =
                        effective_tool_config_for_session(&self.config.tools, session_context);
                    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
                        self.config,
                        session_context,
                        self.runtime,
                        self.fallback,
                    );
                    crate::tools::approval::execute_approval_tool_with_runtime_support(
                        request,
                        &session_context.session().session_id,
                        &memory_config,
                        &effective_tool_config,
                        Some(&approval_runtime),
                    )
                    .await
                }
            }
            "delegate" => {
                execute_delegate_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
                    self.fallback,
                )
                .await
            }
            "delegate_async" => {
                execute_delegate_async_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
                    self.fallback,
                )
                .await
            }
            #[cfg(feature = "memory-sqlite")]
            "session_continue" => {
                let tool_config =
                    effective_tool_config_for_session(&self.config.tools, session_context);
                crate::tools::session::continue_session_with_runtime(
                    request.payload,
                    session_context,
                    &tool_config,
                    self.config,
                    self.runtime,
                    self.fallback,
                )
                .await
            }
            _ => {
                self.fallback
                    .execute_app_tool(session_context, request)
                    .await
            }
        }
    }

    async fn after_tool_execution(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        intent_sequence: usize,
        request: &loong_contracts::ToolCoreRequest,
        outcome: &loong_contracts::ToolCoreOutcome,
    ) {
        let tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());

        persist_tool_discovery_refresh_event_if_needed(
            self.runtime,
            &session_context.session().session_id,
            intent,
            intent_sequence,
            tool_name,
            outcome,
        )
        .await;
    }
}
