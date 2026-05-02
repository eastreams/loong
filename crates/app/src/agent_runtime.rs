use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::CliResult;
use crate::acp::{AcpTurnEventSink, AcpTurnProvenance, JsonlAcpTurnEventSink};
use crate::chat::{
    CliChatOptions, initialize_cli_turn_runtime, initialize_cli_turn_runtime_with_loaded_config,
};
use crate::config::load as load_config;
use crate::conversation::{
    ConversationIngressContext, ConversationSessionAddress, PromptFrameEventSummary,
    load_prompt_frame_event_summary,
};
use crate::tools;
use loong_contracts::ToolCoreRequest;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnMode {
    Interactive,
    #[default]
    Oneshot,
    Delegate,
    Acp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
/// Transport-neutral request envelope for a single agent turn.
///
/// CLI entrypoints, control-plane requests, and channel bridges all normalize
/// into this shape before they borrow the shared chat/runtime pipeline.
pub struct AgentTurnRequest {
    pub message: String,
    pub turn_mode: AgentTurnMode,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub acp: bool,
    pub acp_event_stream: bool,
    pub acp_bootstrap_mcp_servers: Vec<String>,
    pub acp_cwd: Option<String>,
    pub live_surface_enabled: bool,
    pub requested_tool_ids: Option<Vec<String>>,
    pub disable_tools: bool,
}

impl AgentTurnRequest {
    #[must_use]
    pub fn with_requested_tool_ids(mut self, requested_tool_ids: Option<Vec<String>>) -> Self {
        self.requested_tool_ids = requested_tool_ids;
        self
    }

    #[must_use]
    pub fn with_disable_tools(mut self, disable_tools: bool) -> Self {
        self.disable_tools = disable_tools;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptAssemblyPlan {
    pub total_estimated_tokens: Option<usize>,
    pub stable_runtime_estimated_tokens: Option<usize>,
    pub session_latched_estimated_tokens: Option<usize>,
    pub session_local_recall_estimated_tokens: Option<usize>,
    pub turn_ephemeral_estimated_tokens: Option<usize>,
    pub stable_prefix_hash_sha256: Option<String>,
    pub cached_prefix_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptCachePlan {
    pub stable_prefix_hash_sha256: Option<String>,
    pub cached_prefix_sha256: Option<String>,
    pub stable_prefix_reused: bool,
    pub cached_prefix_reused: bool,
    pub session_latched_context_drifted: bool,
    pub session_local_recall_drifted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnResult {
    pub session_id: String,
    pub output_text: String,
    pub turn_mode: AgentTurnMode,
    pub governed_session_mode: loong_contracts::GovernedSessionMode,
    pub state: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub event_count: usize,
    pub prompt_assembly: PromptAssemblyPlan,
    pub prompt_cache: PromptCachePlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeEvent {
    pub id: i64,
    pub session_id: String,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub payload_json: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AgentRuntime;

impl AgentRuntime {
    pub const fn new() -> Self {
        Self
    }

    pub async fn run_turn(
        &self,
        config_path: Option<&str>,
        session_hint: Option<&str>,
        request: &AgentTurnRequest,
    ) -> CliResult<AgentTurnResult> {
        if request.message.trim().is_empty() {
            return Err("agent runtime message must not be empty".to_owned());
        }

        let options = cli_chat_options_for_turn_request(request);
        let runtime = initialize_cli_turn_runtime(
            config_path,
            session_hint,
            &options,
            kernel_scope_for_turn_mode(request.turn_mode),
        )?;
        let acp_event_printer = request
            .acp_event_stream
            .then(|| JsonlAcpTurnEventSink::stderr_with_prefix("acp-event> "));
        self.run_turn_with_runtime(
            &runtime,
            request,
            acp_event_printer
                .as_ref()
                .map(|printer| printer as &dyn AcpTurnEventSink),
        )
        .await
    }

    pub(crate) async fn run_turn_with_runtime(
        &self,
        runtime: &crate::chat::CliTurnRuntime,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
    ) -> CliResult<AgentTurnResult> {
        self.run_turn_with_runtime_and_context_and_manager(
            runtime,
            request,
            event_sink,
            None,
            None,
            AcpTurnProvenance::default(),
            crate::conversation::ProviderErrorMode::InlineMessage,
            None,
        )
        .await
    }

    pub(crate) async fn run_turn_with_runtime_and_observer(
        &self,
        runtime: &crate::chat::CliTurnRuntime,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
        observer: Option<crate::conversation::ConversationTurnObserverHandle>,
    ) -> CliResult<AgentTurnResult> {
        self.run_turn_with_runtime_and_observer_and_context(
            runtime,
            request,
            event_sink,
            observer,
            None,
            AcpTurnProvenance::default(),
        )
        .await
    }

    pub(crate) async fn run_turn_with_runtime_and_observer_and_context(
        &self,
        runtime: &crate::chat::CliTurnRuntime,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
        observer: Option<crate::conversation::ConversationTurnObserverHandle>,
        ingress: Option<&ConversationIngressContext>,
        provenance: AcpTurnProvenance<'_>,
    ) -> CliResult<AgentTurnResult> {
        self.run_turn_with_runtime_and_observer_and_context_and_error_mode(
            runtime,
            request,
            event_sink,
            observer,
            ingress,
            provenance,
            crate::conversation::ProviderErrorMode::InlineMessage,
        )
        .await
    }

    pub(crate) async fn run_turn_with_runtime_and_observer_and_context_and_error_mode(
        &self,
        runtime: &crate::chat::CliTurnRuntime,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
        observer: Option<crate::conversation::ConversationTurnObserverHandle>,
        ingress: Option<&ConversationIngressContext>,
        provenance: AcpTurnProvenance<'_>,
        provider_error_mode: crate::conversation::ProviderErrorMode,
    ) -> CliResult<AgentTurnResult> {
        self.run_turn_with_runtime_and_context_and_manager(
            runtime,
            request,
            event_sink,
            observer,
            ingress,
            provenance,
            provider_error_mode,
            None,
        )
        .await
    }

    async fn run_turn_with_runtime_and_context_and_manager(
        &self,
        runtime: &crate::chat::CliTurnRuntime,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
        observer: Option<crate::conversation::ConversationTurnObserverHandle>,
        ingress: Option<&ConversationIngressContext>,
        provenance: AcpTurnProvenance<'_>,
        provider_error_mode: crate::conversation::ProviderErrorMode,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<AgentTurnResult> {
        if request.message.trim().is_empty() {
            return Err("agent runtime message must not be empty".to_owned());
        }
        let message = request.message.as_str();

        let turn_address = resolved_session_address(runtime, request);
        let explicit_acp_request = runtime.explicit_acp_request
            || request.acp
            || matches!(request.turn_mode, AgentTurnMode::Acp);

        if explicit_acp_request {
            // ACP turns reuse the same runtime shell/session identity but swap
            // the execution lane entirely: they reload provider-facing config,
            // resolve ACP routing metadata, and bypass the normal provider
            // coordinator path below.
            let turn_config = load_runtime_turn_config(runtime)?;
            let acp_manager = match acp_manager {
                Some(manager) => manager,
                None => crate::acp::shared_acp_session_manager(&turn_config)?,
            };
            let acp_options = acp_turn_options_from_runtime(runtime, event_sink, request)
                .with_provenance(provenance);
            let execution = crate::acp::execute_acp_conversation_turn_for_address_with_manager(
                &turn_config,
                &turn_address,
                message,
                &acp_options,
                acp_manager,
            )
            .await?;
            let prompt_frame_summary = load_runtime_prompt_frame_summary(runtime).await;
            let (prompt_assembly, prompt_cache) = build_prompt_plans(&prompt_frame_summary);
            return match execution.outcome {
                crate::acp::AcpConversationTurnExecutionOutcome::Succeeded(success) => {
                    Ok(AgentTurnResult {
                        session_id: runtime.session_id.clone(),
                        output_text: success.result.output_text,
                        turn_mode: request.turn_mode,
                        governed_session_mode: runtime.conversation_binding().session_mode(),
                        state: Some(acp_session_state_label(success.result.state).to_owned()),
                        stop_reason: success
                            .result
                            .stop_reason
                            .map(acp_turn_stop_reason_label)
                            .map(ToOwned::to_owned),
                        usage: success.result.usage,
                        event_count: success.runtime_events.len(),
                        prompt_assembly,
                        prompt_cache,
                    })
                }
                crate::acp::AcpConversationTurnExecutionOutcome::Failed(failure) => {
                    Err(failure.error)
                }
            };
        }

        let (effective_ingress, effective_provenance) = effective_turn_context(ingress, provenance);
        let requested_tool_view = requested_tool_view_for_turn(request);
        let tool_view_runtime = if let Some(requested_tool_view) = requested_tool_view.clone() {
            Some(build_tool_view_override_runtime(
                runtime,
                requested_tool_view,
            )?)
        } else {
            None
        };
        let turn_outcome =
            crate::chat::run_cli_turn_with_runtime_override_and_address_and_ingress_and_error_mode_outcome(
                runtime,
                &turn_address,
                message,
                event_sink,
                request.live_surface_enabled,
                Some(&request.metadata),
                effective_ingress,
                effective_provenance,
                provider_error_mode,
                tool_view_runtime
                    .as_ref()
                    .map(|runtime| runtime as &dyn crate::conversation::ConversationRuntime),
                observer,
            )
            .await?;
        let prompt_frame_summary = load_runtime_prompt_frame_summary(runtime).await;
        let (prompt_assembly, prompt_cache) = build_prompt_plans(&prompt_frame_summary);

        Ok(AgentTurnResult {
            session_id: runtime.session_id.clone(),
            output_text: turn_outcome.reply,
            turn_mode: request.turn_mode,
            governed_session_mode: runtime.conversation_binding().session_mode(),
            state: None,
            stop_reason: None,
            usage: turn_outcome.usage,
            event_count: 0,
            prompt_assembly,
            prompt_cache,
        })
    }

    pub async fn resume_turn(
        &self,
        config_path: Option<&str>,
        session_hint: Option<&str>,
        request: &AgentTurnRequest,
    ) -> CliResult<AgentTurnResult> {
        self.run_turn(config_path, session_hint, request).await
    }

    /// Execute a turn when the caller already resolved config from disk.
    ///
    /// This keeps the high-level `AgentRuntime` flow intact while avoiding a
    /// second config load. It still bootstraps a fresh CLI turn runtime, so it
    /// is the right entrypoint when ACP/session managers do not need to be
    /// shared with an outer host.
    pub async fn run_turn_with_loaded_config(
        &self,
        resolved_path: PathBuf,
        config: crate::config::LoongConfig,
        session_hint: Option<&str>,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
    ) -> CliResult<AgentTurnResult> {
        if request.message.trim().is_empty() {
            return Err("agent runtime message must not be empty".to_owned());
        }

        let options = cli_chat_options_for_turn_request(request);
        let runtime = initialize_cli_turn_runtime_with_loaded_config(
            resolved_path,
            config,
            session_hint,
            &options,
            kernel_scope_for_turn_mode(request.turn_mode),
            crate::chat::CliSessionRequirement::AllowImplicitDefault,
            false,
        )?;

        self.run_turn_with_runtime(&runtime, request, event_sink)
            .await
    }

    pub async fn run_turn_with_loaded_config_and_observer_and_error_mode(
        &self,
        resolved_path: PathBuf,
        config: crate::config::LoongConfig,
        session_hint: Option<&str>,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
        observer: Option<crate::conversation::ConversationTurnObserverHandle>,
        provider_error_mode: crate::conversation::ProviderErrorMode,
    ) -> CliResult<AgentTurnResult> {
        if request.message.trim().is_empty() {
            return Err("agent runtime message must not be empty".to_owned());
        }

        let options = cli_chat_options_for_turn_request(request);
        let runtime = initialize_cli_turn_runtime_with_loaded_config(
            resolved_path,
            config,
            session_hint,
            &options,
            kernel_scope_for_turn_mode(request.turn_mode),
            crate::chat::CliSessionRequirement::AllowImplicitDefault,
            false,
        )?;

        self.run_turn_with_runtime_and_observer_and_context_and_error_mode(
            &runtime,
            request,
            event_sink,
            observer,
            None,
            AcpTurnProvenance::default(),
            provider_error_mode,
        )
        .await
    }

    /// Execute a turn with a preloaded config while reusing an existing ACP
    /// session manager.
    ///
    /// Control-plane and gateway surfaces use this when multiple turns should
    /// share ACP session ownership and streamed runtime events instead of each
    /// turn silently constructing its own manager.
    pub async fn run_turn_with_loaded_config_and_acp_manager(
        &self,
        resolved_path: PathBuf,
        config: crate::config::LoongConfig,
        session_hint: Option<&str>,
        request: &AgentTurnRequest,
        event_sink: Option<&dyn AcpTurnEventSink>,
        acp_manager: Arc<crate::acp::AcpSessionManager>,
    ) -> CliResult<AgentTurnResult> {
        if request.message.trim().is_empty() {
            return Err("agent runtime message must not be empty".to_owned());
        }

        let options = cli_chat_options_for_turn_request(request);
        let runtime = initialize_cli_turn_runtime_with_loaded_config(
            resolved_path,
            config,
            session_hint,
            &options,
            kernel_scope_for_turn_mode(request.turn_mode),
            crate::chat::CliSessionRequirement::AllowImplicitDefault,
            false,
        )?;

        self.run_turn_with_runtime_and_context_and_manager(
            &runtime,
            request,
            event_sink,
            None,
            None,
            AcpTurnProvenance::default(),
            crate::conversation::ProviderErrorMode::InlineMessage,
            Some(acp_manager),
        )
        .await
    }

    #[cfg(feature = "memory-sqlite")]
    pub async fn session_events(
        &self,
        config_path: Option<&str>,
        current_session_hint: Option<&str>,
        target_session_id: &str,
        limit: usize,
        after_id: Option<i64>,
    ) -> CliResult<Vec<AgentRuntimeEvent>> {
        let runtime = initialize_cli_turn_runtime(
            config_path,
            current_session_hint.or(Some(target_session_id)),
            &CliChatOptions::default(),
            "agent-runtime-session-events",
        )?;
        let outcome = tools::execute_app_tool_with_visibility_checked_config(
            ToolCoreRequest {
                tool_name: "session_events".to_owned(),
                payload: json!({
                    "session_id": target_session_id,
                    "limit": limit,
                    "after_id": after_id,
                }),
            },
            &runtime.session_id,
            &runtime.memory_config,
            &runtime.config.tools,
        )?;
        parse_agent_runtime_events(&outcome.payload)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    pub async fn session_events(
        &self,
        _config_path: Option<&str>,
        _current_session_hint: Option<&str>,
        _target_session_id: &str,
        _limit: usize,
        _after_id: Option<i64>,
    ) -> CliResult<Vec<AgentRuntimeEvent>> {
        Err("agent runtime session events unavailable: memory-sqlite feature disabled".to_owned())
    }

    #[cfg(feature = "memory-sqlite")]
    pub async fn cancel_turn(
        &self,
        config_path: Option<&str>,
        current_session_hint: Option<&str>,
        target_session_id: &str,
    ) -> CliResult<Value> {
        let runtime = initialize_cli_turn_runtime(
            config_path,
            current_session_hint.or(Some(target_session_id)),
            &CliChatOptions::default(),
            "agent-runtime-cancel-turn",
        )?;
        let outcome = tools::execute_app_tool_with_visibility_checked_config(
            ToolCoreRequest {
                tool_name: "session_cancel".to_owned(),
                payload: json!({
                    "session_id": target_session_id,
                }),
            },
            &runtime.session_id,
            &runtime.memory_config,
            &runtime.config.tools,
        )?;
        Ok(outcome.payload)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    pub async fn cancel_turn(
        &self,
        _config_path: Option<&str>,
        _current_session_hint: Option<&str>,
        _target_session_id: &str,
    ) -> CliResult<Value> {
        Err("agent runtime cancel unavailable: memory-sqlite feature disabled".to_owned())
    }
}

fn requested_tool_view_for_turn(request: &AgentTurnRequest) -> Option<crate::tools::ToolView> {
    if request.disable_tools {
        return Some(crate::tools::ToolView::from_tool_names(std::iter::empty::<
            &str,
        >()));
    }
    request
        .requested_tool_ids
        .as_ref()
        .map(|tool_ids| crate::tools::ToolView::from_tool_names(tool_ids.iter()))
}

fn build_tool_view_override_runtime(
    runtime: &crate::chat::CliTurnRuntime,
    requested_tool_view: crate::tools::ToolView,
) -> CliResult<ToolViewOverrideConversationRuntime> {
    let turn_config = load_runtime_turn_config(runtime)?;
    let inner: crate::conversation::DefaultConversationRuntime<
        Box<dyn crate::conversation::ConversationContextEngine>,
    > = crate::conversation::DefaultConversationRuntime::from_config_or_env(&turn_config)?;
    Ok(ToolViewOverrideConversationRuntime {
        inner,
        requested_tool_view,
    })
}

struct ToolViewOverrideConversationRuntime {
    inner: crate::conversation::DefaultConversationRuntime<
        Box<dyn crate::conversation::ConversationContextEngine>,
    >,
    requested_tool_view: crate::tools::ToolView,
}

#[async_trait]
impl crate::conversation::ConversationRuntime for ToolViewOverrideConversationRuntime {
    fn tool_view(
        &self,
        _config: &crate::config::LoongConfig,
        _session_id: &str,
        _binding: crate::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<crate::tools::ToolView> {
        Ok(self.requested_tool_view.clone())
    }

    fn async_delegate_spawner(
        &self,
        config: &crate::config::LoongConfig,
    ) -> Option<Arc<dyn crate::conversation::AsyncDelegateSpawner>> {
        self.inner.async_delegate_spawner(config)
    }

    fn background_task_spawner(
        &self,
        config: &crate::config::LoongConfig,
    ) -> Option<Arc<dyn crate::conversation::AsyncDelegateSpawner>> {
        self.inner.background_task_spawner(config)
    }

    async fn bootstrap(
        &self,
        config: &crate::config::LoongConfig,
        session_id: &str,
        kernel_ctx: &crate::KernelContext,
    ) -> CliResult<crate::conversation::ContextEngineBootstrapResult> {
        self.inner.bootstrap(config, session_id, kernel_ctx).await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &crate::KernelContext,
    ) -> CliResult<crate::conversation::ContextEngineIngestResult> {
        self.inner.ingest(session_id, message, kernel_ctx).await
    }

    async fn build_context(
        &self,
        config: &crate::config::LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: crate::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<crate::conversation::AssembledConversationContext> {
        let session_context = crate::conversation::ConversationRuntime::session_context(
            self, config, session_id, binding,
        )?;
        self.inner
            .build_context_for_tool_view(
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
        config: &crate::config::LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &crate::tools::ToolView,
        binding: crate::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        let session_context = crate::conversation::ConversationRuntime::session_context(
            self, config, session_id, binding,
        )?;
        self.inner
            .build_context_for_tool_view(
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
        config: &crate::config::LoongConfig,
        messages: &[Value],
        binding: crate::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.inner
            .request_completion(config, messages, binding)
            .await
    }

    async fn request_turn(
        &self,
        config: &crate::config::LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &crate::tools::ToolView,
        binding: crate::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
        self.inner
            .request_turn(config, session_id, turn_id, messages, tool_view, binding)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &crate::config::LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &crate::tools::ToolView,
        binding: crate::conversation::ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
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
        binding: crate::conversation::ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        self.inner
            .persist_turn(session_id, role, content, binding)
            .await
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &crate::KernelContext,
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
        config: &crate::config::LoongConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &crate::KernelContext,
    ) -> CliResult<()> {
        self.inner
            .compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &crate::KernelContext,
    ) -> CliResult<()> {
        self.inner
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &crate::KernelContext,
    ) -> CliResult<()> {
        self.inner
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }
}

/// Project the subset of an `AgentTurnRequest` that affects CLI/chat runtime
/// bootstrap.
///
/// The rest of the request remains turn-local and is passed later to the
/// coordinator/provider pipeline.
fn cli_chat_options_for_turn_request(request: &AgentTurnRequest) -> CliChatOptions {
    CliChatOptions {
        acp_requested: request.acp || matches!(request.turn_mode, AgentTurnMode::Acp),
        acp_event_stream: request.acp_event_stream,
        acp_bootstrap_mcp_servers: request.acp_bootstrap_mcp_servers.clone(),
        acp_working_directory: normalized_turn_working_directory(request.acp_cwd.as_deref()),
    }
}

fn normalized_turn_working_directory(value: Option<&str>) -> Option<std::path::PathBuf> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(std::path::PathBuf::from(value))
}

/// Enrich the resolved root session id with any channel/account/thread scope
/// that the caller supplied for this turn.
///
/// The underlying persisted session remains keyed by `runtime.session_id`; this
/// helper only projects the structured conversation address used by dispatch
/// and ACP routing.
fn resolved_session_address(
    runtime: &crate::chat::CliTurnRuntime,
    request: &AgentTurnRequest,
) -> ConversationSessionAddress {
    let mut address = ConversationSessionAddress::from_session_id(runtime.session_id.clone());
    if let (Some(channel_id), Some(conversation_id)) = (
        request.channel_id.as_deref(),
        request.conversation_id.as_deref(),
    ) {
        address = address.with_channel_scope(channel_id, conversation_id);
    }
    if let Some(account_id) = request.account_id.as_deref() {
        address = address.with_account_id(account_id);
    }
    if let Some(participant_id) = request.participant_id.as_deref() {
        address = address.with_participant_id(participant_id);
    }
    if let Some(thread_id) = request.thread_id.as_deref() {
        address = address.with_thread_id(thread_id);
    }
    address
}

fn acp_turn_options_from_runtime<'a>(
    runtime: &'a crate::chat::CliTurnRuntime,
    event_sink: Option<&'a dyn AcpTurnEventSink>,
    request: &'a AgentTurnRequest,
) -> crate::acp::AcpConversationTurnOptions<'a> {
    let base = if runtime.explicit_acp_request || request.acp {
        crate::acp::AcpConversationTurnOptions::explicit()
    } else {
        crate::acp::AcpConversationTurnOptions::automatic()
    };
    base.with_event_sink(event_sink)
        .with_additional_bootstrap_mcp_servers(&runtime.effective_bootstrap_mcp_servers)
        .with_working_directory(runtime.effective_working_directory.as_deref())
        .with_metadata(Some(&request.metadata))
}

fn acp_session_state_label(state: crate::acp::AcpSessionState) -> &'static str {
    match state {
        crate::acp::AcpSessionState::Initializing => "initializing",
        crate::acp::AcpSessionState::Ready => "ready",
        crate::acp::AcpSessionState::Busy => "busy",
        crate::acp::AcpSessionState::Cancelling => "cancelling",
        crate::acp::AcpSessionState::Error => "error",
        crate::acp::AcpSessionState::Closed => "closed",
    }
}

fn acp_turn_stop_reason_label(stop_reason: crate::acp::AcpTurnStopReason) -> &'static str {
    match stop_reason {
        crate::acp::AcpTurnStopReason::Completed => "completed",
        crate::acp::AcpTurnStopReason::Cancelled => "cancelled",
    }
}

fn kernel_scope_for_turn_mode(turn_mode: AgentTurnMode) -> &'static str {
    match turn_mode {
        AgentTurnMode::Interactive => "agent-runtime-interactive",
        AgentTurnMode::Oneshot => "agent-runtime-oneshot",
        AgentTurnMode::Delegate => "agent-runtime-delegate",
        AgentTurnMode::Acp => "agent-runtime-acp",
    }
}

fn effective_turn_context<'a>(
    ingress: Option<&'a ConversationIngressContext>,
    provenance: AcpTurnProvenance<'a>,
) -> (
    Option<&'a ConversationIngressContext>,
    AcpTurnProvenance<'a>,
) {
    let has_provenance = provenance.trace_id.is_some()
        || provenance.source_message_id.is_some()
        || provenance.ack_cursor.is_some();
    let effective_provenance = if has_provenance {
        provenance
    } else {
        AcpTurnProvenance::default()
    };
    (ingress, effective_provenance)
}

async fn load_runtime_prompt_frame_summary(
    runtime: &crate::chat::CliTurnRuntime,
) -> PromptFrameEventSummary {
    #[cfg(feature = "memory-sqlite")]
    {
        return load_prompt_frame_event_summary(
            &runtime.session_id,
            32,
            runtime.conversation_binding(),
            &runtime.memory_config,
        )
        .await
        .unwrap_or_default();
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = runtime;
        PromptFrameEventSummary::default()
    }
}

/// Refresh provider-facing runtime state from disk when a concrete config path
/// is still available for this runtime.
///
/// Long-lived channel/control-plane surfaces use this to pick up provider
/// credential/profile changes between turns without rebuilding the surrounding
/// runtime. If the runtime was created from an in-memory config snapshot only,
/// the existing config is reused as-is.
fn load_runtime_turn_config(
    runtime: &crate::chat::CliTurnRuntime,
) -> CliResult<crate::config::LoongConfig> {
    if runtime.resolved_path.as_os_str().is_empty() {
        return Ok(runtime.config.clone());
    }
    let path_exists = runtime
        .resolved_path
        .try_exists()
        .map_err(|error| format!("failed to access runtime config path: {error}"))?;
    if !path_exists {
        return Ok(runtime.config.clone());
    }
    runtime
        .config
        .reload_provider_runtime_state_from_path(runtime.resolved_path.as_path())
}

fn build_prompt_plans(summary: &PromptFrameEventSummary) -> (PromptAssemblyPlan, PromptCachePlan) {
    let prompt_assembly = PromptAssemblyPlan {
        total_estimated_tokens: summary.latest_total_estimated_tokens,
        stable_runtime_estimated_tokens: summary.latest_stable_runtime_estimated_tokens,
        session_latched_estimated_tokens: summary.latest_session_latched_estimated_tokens,
        session_local_recall_estimated_tokens: summary.latest_session_local_recall_estimated_tokens,
        turn_ephemeral_estimated_tokens: summary.latest_turn_ephemeral_estimated_tokens,
        stable_prefix_hash_sha256: summary.latest_stable_prefix_hash.clone(),
        cached_prefix_sha256: summary.latest_cached_prefix_hash.clone(),
    };
    let prompt_cache = PromptCachePlan {
        stable_prefix_hash_sha256: summary.latest_stable_prefix_hash.clone(),
        cached_prefix_sha256: summary.latest_cached_prefix_hash.clone(),
        stable_prefix_reused: summary.snapshot_events > 0
            && summary.stable_prefix_hash_change_events == 0,
        cached_prefix_reused: summary.snapshot_events > 0
            && summary.cached_prefix_hash_change_events == 0,
        session_latched_context_drifted: summary.session_latched_hash_change_events > 0,
        session_local_recall_drifted: summary.session_local_recall_hash_change_events > 0,
    };

    (prompt_assembly, prompt_cache)
}

#[cfg(feature = "memory-sqlite")]
fn parse_agent_runtime_events(payload: &Value) -> CliResult<Vec<AgentRuntimeEvent>> {
    let Some(events) = payload.get("events").and_then(Value::as_array) else {
        return Err("agent runtime session events payload missing `events` array".to_owned());
    };

    events
        .iter()
        .cloned()
        .map(|event| {
            serde_json::from_value::<AgentRuntimeEvent>(event)
                .map_err(|error| format!("parse agent runtime event failed: {error}"))
        })
        .collect()
}

#[cfg(feature = "memory-sqlite")]
pub async fn load_agent_runtime(
    config_path: Option<&str>,
    session_hint: Option<&str>,
) -> CliResult<(std::path::PathBuf, crate::config::LoongConfig, String)> {
    let (resolved_path, config) = load_config(config_path)?;
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        resolved_path.clone(),
        config.clone(),
        session_hint,
        &CliChatOptions::default(),
        "agent-runtime-load",
        crate::chat::CliSessionRequirement::AllowImplicitDefault,
        true,
    )?;
    Ok((resolved_path, config, runtime.session_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_ingress_context() -> ConversationIngressContext {
        ConversationIngressContext {
            channel: crate::conversation::ConversationIngressChannel {
                platform: "feishu".to_owned(),
                configured_account_id: Some("configured-account".to_owned()),
                account_id: Some("account".to_owned()),
                conversation_id: "conversation".to_owned(),
                participant_id: Some("participant".to_owned()),
                thread_id: Some("thread".to_owned()),
            },
            delivery: crate::conversation::ConversationIngressDelivery {
                source_message_id: None,
                sender_identity_key: Some("sender".to_owned()),
                thread_root_id: Some("thread-root".to_owned()),
                parent_message_id: Some("parent".to_owned()),
                resources: Vec::new(),
            },
            private: crate::conversation::ConversationIngressPrivateContext::default(),
        }
    }

    #[test]
    fn build_prompt_plans_flags_prefix_reuse_and_drift() {
        let summary = PromptFrameEventSummary {
            snapshot_events: 2,
            stable_prefix_hash_change_events: 0,
            cached_prefix_hash_change_events: 1,
            session_latched_hash_change_events: 1,
            session_local_recall_hash_change_events: 0,
            latest_total_estimated_tokens: Some(128),
            latest_stable_runtime_estimated_tokens: Some(40),
            latest_session_latched_estimated_tokens: Some(32),
            latest_session_local_recall_estimated_tokens: Some(16),
            latest_turn_ephemeral_estimated_tokens: Some(8),
            latest_stable_prefix_hash: Some("stable-a".to_owned()),
            latest_cached_prefix_hash: Some("cached-b".to_owned()),
            ..PromptFrameEventSummary::default()
        };

        let (prompt_assembly, prompt_cache) = build_prompt_plans(&summary);

        assert_eq!(prompt_assembly.total_estimated_tokens, Some(128));
        assert_eq!(
            prompt_cache.stable_prefix_hash_sha256.as_deref(),
            Some("stable-a")
        );
        assert!(prompt_cache.stable_prefix_reused);
        assert!(!prompt_cache.cached_prefix_reused);
        assert!(prompt_cache.session_latched_context_drifted);
        assert!(!prompt_cache.session_local_recall_drifted);
    }

    #[test]
    fn cli_chat_options_for_turn_request_ignores_blank_working_directory() {
        let request = AgentTurnRequest {
            acp_cwd: Some("   ".to_owned()),
            ..AgentTurnRequest::default()
        };

        let options = cli_chat_options_for_turn_request(&request);

        assert!(options.acp_working_directory.is_none());
        assert!(!options.acp_requested);
        assert!(!options.acp_event_stream);
        assert!(options.acp_bootstrap_mcp_servers.is_empty());
    }

    #[test]
    fn effective_turn_context_keeps_ingress_without_provenance() {
        let ingress = make_test_ingress_context();
        let provenance = AcpTurnProvenance::default();

        let (effective_ingress, effective_provenance) =
            effective_turn_context(Some(&ingress), provenance);

        assert!(effective_ingress.is_some());
        assert!(effective_provenance.trace_id.is_none());
        assert!(effective_provenance.source_message_id.is_none());
        assert!(effective_provenance.ack_cursor.is_none());
    }
}
