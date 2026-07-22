use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{CliResult, Context};

use super::{
    AssembledConversationContext, AsyncDelegateSpawner, BoxedDefaultConversationRuntime,
    ContextEngineBootstrapResult, ContextEngineIngestResult, ConversationRuntime, LoongConfig,
    ProviderTurn, load_default_conversation_runtime,
};
#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub struct HostedConversationRuntime<R> {
    inner: R,
    async_delegate_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
    background_task_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
}

#[cfg(feature = "memory-sqlite")]
impl<R> HostedConversationRuntime<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            async_delegate_spawner_override: None,
            background_task_spawner_override: None,
        }
    }

    #[must_use]
    pub fn with_async_delegate_spawner(
        mut self,
        async_delegate_spawner: Arc<dyn AsyncDelegateSpawner>,
    ) -> Self {
        self.async_delegate_spawner_override = Some(async_delegate_spawner);
        self
    }

    #[must_use]
    pub fn with_background_task_spawner(
        mut self,
        background_task_spawner: Arc<dyn AsyncDelegateSpawner>,
    ) -> Self {
        self.background_task_spawner_override = Some(background_task_spawner);
        self
    }
}

#[cfg(feature = "memory-sqlite")]
pub fn load_hosted_default_conversation_runtime(
    config: &LoongConfig,
) -> CliResult<HostedConversationRuntime<BoxedDefaultConversationRuntime>> {
    let inner_runtime = load_default_conversation_runtime(config)?;
    Ok(HostedConversationRuntime::new(inner_runtime))
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl<R> ConversationRuntime for HostedConversationRuntime<R>
where
    R: ConversationRuntime,
{
    fn async_delegate_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        let override_spawner = self.async_delegate_spawner_override.clone();
        match override_spawner {
            Some(override_spawner) => Some(override_spawner),
            None => self.inner.async_delegate_spawner(config),
        }
    }

    fn background_task_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        let override_spawner = self.background_task_spawner_override.clone();
        match override_spawner {
            Some(override_spawner) => Some(override_spawner),
            None => self.inner.background_task_spawner(config),
        }
    }

    async fn bootstrap(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.inner.bootstrap(config, ctx).await
    }

    async fn ingest(
        &self,
        message: &Value,
        ctx: &Context<'_>,
    ) -> CliResult<ContextEngineIngestResult> {
        self.inner.ingest(message, ctx).await
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        include_system_prompt: bool,
    ) -> CliResult<AssembledConversationContext> {
        self.inner
            .build_context(config, ctx, include_system_prompt)
            .await
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        include_system_prompt: bool,
    ) -> CliResult<Vec<Value>> {
        self.inner
            .build_messages(config, ctx, include_system_prompt)
            .await
    }

    async fn read_session_window(
        &self,
        limit: usize,
        ctx: &Context<'_>,
    ) -> Result<Vec<crate::memory::WindowTurn>, crate::conversation::AssistantHistoryLoadError>
    {
        self.inner.read_session_window(limit, ctx).await
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<String> {
        self.inner.request_completion(config, messages, ctx).await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn(config, turn_id, messages, ctx)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn_streaming(config, turn_id, messages, ctx, on_token)
            .await
    }

    async fn persist_turn(&self, role: &str, content: &str, ctx: &Context<'_>) -> CliResult<()> {
        self.inner.persist_turn(role, content, ctx).await
    }

    async fn after_turn(
        &self,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.inner
            .after_turn(user_input, assistant_reply, messages, ctx)
            .await
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.inner.compact_context(config, messages, ctx).await
    }

    async fn prepare_subagent_spawn(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.inner
            .prepare_subagent_spawn(subagent_session_id, ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.inner.on_subagent_ended(subagent_session_id, ctx).await
    }
}
