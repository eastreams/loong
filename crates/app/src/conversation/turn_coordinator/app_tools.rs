use super::*;
use crate::conversation::autonomy_policy::AutonomyTurnBudgetState;
use crate::conversation::turn_engine::{ApprovalRequirement, ToolPreflightOutcome};

fn effective_tool_config_for_session(
    tool_config: &crate::config::ToolConfig,
    session_context: &SessionContext,
) -> crate::config::ToolConfig {
    let mut tool_config = tool_config.clone();
    if session_context.parent_session_id.is_some() {
        tool_config.sessions.visibility = crate::config::SessionVisibility::SelfOnly;
    }
    tool_config
}

pub(super) struct CoordinatorAppToolDispatcher<'a, R: ?Sized> {
    pub(super) config: &'a LoongConfig,
    pub(super) runtime: &'a R,
    pub(super) fallback: &'a DefaultAppToolDispatcher,
}

#[async_trait::async_trait]
impl<R> AppToolDispatcher for CoordinatorAppToolDispatcher<'_, R>
where
    R: ConversationRuntime + ?Sized,
{
    fn memory_config(&self) -> Option<&SessionStoreConfig> {
        self.fallback.memory_config()
    }

    async fn preflight_tool_intent_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
        budget_state: &AutonomyTurnBudgetState,
    ) -> Result<ToolPreflightOutcome, String> {
        self.fallback
            .preflight_tool_intent_with_binding(
                session_context,
                intent,
                descriptor,
                binding,
                budget_state,
            )
            .await
    }

    async fn maybe_require_approval_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        self.fallback
            .maybe_require_approval_with_binding(session_context, intent, descriptor, binding)
            .await
    }

    async fn preflight_tool_execution_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        request: loong_contracts::ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolExecutionPreflight, String> {
        self.fallback
            .preflight_tool_execution_with_binding(
                session_context,
                intent,
                request,
                descriptor,
                binding,
            )
            .await
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: loong_contracts::ToolCoreRequest,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<loong_contracts::ToolCoreOutcome, String> {
        match crate::tools::canonical_tool_name(request.tool_name.as_str()) {
            "approval_request_resolve" => {
                #[cfg(not(feature = "memory-sqlite"))]
                {
                    let _ = (session_context, binding);
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
                        self.runtime,
                        self.fallback,
                        binding,
                    );
                    crate::tools::approval::execute_approval_tool_with_runtime_support(
                        request,
                        &session_context.session_id,
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
                    binding,
                )
                .await
            }
            "delegate_async" => {
                execute_delegate_async_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
                    binding,
                )
                .await
            }
            _ => {
                self.fallback
                    .execute_app_tool(session_context, request, binding)
                    .await
            }
        }
    }

    async fn after_tool_execution(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        intent_sequence: usize,
        request: &loong_contracts::ToolCoreRequest,
        outcome: &loong_contracts::ToolCoreOutcome,
        binding: ConversationRuntimeBinding<'_>,
    ) {
        let tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());

        persist_tool_discovery_refresh_event_if_needed(
            self.runtime,
            &session_context.session_id,
            intent,
            intent_sequence,
            tool_name,
            outcome,
            binding,
        )
        .await;
    }
}
