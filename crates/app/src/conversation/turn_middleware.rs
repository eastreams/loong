use std::collections::BTreeSet;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::LoongClawConfig;
use crate::{CliResult, KernelContext};

use super::context_engine::AssembledConversationContext;
use super::runtime_binding::ConversationRuntimeBinding;

pub const TURN_MIDDLEWARE_API_VERSION: u16 = 1;
pub const SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID: &str = "system-prompt-addition";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TurnMiddlewareCapability {
    ContextTransform,
    SessionBootstrap,
    MessageIngestion,
    AfterTurn,
    ContextCompaction,
    SubagentLifecycle,
}

impl TurnMiddlewareCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnMiddlewareCapability::ContextTransform => "context_transform",
            TurnMiddlewareCapability::SessionBootstrap => "session_bootstrap",
            TurnMiddlewareCapability::MessageIngestion => "message_ingestion",
            TurnMiddlewareCapability::AfterTurn => "after_turn",
            TurnMiddlewareCapability::ContextCompaction => "context_compaction",
            TurnMiddlewareCapability::SubagentLifecycle => "subagent_lifecycle",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareMetadata {
    pub id: &'static str,
    pub api_version: u16,
    pub capabilities: BTreeSet<TurnMiddlewareCapability>,
}

impl TurnMiddlewareMetadata {
    pub fn new(
        id: &'static str,
        capabilities: impl IntoIterator<Item = TurnMiddlewareCapability>,
    ) -> Self {
        Self {
            id,
            api_version: TURN_MIDDLEWARE_API_VERSION,
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn capability_names(&self) -> Vec<&'static str> {
        self.capabilities
            .iter()
            .copied()
            .map(TurnMiddlewareCapability::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPromptAdditionTurnMiddleware;

#[async_trait]
pub trait ConversationTurnMiddleware: Send + Sync {
    fn id(&self) -> &'static str;

    fn metadata(&self) -> TurnMiddlewareMetadata {
        TurnMiddlewareMetadata::new(self.id(), [])
    }

    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn ingest(
        &self,
        _session_id: &str,
        _message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn transform_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        assembled: AssembledConversationContext,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        Ok(assembled)
    }

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
        _config: &LoongClawConfig,
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
impl ConversationTurnMiddleware for SystemPromptAdditionTurnMiddleware {
    fn id(&self) -> &'static str {
        SYSTEM_PROMPT_ADDITION_TURN_MIDDLEWARE_ID
    }

    fn metadata(&self) -> TurnMiddlewareMetadata {
        TurnMiddlewareMetadata::new(self.id(), [TurnMiddlewareCapability::ContextTransform])
    }

    async fn transform_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        mut assembled: AssembledConversationContext,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        apply_system_prompt_addition(
            &mut assembled.messages,
            assembled.system_prompt_addition.as_deref(),
        );
        Ok(assembled)
    }
}

pub(crate) fn apply_system_prompt_addition(messages: &mut Vec<Value>, addition: Option<&str>) {
    let Some(addition) = addition
        .map(str::trim)
        .filter(|content| !content.is_empty())
    else {
        return;
    };

    for message in messages.iter_mut() {
        let is_system = message.get("role").and_then(Value::as_str) == Some("system");
        if !is_system {
            continue;
        }

        if let Some(object) = message.as_object_mut() {
            let merged_content = match object.get("content").and_then(Value::as_str) {
                Some(existing) if !existing.trim().is_empty() => {
                    format!("{addition}\n\n{}", existing.trim())
                }
                _ => addition.to_owned(),
            };
            object.insert("content".to_owned(), Value::String(merged_content));
            return;
        }
    }

    messages.insert(
        0,
        json!({
            "role": "system",
            "content": addition,
        }),
    );
}

#[async_trait]
impl<T> ConversationTurnMiddleware for Box<T>
where
    T: ConversationTurnMiddleware + ?Sized,
{
    fn id(&self) -> &'static str {
        self.as_ref().id()
    }

    fn metadata(&self) -> TurnMiddlewareMetadata {
        self.as_ref().metadata()
    }

    async fn bootstrap(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .bootstrap(config, session_id, kernel_ctx)
            .await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref().ingest(session_id, message, kernel_ctx).await
    }

    async fn transform_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        assembled: AssembledConversationContext,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.as_ref()
            .transform_context(
                config,
                session_id,
                include_system_prompt,
                assembled,
                binding,
            )
            .await
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
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
        config: &LoongClawConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.as_ref()
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }
}
