use std::collections::BTreeSet;

use async_trait::async_trait;
use loongclaw_contracts::Capability;
use serde_json::{Value, json};

use crate::CliResult;
use crate::KernelContext;

use super::super::memory;
use super::super::{config::LoongClawConfig, provider};
use super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ContextEngineMetadata, ConversationContextEngine, DefaultContextEngine,
};
use super::context_engine_registry::{
    DEFAULT_CONTEXT_ENGINE_ID, context_engine_id_from_env, describe_context_engine,
    list_context_engine_metadata, resolve_context_engine,
};
use super::turn_engine::ProviderTurn;

pub struct DefaultConversationRuntime<E = DefaultContextEngine> {
    context_engine: E,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextEngineSelectionSource {
    Env,
    Config,
    Default,
}

impl ContextEngineSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextEngineSelectionSource::Env => "env",
            ContextEngineSelectionSource::Config => "config",
            ContextEngineSelectionSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineSelection {
    pub id: String,
    pub source: ContextEngineSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompactionPolicySnapshot {
    pub enabled: bool,
    pub min_messages: Option<usize>,
    pub trigger_estimated_tokens: Option<usize>,
    pub fail_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineRuntimeSnapshot {
    pub selected: ContextEngineSelection,
    pub selected_metadata: ContextEngineMetadata,
    pub available: Vec<ContextEngineMetadata>,
    pub compaction: ContextCompactionPolicySnapshot,
}

pub fn resolve_context_engine_selection(config: &LoongClawConfig) -> ContextEngineSelection {
    if let Some(id) = context_engine_id_from_env() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Env,
        };
    }

    if let Some(id) = config.conversation.context_engine_id() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Config,
        };
    }

    ContextEngineSelection {
        id: DEFAULT_CONTEXT_ENGINE_ID.to_owned(),
        source: ContextEngineSelectionSource::Default,
    }
}

pub fn collect_context_engine_runtime_snapshot(
    config: &LoongClawConfig,
) -> CliResult<ContextEngineRuntimeSnapshot> {
    let selected = resolve_context_engine_selection(config);
    let selected_metadata = describe_context_engine(Some(selected.id.as_str()))?;
    let available = list_context_engine_metadata()?;
    let compaction = ContextCompactionPolicySnapshot {
        enabled: config.conversation.compact_enabled,
        min_messages: config.conversation.compact_min_messages(),
        trigger_estimated_tokens: config.conversation.compact_trigger_estimated_tokens(),
        fail_open: config.conversation.compaction_fail_open(),
    };

    Ok(ContextEngineRuntimeSnapshot {
        selected,
        selected_metadata,
        available,
        compaction,
    })
}

impl Default for DefaultConversationRuntime<DefaultContextEngine> {
    fn default() -> Self {
        Self {
            context_engine: DefaultContextEngine,
        }
    }
}

impl DefaultConversationRuntime<DefaultContextEngine> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<E> DefaultConversationRuntime<E> {
    pub fn with_context_engine(context_engine: E) -> Self {
        Self { context_engine }
    }
}

impl<E> DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    pub fn context_engine_metadata(&self) -> ContextEngineMetadata {
        self.context_engine.metadata()
    }
}

impl DefaultConversationRuntime<Box<dyn ConversationContextEngine>> {
    pub fn from_engine_id(engine_id: Option<&str>) -> CliResult<Self> {
        let context_engine = resolve_context_engine(engine_id)?;
        Ok(Self { context_engine })
    }

    pub fn from_config_or_env(config: &LoongClawConfig) -> CliResult<Self> {
        let selection = resolve_context_engine_selection(config);
        Self::from_engine_id(Some(selection.id.as_str()))
    }
}

#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ContextEngineBootstrapResult> {
        Ok(ContextEngineBootstrapResult::default())
    }

    async fn ingest(
        &self,
        _session_id: &str,
        _message: &Value,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ContextEngineIngestResult> {
        Ok(ContextEngineIngestResult::default())
    }

    async fn build_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<AssembledConversationContext> {
        self.build_messages(config, session_id, include_system_prompt, kernel_ctx)
            .await
            .map(AssembledConversationContext::from_messages)
    }
    async fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>>;

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String>;

    async fn request_turn(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ProviderTurn>;

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()>;

    async fn after_turn(
        &self,
        _session_id: &str,
        _user_input: &str,
        _assistant_reply: &str,
        _messages: &[Value],
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn compact_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _messages: &[Value],
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn prepare_subagent_spawn(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: Option<&KernelContext>,
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
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.context_engine
            .bootstrap(config, session_id, kernel_ctx)
            .await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ContextEngineIngestResult> {
        self.context_engine
            .ingest(session_id, message, kernel_ctx)
            .await
    }

    async fn build_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<AssembledConversationContext> {
        let mut assembled = self
            .context_engine
            .assemble_context(config, session_id, include_system_prompt, kernel_ctx)
            .await?;
        apply_system_prompt_addition(
            &mut assembled.messages,
            assembled.system_prompt_addition.as_deref(),
        );
        Ok(assembled)
    }

    async fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
        self.build_context(config, session_id, include_system_prompt, kernel_ctx)
            .await
            .map(|assembled| assembled.messages)
    }

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        provider::request_completion(config, messages, kernel_ctx).await
    }

    async fn request_turn(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn(config, messages, kernel_ctx).await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        if let Some(ctx) = kernel_ctx {
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
            memory::append_turn_direct(
                session_id,
                role,
                content,
                memory::runtime_config::get_memory_runtime_config(),
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
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        self.context_engine
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
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        self.context_engine
            .compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        self.context_engine
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        self.context_engine
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }
}

fn apply_system_prompt_addition(messages: &mut Vec<Value>, addition: Option<&str>) {
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
