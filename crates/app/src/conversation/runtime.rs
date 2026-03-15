use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use loongclaw_contracts::Capability;
use serde_json::{Value, json};

use crate::CliResult;
use crate::KernelContext;
use crate::tools::{
    ToolView, delegate_child_tool_view_for_config,
    delegate_child_tool_view_for_config_with_delegate,
};

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

#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    SessionKind, SessionRepository, SessionState, TransitionSessionWithEventIfCurrentRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub tool_view: ToolView,
}

impl SessionContext {
    pub fn root_with_tool_view(session_id: impl Into<String>, tool_view: ToolView) -> Self {
        Self {
            session_id: normalize_session_id(session_id.into()),
            parent_session_id: None,
            tool_view,
        }
    }

    pub fn child(
        session_id: impl Into<String>,
        parent_session_id: impl Into<String>,
        tool_view: ToolView,
    ) -> Self {
        Self {
            session_id: normalize_session_id(session_id.into()),
            parent_session_id: Some(normalize_session_id(parent_session_id.into())),
            tool_view,
        }
    }
}

fn normalize_session_id(session_id: String) -> String {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        "default".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncDelegateSpawnRequest {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub task: String,
    pub label: Option<String>,
    pub timeout_seconds: u64,
}

#[async_trait]
pub trait AsyncDelegateSpawner: Send + Sync {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String>;
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
struct DefaultAsyncDelegateSpawner {
    config: Arc<LoongClawConfig>,
}

#[cfg(feature = "memory-sqlite")]
impl DefaultAsyncDelegateSpawner {
    fn new(config: &LoongClawConfig) -> Self {
        Self {
            config: Arc::new(config.clone()),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl AsyncDelegateSpawner for DefaultAsyncDelegateSpawner {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        let repo = SessionRepository::new(&MemoryRuntimeConfig::from_memory_config(
            &self.config.memory,
        ))?;
        let started = repo.transition_session_with_event_if_current(
            &request.child_session_id,
            TransitionSessionWithEventIfCurrentRequest {
                expected_state: SessionState::Ready,
                next_state: SessionState::Running,
                last_error: None,
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some(request.parent_session_id.clone()),
                event_payload_json: json!({
                    "task": request.task.clone(),
                    "label": request.label.clone(),
                    "timeout_seconds": request.timeout_seconds,
                }),
            },
        )?;
        if started.is_none() {
            return Err(format!(
                "async_delegate_spawn_skipped: session `{}` was not in Ready state",
                request.child_session_id,
            ));
        }

        let runtime = DefaultConversationRuntime::from_config_or_env(self.config.as_ref())?;
        let _ = super::turn_coordinator::run_started_delegate_child_turn_with_runtime(
            self.config.as_ref(),
            &runtime,
            &request.child_session_id,
            &request.parent_session_id,
            request.label,
            &request.task,
            request.timeout_seconds,
            None,
        )
        .await;
        Ok(())
    }
}

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
    fn session_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<SessionContext> {
        Ok(SessionContext::root_with_tool_view(
            session_id,
            self.tool_view(config, session_id, kernel_ctx)?,
        ))
    }

    fn tool_view(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ToolView> {
        let _ = (session_id, kernel_ctx);
        Ok(crate::tools::runtime_tool_view_from_loongclaw_config(
            config,
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    fn async_delegate_spawner(
        &self,
        config: &LoongClawConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        Some(Arc::new(DefaultAsyncDelegateSpawner::new(config)))
    }

    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
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
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<AssembledConversationContext> {
        let session_context = self.session_context(config, session_id, kernel_ctx)?;
        self.build_messages(
            config,
            session_id,
            include_system_prompt,
            &session_context.tool_view,
            kernel_ctx,
        )
        .await
        .map(AssembledConversationContext::from_messages)
    }
    async fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
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
        tool_view: &ToolView,
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
impl<E> ConversationRuntime for DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    fn session_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<SessionContext> {
        let tool_view = self.tool_view(config, session_id, kernel_ctx)?;

        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
            if let Ok(repo) = SessionRepository::new(&memory_config) {
                if let Some(session) = repo
                    .load_session(session_id)
                    .map_err(|error| format!("load session context failed: {error}"))?
                {
                    if let Some(parent_session_id) = session.parent_session_id {
                        return Ok(SessionContext::child(
                            session.session_id,
                            parent_session_id,
                            tool_view,
                        ));
                    }
                } else if let Some(summary) = repo
                    .load_session_summary_with_legacy_fallback(session_id)
                    .map_err(|error| format!("load legacy session context failed: {error}"))?
                    && let Some(parent_session_id) = summary.parent_session_id
                {
                    return Ok(SessionContext::child(
                        summary.session_id,
                        parent_session_id,
                        tool_view,
                    ));
                }
            }
        }

        Ok(SessionContext::root_with_tool_view(session_id, tool_view))
    }

    fn tool_view(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ToolView> {
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
            if let Ok(repo) = SessionRepository::new(&memory_config) {
                if let Some(session) = repo
                    .load_session(session_id)
                    .map_err(|error| format!("load session tool-view context failed: {error}"))?
                {
                    if session.parent_session_id.is_some() {
                        let depth = match repo.session_lineage_depth(session_id) {
                            Ok(depth) => depth,
                            Err(error)
                                if error.starts_with("session_lineage_broken:")
                                    || error.starts_with("session_lineage_cycle_detected:") =>
                            {
                                return Ok(delegate_child_tool_view_for_config_with_delegate(
                                    &config.tools,
                                    false,
                                ));
                            }
                            Err(error) => {
                                return Err(format!(
                                    "compute session lineage depth for tool view failed: {error}"
                                ));
                            }
                        };
                        let allow_nested_delegate = depth < config.tools.delegate.max_depth;
                        return Ok(delegate_child_tool_view_for_config_with_delegate(
                            &config.tools,
                            allow_nested_delegate,
                        ));
                    }
                } else if repo
                    .load_session_summary_with_legacy_fallback(session_id)
                    .map_err(|error| {
                        format!("load legacy session tool-view context failed: {error}")
                    })?
                    .is_some_and(|session| session.kind == SessionKind::DelegateChild)
                {
                    return Ok(delegate_child_tool_view_for_config(&config.tools));
                }
            }
        }

        Ok(crate::tools::runtime_tool_view_from_loongclaw_config(
            config,
        ))
    }

    async fn bootstrap(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.context_engine
            .bootstrap(config, session_id, kernel_ctx)
            .await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
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
        let session_context = self.session_context(config, session_id, kernel_ctx)?;
        let mut assembled = self
            .context_engine
            .assemble_context(config, session_id, include_system_prompt, kernel_ctx)
            .await?;
        apply_system_prompt_addition(
            &mut assembled.messages,
            assembled.system_prompt_addition.as_deref(),
        );
        if include_system_prompt {
            apply_tool_view_to_system_prompt_if_needed(
                &mut assembled.messages,
                &crate::tools::runtime_tool_view_from_loongclaw_config(config),
                &session_context.tool_view,
            );
        }
        Ok(assembled)
    }

    async fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
        self.build_context(config, session_id, include_system_prompt, kernel_ctx)
            .await
            .map(|mut assembled| {
                apply_tool_view_to_system_prompt_if_needed(
                    &mut assembled.messages,
                    &crate::tools::runtime_tool_view_from_loongclaw_config(config),
                    tool_view,
                );
                assembled.messages
            })
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
        tool_view: &ToolView,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_in_view(config, messages, tool_view, kernel_ctx).await
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
            .await
    }

    async fn compact_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .compact_context(config, session_id, messages, kernel_ctx)
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

fn apply_tool_view_to_system_prompt_if_needed(
    messages: &mut [Value],
    runtime_tool_view: &ToolView,
    requested_tool_view: &ToolView,
) {
    if requested_tool_view != runtime_tool_view {
        apply_tool_view_to_system_prompt(messages, requested_tool_view);
    }
}

fn apply_tool_view_to_system_prompt(messages: &mut [Value], tool_view: &ToolView) {
    for message in messages.iter_mut() {
        let is_system = message.get("role").and_then(Value::as_str) == Some("system");
        if !is_system {
            continue;
        }

        let Some(content) = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(snapshot_start) = content.find("[available_tools]") else {
            continue;
        };
        let snapshot = crate::tools::capability_snapshot_for_view(tool_view);

        let prefix = content[..snapshot_start].trim_end();
        let rewritten = if prefix.is_empty() {
            snapshot
        } else {
            format!("{prefix}\n\n{snapshot}")
        };

        if let Some(object) = message.as_object_mut() {
            object.insert("content".to_owned(), Value::String(rewritten));
        }
        return;
    }
}
