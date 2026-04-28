use serde_json::Value;

use crate::{CliResult, KernelContext};

use super::super::context_engine::{AssembledConversationContext, ConversationContextEngine};
use super::super::runtime_binding::ConversationRuntimeBinding;
use super::super::turn_middleware::TurnMiddlewareMetadata;
use super::{DefaultConversationRuntime, LoongConfig, ToolView};

impl<E> DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    pub fn turn_middleware_metadata(&self) -> Vec<TurnMiddlewareMetadata> {
        self.turn_middlewares
            .iter()
            .map(|middleware| middleware.metadata())
            .collect()
    }

    pub(super) async fn run_turn_middlewares_bootstrap(
        &self,
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.bootstrap(config, session_id, kernel_ctx).await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.ingest(session_id, message, kernel_ctx).await?;
        }
        Ok(())
    }

    pub(super) async fn apply_turn_middlewares_to_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        mut assembled: AssembledConversationContext,
        runtime_tool_view: &ToolView,
        requested_tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        for middleware in &self.turn_middlewares {
            assembled = middleware
                .transform_context(
                    config,
                    session_id,
                    include_system_prompt,
                    assembled,
                    runtime_tool_view,
                    requested_tool_view,
                    binding,
                )
                .await?;
        }
        Ok(assembled)
    }

    pub(super) async fn run_turn_middlewares_after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .after_turn(
                    session_id,
                    user_input,
                    assistant_reply,
                    messages,
                    kernel_ctx,
                )
                .await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_compact_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .compact_context(config, session_id, messages, kernel_ctx)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
                .await?;
        }
        Ok(())
    }
}
