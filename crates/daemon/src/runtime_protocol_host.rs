use async_trait::async_trait;
use loong_app_protocol::{
    AppProtocolRuntimeExecutorRequest, AppProtocolRuntimeExecutorResult, AppProtocolRuntimeHost,
    AppProtocolRuntimeInteractiveExecutorRequest, AppProtocolRuntimeInteractiveExecutorResult,
    RuntimeExecutorConfig, RuntimeRoutedExecutorRequest, executor_requested_config_path,
    executor_resolved_config_path,
};
use std::path::PathBuf;
use std::sync::Arc;

use crate::mvp::{
    self, CliResult,
    acp::AcpRoutingIntent,
    acp::{AcpSessionManager, AcpTurnEventSink},
    agent_runtime::{
        AgentRuntime, AgentTurnMode, AgentTurnRequest, TurnExecutionOptions, TurnExecutionService,
    },
};

pub struct LoongAppRuntimeProtocolHost<'a> {
    acp_manager: Option<Arc<AcpSessionManager>>,
    event_sink: Option<&'a dyn AcpTurnEventSink>,
    loaded_config: Option<mvp::config::LoongConfig>,
}

impl<'a> LoongAppRuntimeProtocolHost<'a> {
    pub const fn new() -> Self {
        Self {
            acp_manager: None,
            event_sink: None,
            loaded_config: None,
        }
    }

    pub fn with_acp_manager(mut self, acp_manager: Arc<AcpSessionManager>) -> Self {
        self.acp_manager = Some(acp_manager);
        self
    }

    pub fn with_event_sink(mut self, event_sink: &'a dyn AcpTurnEventSink) -> Self {
        self.event_sink = Some(event_sink);
        self
    }

    pub fn with_loaded_config(mut self, loaded_config: mvp::config::LoongConfig) -> Self {
        self.loaded_config = Some(loaded_config);
        self
    }
}

#[async_trait]
impl AppProtocolRuntimeHost for LoongAppRuntimeProtocolHost<'_> {
    async fn execute_oneshot_request(
        &self,
        config: &RuntimeExecutorConfig,
        request: AppProtocolRuntimeExecutorRequest,
    ) -> Result<AppProtocolRuntimeExecutorResult, String> {
        let resolved_path = executor_resolved_config_path(config).to_path_buf();
        let loaded = load_runtime_config(config, self.loaded_config.as_ref())?;
        let result = AgentRuntime::new()
            .run_turn_with_loaded_config(
                resolved_path,
                loaded,
                request.session_hint.as_deref(),
                &AgentTurnRequest {
                    message: request.message,
                    turn_mode: AgentTurnMode::Oneshot,
                    ..AgentTurnRequest::default()
                },
                None,
            )
            .await?;

        Ok(AppProtocolRuntimeExecutorResult {
            session_id: result.session_id,
            output_text: result.output_text,
            state: result.state,
            stop_reason: result.stop_reason,
            usage: result.usage,
            event_count: result.event_count,
        })
    }

    async fn execute_interactive_request(
        &self,
        config: &RuntimeExecutorConfig,
        request: AppProtocolRuntimeInteractiveExecutorRequest,
    ) -> Result<AppProtocolRuntimeInteractiveExecutorResult, String> {
        mvp::chat::run_cli_chat(
            executor_requested_config_path(config),
            request.session_hint.as_deref(),
            &mvp::chat::CliChatOptions::default(),
        )
        .await?;

        Ok(AppProtocolRuntimeInteractiveExecutorResult {
            session_id: resolved_interactive_session_id(config, request.session_hint.as_deref())?,
            exit_state: "completed".to_owned(),
        })
    }

    async fn execute_routed_oneshot_request(
        &self,
        config: &RuntimeExecutorConfig,
        request: RuntimeRoutedExecutorRequest,
    ) -> Result<AppProtocolRuntimeExecutorResult, String> {
        let resolved_path = executor_resolved_config_path(config).to_path_buf();
        let loaded = load_runtime_config(config, self.loaded_config.as_ref())?;
        let turn_service = TurnExecutionService::new(resolved_path, loaded)
            .with_acp_manager(
                self.acp_manager
                    .clone()
                    .unwrap_or_else(|| Arc::new(AcpSessionManager::default())),
            )
            .without_runtime_environment_init();
        let result = turn_service
            .execute(
                request.session_hint.as_deref(),
                &AgentTurnRequest {
                    message: request.message,
                    turn_mode: AgentTurnMode::Oneshot,
                    channel_id: request.channel_id,
                    account_id: request.account_id,
                    conversation_id: request.conversation_id,
                    participant_id: request.participant_id,
                    thread_id: request.thread_id,
                    metadata: request.metadata,
                    live_surface_enabled: false,
                },
                TurnExecutionOptions {
                    event_sink: self.event_sink,
                    observer: None,
                    ingress: None,
                    provenance: mvp::acp::AcpTurnProvenance::default(),
                    provider_error_mode: mvp::conversation::ProviderErrorMode::InlineMessage,
                    retry_progress: None,
                    acp_routing_intent: if request.acp_requested {
                        AcpRoutingIntent::Explicit
                    } else {
                        AcpRoutingIntent::Automatic
                    },
                    acp_event_stream: request.acp_event_stream,
                    acp_bootstrap_mcp_servers: Vec::new(),
                    acp_working_directory: request.working_directory.as_deref().map(PathBuf::from),
                },
            )
            .await?;

        Ok(AppProtocolRuntimeExecutorResult {
            session_id: result.session_id,
            output_text: result.output_text,
            state: result.state,
            stop_reason: result.stop_reason,
            usage: result.usage,
            event_count: result.event_count,
        })
    }

    async fn resolve_latest_root_session_id(
        &self,
        config: &RuntimeExecutorConfig,
    ) -> Result<Option<String>, String> {
        #[cfg(feature = "memory-sqlite")]
        {
            let loaded = load_runtime_config(config, self.loaded_config.as_ref())?;
            let memory_config =
                mvp::session::store::SessionStoreConfig::from_memory_config(&loaded.memory);
            return mvp::session::latest_resumable_root_session_id(&memory_config);
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = config;
            Ok(None)
        }
    }
}

