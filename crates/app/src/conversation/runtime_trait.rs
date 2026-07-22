use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::memory;
use crate::provider;
use crate::{CliResult, Context};

use super::super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ConversationContextEngine,
};
use super::super::prompt_orchestrator::{
    seed_prompt_fragments_from_context, sync_prompt_fragments_into_context,
};
#[cfg(feature = "memory-sqlite")]
use super::super::session_history::AssistantHistoryLoadError;
#[cfg(feature = "memory-sqlite")]
use super::runtime_prompt::active_skills_prompt_summary;
use super::runtime_prompt::{
    append_runtime_prompt_fragment, delegate_child_profile_prompt_summary,
    delegate_child_runtime_contract_prompt_summary, runtime_self_continuity_prompt_summary,
};
use super::{
    AsyncDelegateSpawner, DefaultAsyncDelegateSpawner, DefaultConversationRuntime, LoongConfig,
    PromptFrameAuthority, ProviderTurn,
};

/// Conversation services bound to the authority of an execution Context.
///
/// Every context-bearing operation takes session identity and tool visibility
/// from `ctx.session()`. Callers targeting another session authority must first
/// bind a Context to that Session; parallel identity or visibility arguments
/// are intentionally not accepted because they could disagree with ctx.
#[async_trait]
pub trait ConversationRuntime: Send + Sync {
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
        _ctx: &Context<'_>,
    ) -> CliResult<ContextEngineBootstrapResult> {
        Ok(ContextEngineBootstrapResult::default())
    }

    async fn ingest(
        &self,
        _message: &Value,
        _ctx: &Context<'_>,
    ) -> CliResult<ContextEngineIngestResult> {
        Ok(ContextEngineIngestResult::default())
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        include_system_prompt: bool,
    ) -> CliResult<AssembledConversationContext> {
        self.build_messages(config, ctx, include_system_prompt)
            .await
            .map(AssembledConversationContext::from_messages)
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        include_system_prompt: bool,
    ) -> CliResult<Vec<Value>>;

    /// Reads the durable turn window through the current Context's MemoryAccess.
    ///
    /// Custom runtimes may provide another backing store. The default runtime
    /// uses the typed memory runtime owned by the Context's Session.
    #[cfg(feature = "memory-sqlite")]
    async fn read_session_window(
        &self,
        limit: usize,
        ctx: &Context<'_>,
    ) -> Result<Vec<memory::WindowTurn>, AssistantHistoryLoadError> {
        let _ = (limit, ctx);
        Err(AssistantHistoryLoadError::unavailable(
            "session-window reads are unavailable for this conversation runtime",
        ))
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<String>;

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        self.request_completion(config, messages, ctx).await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn(config, turn_id, messages, ctx).await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn_streaming(config, turn_id, messages, ctx, on_token)
            .await
    }

    async fn persist_turn(&self, role: &str, content: &str, ctx: &Context<'_>) -> CliResult<()>;

    async fn after_turn(
        &self,
        _user_input: &str,
        _assistant_reply: &str,
        _messages: &[Value],
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn compact_context(
        &self,
        _config: &LoongConfig,
        _messages: &[Value],
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn prepare_subagent_spawn(
        &self,
        _subagent_session_id: &str,
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        _subagent_session_id: &str,
        _ctx: &Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[async_trait]
impl<E> ConversationRuntime for DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    async fn bootstrap(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
    ) -> CliResult<ContextEngineBootstrapResult> {
        let result = self.context_engine.bootstrap(config, ctx).await?;
        self.run_turn_middlewares_bootstrap(config, ctx).await?;
        Ok(result)
    }

    async fn ingest(
        &self,
        message: &Value,
        ctx: &Context<'_>,
    ) -> CliResult<ContextEngineIngestResult> {
        let result = self.context_engine.ingest(message, ctx).await?;
        self.run_turn_middlewares_ingest(message, ctx).await?;
        Ok(result)
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_context: &Context<'_>,
        include_system_prompt: bool,
    ) -> CliResult<AssembledConversationContext> {
        let effective_config_storage;
        let effective_config = match session_context.session().workspace_root.as_ref() {
            Some(workspace_root) => {
                let mut overridden_config = config.clone();
                overridden_config.tools.file_root = Some(workspace_root.display().to_string());
                effective_config_storage = overridden_config;
                &effective_config_storage
            }
            None => config,
        };
        let runtime_tool_view = crate::tools::runtime_tool_view_from_loong_config(effective_config);
        let mut assembled = self
            .context_engine
            .assemble_context(effective_config, include_system_prompt, session_context)
            .await?;
        let runtime_self_continuity = include_system_prompt
            .then(|| {
                runtime_self_continuity_prompt_summary(
                    session_context,
                    assembled.runtime_self_continuity.as_ref(),
                )
            })
            .flatten();
        #[cfg(feature = "memory-sqlite")]
        let active_skills = include_system_prompt
            .then(|| {
                active_skills_prompt_summary(
                    effective_config,
                    session_context.session().session_id(),
                )
            })
            .flatten();
        #[cfg(not(feature = "memory-sqlite"))]
        let active_skills: Option<String> = None;
        let delegate_runtime_contract = include_system_prompt
            .then(|| {
                delegate_child_runtime_contract_prompt_summary(effective_config, session_context)
            })
            .flatten();
        let delegate_profile_contract = include_system_prompt
            .then(|| delegate_child_profile_prompt_summary(session_context))
            .flatten();

        seed_prompt_fragments_from_context(&mut assembled);
        append_runtime_prompt_fragment(
            &mut assembled,
            "runtime-self-continuity",
            runtime_self_continuity,
            PromptFrameAuthority::RuntimeSelf,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "active-skills",
            active_skills,
            PromptFrameAuthority::SessionLocalRecall,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "delegate-child-profile",
            delegate_profile_contract,
            PromptFrameAuthority::AdvisoryProfile,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "delegate-child-runtime-contract",
            delegate_runtime_contract,
            PromptFrameAuthority::CapabilityContract,
        );
        sync_prompt_fragments_into_context(&mut assembled);

        self.apply_turn_middlewares_to_context(
            effective_config,
            include_system_prompt,
            assembled,
            &runtime_tool_view,
            session_context,
        )
        .await
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        include_system_prompt: bool,
    ) -> CliResult<Vec<Value>> {
        self.build_context(config, ctx, include_system_prompt)
            .await
            .map(|assembled| assembled.messages)
    }

    #[cfg(feature = "memory-sqlite")]
    async fn read_session_window(
        &self,
        limit: usize,
        ctx: &Context<'_>,
    ) -> Result<Vec<memory::WindowTurn>, AssistantHistoryLoadError> {
        let snapshot = ctx.access().memory().window(limit, true).await?;
        Ok(snapshot
            .turns
            .into_iter()
            .map(|turn| memory::WindowTurn {
                role: turn.role,
                content: turn.content,
                ts: turn.ts,
            })
            .collect())
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<String> {
        provider::request_completion(config, messages, ctx).await
    }

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        provider::request_completion_with_retry_progress(config, messages, ctx, retry_progress)
            .await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn(config, turn_id, messages, ctx).await
    }

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_with_retry_progress(config, turn_id, messages, ctx, retry_progress)
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
        provider::request_turn_streaming(config, turn_id, messages, ctx, on_token).await
    }

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        turn_id: &str,
        messages: &[Value],
        ctx: &Context<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_streaming_with_retry_progress(
            config,
            turn_id,
            messages,
            ctx,
            on_token,
            retry_progress,
        )
        .await
    }

    async fn persist_turn(&self, role: &str, content: &str, ctx: &Context<'_>) -> CliResult<()> {
        ctx.access()
            .memory()
            .append_turn(role, content)
            .await
            .map_err(|error| format!("persist {role} turn via memory access failed: {error}"))?;
        Ok(())
    }

    async fn after_turn(
        &self,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.context_engine
            .after_turn(user_input, assistant_reply, messages, ctx)
            .await?;
        self.run_turn_middlewares_after_turn(user_input, assistant_reply, messages, ctx)
            .await
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.context_engine
            .compact_context(config, messages, ctx)
            .await?;
        self.run_turn_middlewares_compact_context(config, messages, ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.context_engine
            .prepare_subagent_spawn(subagent_session_id, ctx)
            .await?;
        self.run_turn_middlewares_prepare_subagent_spawn(subagent_session_id, ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        subagent_session_id: &str,
        ctx: &Context<'_>,
    ) -> CliResult<()> {
        self.context_engine
            .on_subagent_ended(subagent_session_id, ctx)
            .await?;
        self.run_turn_middlewares_on_subagent_ended(subagent_session_id, ctx)
            .await
    }
}
