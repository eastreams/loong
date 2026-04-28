use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{CliResult, KernelContext};

use super::super::runtime_binding::ConversationRuntimeBinding;
use super::{
    AssembledConversationContext, AsyncDelegateSpawner, BoxedDefaultConversationRuntime,
    ContextEngineBootstrapResult, ContextEngineIngestResult, ConversationRuntime, LoongConfig,
    ProviderTurn, SessionContext, ToolView, load_default_conversation_runtime,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::store;

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub struct HostedConversationRuntime<R> {
    inner: R,
    memory_config: store::SessionStoreConfig,
    async_delegate_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
    background_task_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
}

#[cfg(feature = "memory-sqlite")]
impl<R> HostedConversationRuntime<R> {
    pub fn new(inner: R) -> Self {
        let memory_config = store::current_session_store_config().clone();
        Self::new_with_memory_config(inner, memory_config)
    }

    pub fn new_with_memory_config(inner: R, memory_config: store::SessionStoreConfig) -> Self {
        Self {
            inner,
            memory_config,
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
    let memory_config =
        store::session_store_config_from_memory_config_without_env_overrides(&config.memory);
    let runtime = HostedConversationRuntime::new_with_memory_config(inner_runtime, memory_config);
    Ok(runtime)
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl<R> ConversationRuntime for HostedConversationRuntime<R>
where
    R: ConversationRuntime,
{
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        self.inner.session_context(config, session_id, binding)
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        self.inner.tool_view(config, session_id, binding)
    }

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
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.inner.bootstrap(config, session_id, kernel_ctx).await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        self.inner.ingest(session_id, message, kernel_ctx).await
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.inner
            .build_context(config, session_id, include_system_prompt, binding)
            .await
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        self.inner
            .build_messages(
                config,
                session_id,
                include_system_prompt,
                tool_view,
                binding,
            )
            .await
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.inner
            .request_completion(config, messages, binding)
            .await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn(config, session_id, turn_id, messages, tool_view, binding)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn_streaming(
                config, session_id, turn_id, messages, tool_view, binding, on_token,
            )
            .await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        if binding.kernel_context().is_some() {
            return self
                .inner
                .persist_turn(session_id, role, content, binding)
                .await;
        }

        store::append_session_turn_direct(session_id, role, content, &self.memory_config)
            .map_err(|error| format!("persist {role} turn failed: {error}"))?;

        Ok(())
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .after_turn(
                session_id,
                user_input,
                assistant_reply,
                messages,
                kernel_ctx,
            )
            .await
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }
}