fn load_runtime_config(
    config: &RuntimeExecutorConfig,
    loaded_override: Option<&mvp::config::LoongConfig>,
) -> CliResult<mvp::config::LoongConfig> {
    if let Some(loaded_override) = loaded_override {
        return Ok(loaded_override.clone());
    }

    let path = executor_resolved_config_path(config);
    if path
        .try_exists()
        .map_err(|error| format!("failed to access config path {}: {error}", path.display()))?
    {
        let path_string = path.to_string_lossy().into_owned();
        let (_resolved_path, config) = mvp::config::load(Some(path_string.as_str()))?;
        return Ok(config);
    }

    let mut config = mvp::config::LoongConfig::default();
    let runtime_workspace_root = std::env::current_dir()
        .ok()
        .unwrap_or_else(|| config.tools.resolved_file_root());
    let runtime_workspace_root =
        dunce::canonicalize(&runtime_workspace_root).unwrap_or(runtime_workspace_root);
    config.tools.runtime_workspace_root = Some(runtime_workspace_root.display().to_string());
    Ok(config)
}

fn resolved_interactive_session_id(
    config: &RuntimeExecutorConfig,
    session_hint: Option<&str>,
) -> CliResult<String> {
    let session_id = session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default");

    if config.latest_session_selector.as_deref() == Some(session_id) {
        let latest = latest_root_session_id(config)?;
        return latest.ok_or_else(|| {
            "CLI session selector `latest` did not find any resumable root session".to_owned()
        });
    }

    Ok(session_id.to_owned())
}

fn latest_root_session_id(config: &RuntimeExecutorConfig) -> CliResult<Option<String>> {
    #[cfg(feature = "memory-sqlite")]
    {
        let loaded = load_runtime_config(config, None)?;
        let memory_config =
            mvp::session::store::SessionStoreConfig::from_memory_config(&loaded.memory);
        mvp::session::latest_resumable_root_session_id(&memory_config)
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = config;
        Ok(None)
    }
}
