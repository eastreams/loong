use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use loong_contracts::Capability;
use serde_json::Value;

use crate::memory;
use crate::provider;
#[cfg(feature = "memory-sqlite")]
use crate::session::store;
use crate::{CliResult, KernelContext};

use super::super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ConversationContextEngine,
};
use super::super::runtime_binding::ConversationRuntimeBinding;
use super::{
    AsyncDelegateSpawner, DefaultAsyncDelegateSpawner, DefaultConversationRuntime, LoongConfig,
    ProviderTurn, SessionContext, ToolView, apply_active_skill_blocked_tools_to_tool_view,
    apply_session_tool_policy_to_tool_view, build_base_tool_view_from_snapshot,
    build_session_context_from_snapshot, load_persisted_session_context,
    load_persisted_session_snapshot, open_session_repository, provider_runtime_binding,
    root_session_context_from_config,
};

#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        let tool_view = self.tool_view(config, session_id, binding)?;

        #[cfg(feature = "memory-sqlite")]
        if let Some(session_context) =
            load_persisted_session_context(config, session_id, &tool_view)?
        {
            return Ok(session_context);
        }

        Ok(root_session_context_from_config(
            config, session_id, tool_view,
        ))
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        let _ = (session_id, binding);
        Ok(crate::tools::runtime_tool_view_from_loong_config(config))
    }

    #[cfg(feature = "memory-sqlite")]
    fn async_delegate_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        Some(Arc::new(DefaultAsyncDelegateSpawner::new(config)))
    }

    #[cfg(feature = "memory-sqlite")]
    fn background_task_spawner(
        &self,
        _config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        None
    }

    async fn bootstrap(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        Ok(ContextEngineBootstrapResult::default())
    }

    async fn ingest(
        &self,
        _session_id: &str,
        _message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        Ok(ContextEngineIngestResult::default())
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_messages(
            config,
            session_id,
            include_system_prompt,
            &session_context.tool_view,
            binding,
        )
        .await
        .map(AssembledConversationContext::from_messages)
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>>;

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String>;

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        self.request_completion(config, messages, binding).await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn(config, session_id, turn_id, messages, tool_view, binding)
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
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn_streaming(
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
    ) -> CliResult<()>;

    async fn after_turn(
        &self,
        _session_id: &str,
        _user_input: &str,
        _assistant_reply: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn compact_context(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn prepare_subagent_spawn(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[async_trait]
impl<E> ConversationRuntime for DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;

            if let Some(snapshot) = snapshot {
                return build_session_context_from_snapshot(
                    config,
                    &repo,
                    session_id,
                    base_tool_view,
                    snapshot,
                );
            }

            Ok(root_session_context_from_config(
                config,
                session_id,
                base_tool_view,
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let tool_view = self.tool_view(config, session_id, _binding)?;
            Ok(root_session_context_from_config(
                config, session_id, tool_view,
            ))
        }
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;
            let tool_view = apply_session_tool_policy_to_tool_view(
                base_tool_view,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.session_tool_policy.as_ref()),
            );
            Ok(apply_active_skill_blocked_tools_to_tool_view(
                tool_view,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.active_skills.as_ref()),
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        Ok(crate::tools::runtime_tool_view_from_loong_config(config))
    }

    async fn bootstrap(
        &self,
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        let result = self
            .context_engine
            .bootstrap(config, session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_bootstrap(config, session_id, kernel_ctx)
            .await?;
        Ok(result)
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        let result = self
            .context_engine
            .ingest(session_id, message, kernel_ctx)
            .await?;
        self.run_turn_middlewares_ingest(session_id, message, kernel_ctx)
            .await?;
        Ok(result)
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_context_for_tool_view(
            config,
            &session_context,
            include_system_prompt,
            &session_context.tool_view,
            binding,
        )
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
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_context_for_tool_view(
            config,
            &session_context,
            include_system_prompt,
            tool_view,
            binding,
        )
        .await
        .map(|assembled| assembled.messages)
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        provider::request_completion(config, messages, provider_runtime_binding(binding)).await
    }

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        provider::request_completion_with_retry_progress(
            config,
            messages,
            provider_runtime_binding(binding),
            retry_progress,
        )
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
        provider::request_turn_in_view(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
        )
        .await
    }

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_in_view_with_retry_progress(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            retry_progress,
        )
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
        provider::request_turn_streaming_in_view(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            on_token,
        )
        .await
    }

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_streaming_in_view_with_retry_progress(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            on_token,
            retry_progress,
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
        if let Some(ctx) = binding.kernel_context() {
            let request = memory::build_append_turn_request(session_id, role, content);
            let caps = BTreeSet::from([Capability::MemoryWrite]);
            ctx.kernel
                .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|error| format!("persist {role} turn via kernel failed: {error}"))?;
            return Ok(());
        }

        #[cfg(feature = "memory-sqlite")]
        {
            store::append_session_turn_direct(
                session_id,
                role,
                content,
                store::current_session_store_config(),
            )
            .map_err(|error| format!("persist {role} turn failed: {error}"))?;
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_id, role, content);
        }

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
        self.context_engine
            .after_turn(
                session_id,
                user_input,
                assistant_reply,
                messages,
                kernel_ctx,
            )
            .await?;
        self.run_turn_middlewares_after_turn(
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
        self.context_engine
            .compact_context(config, session_id, messages, kernel_ctx)
            .await?;
        self.run_turn_middlewares_compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_prepare_subagent_spawn(
            parent_session_id,
            subagent_session_id,
            kernel_ctx,
        )
        .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_on_subagent_ended(
            parent_session_id,
            subagent_session_id,
            kernel_ctx,
        )
        .await
    }
}
