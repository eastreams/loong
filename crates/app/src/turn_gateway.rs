use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use crate::{
    CliResult, KernelContext,
    acp::{AcpSessionManager, AcpTurnEventSink, AcpTurnProvenance},
    agent_runtime::{
        AgentTurnMode, AgentTurnRequest, AgentTurnResult, TurnExecutionOptions,
        TurnExecutionService,
    },
    config::LoongConfig,
    conversation::{
        ConversationIngressContext, ConversationSessionAddress, ConversationTurnObserverHandle,
        ProviderErrorMode,
    },
    provider::ProviderRetryProgressCallback,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnGatewayProvenance {
    pub trace_id: Option<String>,
    pub source_message_id: Option<String>,
    pub ack_cursor: Option<String>,
}

impl TurnGatewayProvenance {
    pub fn as_acp_turn_provenance(&self) -> AcpTurnProvenance<'_> {
        AcpTurnProvenance {
            trace_id: self.trace_id.as_deref(),
            source_message_id: self.source_message_id.as_deref(),
            ack_cursor: self.ack_cursor.as_deref(),
        }
    }
}

pub struct TurnGatewayExecution<'a> {
    pub resolved_path: PathBuf,
    pub config: LoongConfig,
    pub kernel_ctx: Option<KernelContext>,
    pub acp_manager: Option<Arc<AcpSessionManager>>,
    pub event_sink: Option<&'a dyn AcpTurnEventSink>,
    pub initialize_runtime_environment: bool,
}

pub struct TurnGatewayRequest {
    pub address: ConversationSessionAddress,
    pub message: String,
    pub metadata: BTreeMap<String, String>,
    pub turn_mode: AgentTurnMode,
    pub acp_routing_intent: crate::acp::AcpRoutingIntent,
    pub acp_event_stream: bool,
    pub acp_bootstrap_mcp_servers: Vec<String>,
    pub acp_cwd: Option<String>,
    pub live_surface_enabled: bool,
    pub ingress: Option<ConversationIngressContext>,
    pub observer: Option<ConversationTurnObserverHandle>,
    pub provenance: TurnGatewayProvenance,
    pub provider_error_mode: ProviderErrorMode,
    pub retry_progress: ProviderRetryProgressCallback,
}

pub async fn run_turn_gateway(
    execution: TurnGatewayExecution<'_>,
    request: TurnGatewayRequest,
) -> CliResult<AgentTurnResult> {
    let agent_turn_request = build_agent_turn_request(&request)?;
    let session_hint = session_hint(request.address.session_id.as_str())?.to_owned();
    let TurnGatewayRequest {
        address: _,
        message: _,
        metadata: _,
        turn_mode: _,
        acp_routing_intent,
        acp_event_stream,
        acp_bootstrap_mcp_servers,
        acp_cwd,
        live_surface_enabled: _,
        ingress,
        observer,
        provenance,
        provider_error_mode,
        retry_progress,
    } = request;
    let mut turn_service = TurnExecutionService::new(execution.resolved_path, execution.config);
    if let Some(kernel_ctx) = execution.kernel_ctx {
        turn_service = turn_service.with_kernel_ctx(kernel_ctx);
    }
    if let Some(acp_manager) = execution.acp_manager {
        turn_service = turn_service.with_acp_manager(acp_manager);
    }
    if !execution.initialize_runtime_environment {
        turn_service = turn_service.without_runtime_environment_init();
    }
    let projection_request = TurnGatewayRequest {
        address: ConversationSessionAddress::from_session_id(session_hint.as_str()),
        message: String::new(),
        metadata: BTreeMap::new(),
        turn_mode: AgentTurnMode::Oneshot,
        acp_routing_intent,
        acp_event_stream,
        acp_bootstrap_mcp_servers,
        acp_cwd,
        live_surface_enabled: false,
        ingress,
        observer,
        provenance,
        provider_error_mode,
        retry_progress,
    };
    let turn_options = build_turn_execution_options(&projection_request, execution.event_sink);

    turn_service
        .execute(
            Some(session_hint.as_str()),
            &agent_turn_request,
            turn_options,
        )
        .await
}

fn session_hint(session_id: &str) -> CliResult<&str> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("turn gateway requires a non-empty session id".to_owned());
    }
    Ok(session_id)
}

pub fn build_agent_turn_request(request: &TurnGatewayRequest) -> CliResult<AgentTurnRequest> {
    session_hint(request.address.session_id.as_str())?;
    if request.message.trim().is_empty() {
        return Err("agent runtime message must not be empty".to_owned());
    }

    Ok(AgentTurnRequest {
        message: request.message.clone(),
        turn_mode: request.turn_mode,
        channel_id: request.address.channel_id.clone(),
        account_id: request.address.account_id.clone(),
        conversation_id: request.address.conversation_id.clone(),
        participant_id: request.address.participant_id.clone(),
        thread_id: request.address.thread_id.clone(),
        metadata: request.metadata.clone(),
        live_surface_enabled: request.live_surface_enabled,
    })
}

