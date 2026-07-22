use serde_json::Value;

use crate::tools::ToolView;
use crate::{CliResult, Context};

use super::super::context_engine::{AssembledConversationContext, ConversationContextEngine};
use super::super::turn_middleware::TurnMiddlewareMetadata;
use super::{DefaultConversationRuntime, LoongConfig};

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
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.bootstrap(config, ctx).await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_ingest(
        &self,
        message: &Value,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.ingest(message, ctx).await?;
        }
        Ok(())
    }

    pub(super) async fn apply_turn_middlewares_to_context(
        &self,
        config: &LoongConfig,
        include_system_prompt: bool,
        mut assembled: AssembledConversationContext,
        runtime_tool_view: &ToolView,
        ctx: &Context<'_>,
    ) -> CliResult<AssembledConversationContext> {
        for middleware in &self.turn_middlewares {
            assembled = middleware
                .transform_context(
                    config,
                    include_system_prompt,
                    assembled,
                    runtime_tool_view,
                    ctx,
                )
                .await?;
        }
        Ok(assembled)
    }

    pub(super) async fn run_turn_middlewares_after_turn(
        &self,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .after_turn(user_input, assistant_reply, messages, ctx)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_compact_context(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.compact_context(config, messages, ctx).await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_prepare_subagent_spawn(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .prepare_subagent_spawn(subagent_session_id, ctx)
                .await?;
        }
        Ok(())
    }

    pub(super) async fn run_turn_middlewares_on_subagent_ended(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .on_subagent_ended(subagent_session_id, ctx)
                .await?;
        }
        Ok(())
    }
}