pub fn build_turn_execution_options<'a>(
    request: &'a TurnGatewayRequest,
    event_sink: Option<&'a dyn AcpTurnEventSink>,
) -> TurnExecutionOptions<'a> {
    TurnExecutionOptions {
        event_sink,
        observer: request.observer.clone(),
        ingress: request.ingress.as_ref(),
        provenance: request.provenance.as_acp_turn_provenance(),
        provider_error_mode: request.provider_error_mode,
        retry_progress: request.retry_progress.clone(),
        acp_routing_intent: request.acp_routing_intent,
        acp_event_stream: request.acp_event_stream,
        acp_bootstrap_mcp_servers: request.acp_bootstrap_mcp_servers.clone(),
        acp_working_directory: request.acp_cwd.clone().map(PathBuf::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_agent_turn_request_preserves_structured_session_scope() {
        let address = ConversationSessionAddress::from_session_id("session-1")
            .with_channel_scope("telegram", "chat-42")
            .with_account_id("ops-bot")
            .with_participant_id("alice")
            .with_thread_id("thread-7");
        let request = TurnGatewayRequest {
            address,
            message: "hello".to_owned(),
            metadata: BTreeMap::from([("trace".to_owned(), "abc".to_owned())]),
            turn_mode: AgentTurnMode::Oneshot,
            acp_routing_intent: crate::acp::AcpRoutingIntent::Explicit,
            acp_event_stream: true,
            acp_bootstrap_mcp_servers: vec!["mcp-1".to_owned()],
            acp_cwd: Some("/tmp/runtime".to_owned()),
            live_surface_enabled: false,
            ingress: None,
            observer: None,
            provenance: TurnGatewayProvenance::default(),
            provider_error_mode: ProviderErrorMode::InlineMessage,
            retry_progress: None,
        };

        let built = build_agent_turn_request(&request).expect("build turn gateway request");

        assert_eq!(built.message, "hello");
        assert_eq!(built.turn_mode, AgentTurnMode::Oneshot);
        assert_eq!(built.channel_id.as_deref(), Some("telegram"));
        assert_eq!(built.conversation_id.as_deref(), Some("chat-42"));
        assert_eq!(built.account_id.as_deref(), Some("ops-bot"));
        assert_eq!(built.participant_id.as_deref(), Some("alice"));
        assert_eq!(built.thread_id.as_deref(), Some("thread-7"));
        assert_eq!(built.metadata.get("trace").map(String::as_str), Some("abc"));
    }

    #[test]
    fn build_agent_turn_request_rejects_empty_session_id() {
        let request = TurnGatewayRequest {
            address: ConversationSessionAddress::from_session_id("   "),
            message: "hello".to_owned(),
            metadata: BTreeMap::new(),
            turn_mode: AgentTurnMode::Oneshot,
            acp_routing_intent: crate::acp::AcpRoutingIntent::Automatic,
            acp_event_stream: false,
            acp_bootstrap_mcp_servers: Vec::new(),
            acp_cwd: None,
            live_surface_enabled: false,
            ingress: None,
            observer: None,
            provenance: TurnGatewayProvenance::default(),
            provider_error_mode: ProviderErrorMode::InlineMessage,
            retry_progress: None,
        };

        let error = build_agent_turn_request(&request).expect_err("empty session id should fail");
        assert_eq!(error, "turn gateway requires a non-empty session id");
    }

    #[test]
    fn build_turn_execution_options_projects_acp_adapter_inputs() {
        let request = TurnGatewayRequest {
            address: ConversationSessionAddress::from_session_id("session-1"),
            message: "hello".to_owned(),
            metadata: BTreeMap::new(),
            turn_mode: AgentTurnMode::Oneshot,
            acp_routing_intent: crate::acp::AcpRoutingIntent::Explicit,
            acp_event_stream: true,
            acp_bootstrap_mcp_servers: vec!["filesystem".to_owned(), "search".to_owned()],
            acp_cwd: Some("/workspace/project".to_owned()),
            live_surface_enabled: false,
            ingress: None,
            observer: None,
            provenance: TurnGatewayProvenance {
                trace_id: Some("trace-1".to_owned()),
                source_message_id: Some("message-2".to_owned()),
                ack_cursor: Some("cursor-3".to_owned()),
            },
            provider_error_mode: ProviderErrorMode::InlineMessage,
            retry_progress: None,
        };

        let options = build_turn_execution_options(&request, None);

        assert_eq!(
            options.acp_routing_intent,
            crate::acp::AcpRoutingIntent::Explicit
        );
        assert!(options.acp_event_stream);
        assert_eq!(
            options.acp_bootstrap_mcp_servers,
            vec!["filesystem".to_owned(), "search".to_owned()]
        );
        assert_eq!(
            options.acp_working_directory,
            Some(PathBuf::from("/workspace/project"))
        );
        assert_eq!(options.provenance.trace_id, Some("trace-1"));
        assert_eq!(options.provenance.source_message_id, Some("message-2"));
        assert_eq!(options.provenance.ack_cursor, Some("cursor-3"));
    }
}
