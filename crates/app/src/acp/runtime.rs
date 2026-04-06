use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde_json::Value;

use crate::CliResult;
use crate::config::{
    AcpConversationRoutingMode, AcpDispatchThreadRoutingMode, LoongClawConfig,
    normalize_dispatch_account_id, normalize_dispatch_channel_id,
};
use crate::conversation::{ConversationSessionAddress, parse_route_session_id};

use super::AcpSessionManager;
use super::analytics::PersistedAcpRuntimeEventContext;
use super::backend::{
    ACP_SESSION_METADATA_ACTIVATION_ORIGIN, ACP_TURN_METADATA_ROUTING_INTENT,
    ACP_TURN_METADATA_ROUTING_ORIGIN, AcpBackendMetadata, AcpConversationTurnOptions,
    AcpRoutingIntent, AcpRoutingOrigin, AcpSessionBootstrap, AcpSessionMode, AcpTurnRequest,
    AcpTurnResult, BufferedAcpTurnEventSink, CompositeAcpTurnEventSink,
};
use super::binding::AcpSessionBindingScope;
use super::merge_turn_events;
use super::registry::{
    DEFAULT_ACP_BACKEND_ID, acp_backend_id_from_env, describe_acp_backend,
    list_acp_backend_metadata,
};
#[cfg(feature = "memory-sqlite")]
use super::store::AcpSqliteSessionStore;
#[cfg(not(feature = "memory-sqlite"))]
use super::store::InMemoryAcpSessionStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpConversationRoute {
    pub conversation_id: String,
    pub agent_id: String,
    pub session_key: String,
    pub binding: Option<AcpSessionBindingScope>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpConversationDispatchTarget {
    pub original_session_id: String,
    pub route_session_id: String,
    pub prefixed_agent_id: Option<String>,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<String>,
    pub channel_path: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpConversationDispatchReason {
    Allowed,
    DispatchDisabled,
    ChannelNotAllowed,
    AccountNotAllowed,
    AgentPrefixRequired,
    ThreadRequired,
    RootConversationRequired,
}

impl AcpConversationDispatchReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::DispatchDisabled => "dispatch_disabled",
            Self::ChannelNotAllowed => "channel_not_allowed",
            Self::AccountNotAllowed => "account_not_allowed",
            Self::AgentPrefixRequired => "agent_prefix_required",
            Self::ThreadRequired => "thread_required",
            Self::RootConversationRequired => "root_conversation_required",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpConversationDispatchDecision {
    pub route_via_acp: bool,
    pub reason: AcpConversationDispatchReason,
    pub automatic_routing_origin: Option<AcpRoutingOrigin>,
    pub target: AcpConversationDispatchTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAcpConversationTurn {
    pub route: AcpConversationRoute,
    pub routing_origin: AcpRoutingOrigin,
    pub bootstrap: AcpSessionBootstrap,
    pub request: AcpTurnRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpConversationTurnEntryDecision {
    RouteViaAcp,
    StayOnProvider,
    RejectExplicitWhenDisabled,
}

impl AcpConversationTurnEntryDecision {
    pub const fn routes_via_acp(self) -> bool {
        matches!(self, Self::RouteViaAcp)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExecutedAcpConversationTurn {
    pub prepared: PreparedAcpConversationTurn,
    pub backend_selection: AcpBackendSelection,
    pub persistence_context: PersistedAcpRuntimeEventContext,
    pub outcome: AcpConversationTurnExecutionOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AcpConversationTurnExecutionOutcome {
    Succeeded(AcpConversationTurnSuccess),
    Failed(AcpConversationTurnFailure),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AcpConversationTurnSuccess {
    pub result: AcpTurnResult,
    pub runtime_events: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcpConversationTurnFailure {
    pub error: String,
    pub runtime_events: Vec<Value>,
}

static ACP_SESSION_MANAGER_REGISTRY: OnceLock<RwLock<BTreeMap<String, Arc<AcpSessionManager>>>> =
    OnceLock::new();

fn manager_registry() -> &'static RwLock<BTreeMap<String, Arc<AcpSessionManager>>> {
    ACP_SESSION_MANAGER_REGISTRY.get_or_init(|| RwLock::new(BTreeMap::new()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpBackendSelectionSource {
    Env,
    Config,
    Default,
}

impl AcpBackendSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Config => "config",
            Self::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpBackendSelection {
    pub id: String,
    pub source: AcpBackendSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpControlPlaneSnapshot {
    pub enabled: bool,
    pub dispatch_enabled: bool,
    pub conversation_routing: AcpConversationRoutingMode,
    pub allowed_channels: Vec<String>,
    pub allowed_account_ids: Vec<String>,
    pub bootstrap_mcp_servers: Vec<String>,
    pub working_directory: Option<String>,
    pub thread_routing: AcpDispatchThreadRoutingMode,
    pub default_agent: String,
    pub allowed_agents: Vec<String>,
    pub max_concurrent_sessions: usize,
    pub session_idle_ttl_ms: u64,
    pub startup_timeout_ms: u64,
    pub turn_timeout_ms: u64,
    pub queue_owner_ttl_ms: u64,
    pub bindings_enabled: bool,
    pub emit_runtime_events: bool,
    pub allow_mcp_server_injection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpRuntimeSnapshot {
    pub selected: AcpBackendSelection,
    pub selected_metadata: AcpBackendMetadata,
    pub available: Vec<AcpBackendMetadata>,
    pub control_plane: AcpControlPlaneSnapshot,
    pub mcp: crate::mcp::McpRuntimeSnapshot,
}

pub fn resolve_acp_backend_selection(config: &LoongClawConfig) -> AcpBackendSelection {
    if let Some(id) = acp_backend_id_from_env() {
        return AcpBackendSelection {
            id,
            source: AcpBackendSelectionSource::Env,
        };
    }

    if let Some(id) = config.acp.backend_id() {
        return AcpBackendSelection {
            id,
            source: AcpBackendSelectionSource::Config,
        };
    }

    AcpBackendSelection {
        id: DEFAULT_ACP_BACKEND_ID.to_owned(),
        source: AcpBackendSelectionSource::Default,
    }
}

pub fn collect_acp_runtime_snapshot(config: &LoongClawConfig) -> CliResult<AcpRuntimeSnapshot> {
    let selected = resolve_acp_backend_selection(config);
    let selected_metadata = describe_acp_backend(Some(selected.id.as_str()))?;
    let available = list_acp_backend_metadata()?;
    let mcp = crate::mcp::collect_mcp_runtime_snapshot(config)?;
    let default_agent = config.acp.resolved_default_agent()?;
    let allowed_agents = config.acp.allowed_agent_ids()?;
    let allowed_channels = config.acp.dispatch.allowed_channel_ids()?;
    let allowed_account_ids = config.acp.dispatch.allowed_account_ids()?;
    let bootstrap_mcp_servers = config.acp.dispatch.bootstrap_mcp_server_names()?;
    let working_directory = config
        .acp
        .dispatch
        .resolved_working_directory()
        .map(|path| path.display().to_string());
    let control_plane = AcpControlPlaneSnapshot {
        enabled: config.acp.enabled,
        dispatch_enabled: config.acp.dispatch_enabled(),
        conversation_routing: config.acp.dispatch.conversation_routing,
        allowed_channels,
        allowed_account_ids,
        bootstrap_mcp_servers,
        working_directory,
        thread_routing: config.acp.dispatch.thread_routing,
        default_agent,
        allowed_agents,
        max_concurrent_sessions: config.acp.max_concurrent_sessions(),
        session_idle_ttl_ms: config.acp.session_idle_ttl_ms(),
        startup_timeout_ms: config.acp.startup_timeout_ms(),
        turn_timeout_ms: config.acp.turn_timeout_ms(),
        queue_owner_ttl_ms: config.acp.queue_owner_ttl_ms(),
        bindings_enabled: config.acp.bindings_enabled,
        emit_runtime_events: config.acp.emit_runtime_events,
        allow_mcp_server_injection: config.acp.allow_mcp_server_injection,
    };

    Ok(AcpRuntimeSnapshot {
        selected,
        selected_metadata,
        available,
        control_plane,
        mcp,
    })
}

pub fn describe_acp_conversation_dispatch_target(
    session_id: &str,
) -> CliResult<AcpConversationDispatchTarget> {
    let address = ConversationSessionAddress::from_session_id(session_id);
    describe_acp_conversation_dispatch_target_for_address(&address)
}

pub fn describe_acp_conversation_dispatch_target_for_address(
    address: &ConversationSessionAddress,
) -> CliResult<AcpConversationDispatchTarget> {
    let original_session_id = address.session_id.trim();
    if original_session_id.is_empty() {
        return Err("ACP conversation dispatch target requires a non-empty session id".to_owned());
    }

    let (prefixed_agent_id, parsed_route_session_id) = if let Some((agent_id, route_session_id)) =
        parse_agent_prefixed_route_session_id(original_session_id)
    {
        (Some(agent_id.to_owned()), route_session_id.to_owned())
    } else {
        (None, original_session_id.to_owned())
    };

    let explicit_channel_id = address.canonical_channel_id();
    let explicit_account_id = address
        .account_id
        .as_deref()
        .and_then(normalize_dispatch_account_id);
    let explicit_conversation_id = trimmed_non_empty(address.conversation_id.as_deref());
    let explicit_thread_id = trimmed_non_empty(address.thread_id.as_deref());
    let explicit_channel_path = address.structured_channel_path();
    let explicit_route_session_id = address.structured_route_session_id();

    let (route_session_id, channel_id, account_id, conversation_id, thread_id, channel_path) =
        if let Some(channel_id) = explicit_channel_id {
            let route_session_id = explicit_route_session_id.unwrap_or_else(|| channel_id.clone());
            (
                route_session_id,
                Some(channel_id),
                explicit_account_id,
                explicit_conversation_id,
                explicit_thread_id,
                explicit_channel_path,
            )
        } else if let Some((parsed_channel_id, channel_path)) =
            parse_route_session_id(parsed_route_session_id.as_str())?
        {
            if let Some(channel_id) = normalize_dispatch_channel_id(parsed_channel_id.as_str()) {
                (
                    parsed_route_session_id,
                    Some(channel_id),
                    None,
                    None,
                    None,
                    channel_path,
                )
            } else {
                (parsed_route_session_id, None, None, None, None, Vec::new())
            }
        } else {
            (parsed_route_session_id, None, None, None, None, Vec::new())
        };

    Ok(AcpConversationDispatchTarget {
        original_session_id: original_session_id.to_owned(),
        route_session_id,
        prefixed_agent_id,
        channel_id,
        account_id,
        conversation_id,
        thread_id,
        channel_path,
    })
}

pub fn should_route_conversation_turn_via_acp(
    config: &LoongClawConfig,
    session_id: &str,
) -> CliResult<bool> {
    let address = ConversationSessionAddress::from_session_id(session_id);
    should_route_conversation_turn_via_acp_for_address(config, &address)
}

pub fn should_route_conversation_turn_via_acp_for_address(
    config: &LoongClawConfig,
    address: &ConversationSessionAddress,
) -> CliResult<bool> {
    Ok(evaluate_acp_conversation_dispatch_for_address(config, address)?.route_via_acp)
}

pub fn evaluate_acp_conversation_turn_entry_for_address(
    config: &LoongClawConfig,
    address: &ConversationSessionAddress,
    options: &AcpConversationTurnOptions<'_>,
) -> CliResult<AcpConversationTurnEntryDecision> {
    match options.routing_intent {
        AcpRoutingIntent::Explicit if !config.acp.enabled => {
            Ok(AcpConversationTurnEntryDecision::RejectExplicitWhenDisabled)
        }
        AcpRoutingIntent::Explicit => Ok(AcpConversationTurnEntryDecision::RouteViaAcp),
        AcpRoutingIntent::Automatic => {
            if should_route_conversation_turn_via_acp_for_address(config, address)? {
                Ok(AcpConversationTurnEntryDecision::RouteViaAcp)
            } else {
                Ok(AcpConversationTurnEntryDecision::StayOnProvider)
            }
        }
    }
}

pub(crate) async fn execute_acp_conversation_turn_for_address(
    config: &LoongClawConfig,
    address: &ConversationSessionAddress,
    user_input: &str,
    options: &AcpConversationTurnOptions<'_>,
) -> CliResult<ExecutedAcpConversationTurn> {
    let prepared = prepare_acp_conversation_turn_for_address(config, address, user_input, options)?;
    let manager = shared_acp_session_manager(config)?;
    let backend_selection = resolve_acp_backend_selection(config);
    let persistence_event_sink = config
        .acp
        .emit_runtime_events
        .then(BufferedAcpTurnEventSink::default);

    let turn_result = match (options.event_sink, persistence_event_sink.as_ref()) {
        (Some(external_sink), Some(persistence_sink)) => {
            let composite_sink = CompositeAcpTurnEventSink {
                primary: external_sink,
                secondary: persistence_sink,
            };
            manager
                .run_turn_with_sink(
                    config,
                    &prepared.bootstrap,
                    &prepared.request,
                    Some(&composite_sink),
                )
                .await
        }
        (Some(external_sink), None) => {
            manager
                .run_turn_with_sink(
                    config,
                    &prepared.bootstrap,
                    &prepared.request,
                    Some(external_sink),
                )
                .await
        }
        (None, Some(persistence_sink)) => {
            manager
                .run_turn_with_sink(
                    config,
                    &prepared.bootstrap,
                    &prepared.request,
                    Some(persistence_sink),
                )
                .await
        }
        (None, None) => {
            manager
                .run_turn(config, &prepared.bootstrap, &prepared.request)
                .await
        }
    };

    let outcome = match turn_result {
        Ok(result) => {
            let runtime_events = if let Some(persistence_sink) = persistence_event_sink.as_ref() {
                let streamed_events = persistence_sink.snapshot()?;
                merge_turn_events(&result.events, &streamed_events)
            } else {
                result.events.clone()
            };
            AcpConversationTurnExecutionOutcome::Succeeded(AcpConversationTurnSuccess {
                result,
                runtime_events,
            })
        }
        Err(error) => {
            let runtime_events = persistence_event_sink
                .as_ref()
                .map(BufferedAcpTurnEventSink::snapshot)
                .transpose()?
                .unwrap_or_default();
            AcpConversationTurnExecutionOutcome::Failed(AcpConversationTurnFailure {
                error,
                runtime_events,
            })
        }
    };

    let persistence_context = PersistedAcpRuntimeEventContext {
        backend_id: backend_selection.id.clone(),
        agent_id: prepared.route.agent_id.clone(),
        session_key: prepared.request.session_key.clone(),
        conversation_id: Some(prepared.route.conversation_id.clone()),
        binding: prepared.route.binding.clone(),
        request_metadata: prepared.request.metadata.clone(),
    };

    Ok(ExecutedAcpConversationTurn {
        prepared,
        backend_selection,
        persistence_context,
        outcome,
    })
}

pub fn derive_automatic_acp_routing_origin_for_address(
    address: &ConversationSessionAddress,
) -> CliResult<AcpRoutingOrigin> {
    let target = describe_acp_conversation_dispatch_target_for_address(address)?;
    Ok(automatic_routing_origin_for_target(&target))
}

pub fn prepare_acp_conversation_turn_for_address(
    config: &LoongClawConfig,
    address: &ConversationSessionAddress,
    user_input: &str,
    options: &AcpConversationTurnOptions<'_>,
) -> CliResult<PreparedAcpConversationTurn> {
    let route = derive_acp_conversation_route_for_address(config, address)?;
    let routing_origin = match options.routing_intent {
        AcpRoutingIntent::Explicit => AcpRoutingOrigin::ExplicitRequest,
        AcpRoutingIntent::Automatic => derive_automatic_acp_routing_origin_for_address(address)?,
    };
    let mut bootstrap_metadata = route.metadata.clone();
    insert_trimmed_metadata(
        &mut bootstrap_metadata,
        ACP_SESSION_METADATA_ACTIVATION_ORIGIN,
        Some(routing_origin.as_str()),
    );
    let mut request_metadata = route.metadata.clone();
    insert_trimmed_metadata(
        &mut request_metadata,
        ACP_TURN_METADATA_ROUTING_INTENT,
        Some(options.routing_intent.as_str()),
    );
    insert_trimmed_metadata(
        &mut request_metadata,
        ACP_TURN_METADATA_ROUTING_ORIGIN,
        Some(routing_origin.as_str()),
    );
    options
        .provenance
        .extend_request_metadata(&mut request_metadata);
    let additional_bootstrap_mcp_servers = options.additional_bootstrap_mcp_servers.unwrap_or(&[]);
    let bootstrap_mcp_servers = config
        .acp
        .dispatch
        .bootstrap_mcp_server_names_with_additions(additional_bootstrap_mcp_servers)?;
    let effective_working_directory = options
        .working_directory
        .map(|path| path.to_path_buf())
        .or_else(|| config.acp.dispatch.resolved_working_directory());
    let bootstrap = AcpSessionBootstrap {
        session_key: route.session_key.clone(),
        conversation_id: Some(route.conversation_id.clone()),
        binding: route.binding.clone(),
        working_directory: effective_working_directory.clone(),
        initial_prompt: None,
        mode: Some(AcpSessionMode::Interactive),
        mcp_servers: bootstrap_mcp_servers,
        metadata: bootstrap_metadata,
    };
    let request = AcpTurnRequest {
        session_key: route.session_key.clone(),
        input: user_input.to_owned(),
        working_directory: effective_working_directory,
        metadata: request_metadata,
    };

    Ok(PreparedAcpConversationTurn {
        route,
        routing_origin,
        bootstrap,
        request,
    })
}

pub fn evaluate_acp_conversation_dispatch_for_address(
    config: &LoongClawConfig,
    address: &ConversationSessionAddress,
) -> CliResult<AcpConversationDispatchDecision> {
    let target = describe_acp_conversation_dispatch_target_for_address(address)?;
    if !config.acp.dispatch_enabled() {
        return Ok(AcpConversationDispatchDecision {
            route_via_acp: false,
            reason: AcpConversationDispatchReason::DispatchDisabled,
            automatic_routing_origin: None,
            target,
        });
    }

    if !config
        .acp
        .dispatch
        .allows_channel_id(target.channel_id.as_deref())?
    {
        return Ok(AcpConversationDispatchDecision {
            route_via_acp: false,
            reason: AcpConversationDispatchReason::ChannelNotAllowed,
            automatic_routing_origin: None,
            target,
        });
    }

    if !config
        .acp
        .dispatch
        .allows_account_id(target.account_id.as_deref())?
    {
        return Ok(AcpConversationDispatchDecision {
            route_via_acp: false,
            reason: AcpConversationDispatchReason::AccountNotAllowed,
            automatic_routing_origin: None,
            target,
        });
    }

    if !config
        .acp
        .dispatch
        .allows_thread_id(target.thread_id.as_deref())
    {
        let reason = match config.acp.dispatch.thread_routing {
            AcpDispatchThreadRoutingMode::All => AcpConversationDispatchReason::Allowed,
            AcpDispatchThreadRoutingMode::ThreadOnly => {
                AcpConversationDispatchReason::ThreadRequired
            }
            AcpDispatchThreadRoutingMode::RootOnly => {
                AcpConversationDispatchReason::RootConversationRequired
            }
        };
        return Ok(AcpConversationDispatchDecision {
            route_via_acp: false,
            reason,
            automatic_routing_origin: None,
            target,
        });
    }

    let reason = match config.acp.dispatch.conversation_routing {
        AcpConversationRoutingMode::All => AcpConversationDispatchReason::Allowed,
        AcpConversationRoutingMode::AgentPrefixedOnly if target.prefixed_agent_id.is_some() => {
            AcpConversationDispatchReason::Allowed
        }
        AcpConversationRoutingMode::AgentPrefixedOnly => {
            AcpConversationDispatchReason::AgentPrefixRequired
        }
    };
    let route_via_acp = reason == AcpConversationDispatchReason::Allowed;

    Ok(AcpConversationDispatchDecision {
        route_via_acp,
        reason,
        automatic_routing_origin: route_via_acp
            .then(|| automatic_routing_origin_for_target(&target)),
        target,
    })
}

pub fn derive_acp_conversation_route(
    config: &LoongClawConfig,
    session_id: &str,
) -> CliResult<AcpConversationRoute> {
    let address = ConversationSessionAddress::from_session_id(session_id);
    derive_acp_conversation_route_for_address(config, &address)
}

pub fn derive_acp_conversation_route_for_address(
    config: &LoongClawConfig,
    address: &ConversationSessionAddress,
) -> CliResult<AcpConversationRoute> {
    let target = describe_acp_conversation_dispatch_target_for_address(address)?;
    let conversation_id = target.original_session_id.as_str();

    let agent_id = if let Some(agent_id) = session_key_agent_id(conversation_id) {
        config.acp.resolve_allowed_agent(agent_id)?
    } else {
        config.acp.resolved_default_agent()?
    };
    let session_key = if conversation_id.starts_with("agent:") {
        conversation_id.to_owned()
    } else {
        format!("agent:{agent_id}:{conversation_id}")
    };
    let mut metadata = BTreeMap::from([
        ("conversation_id".to_owned(), conversation_id.to_owned()),
        ("acp_agent".to_owned(), agent_id.clone()),
        ("origin_session_id".to_owned(), conversation_id.to_owned()),
        (
            "route_session_id".to_owned(),
            target.route_session_id.clone(),
        ),
        ("route_kind".to_owned(), "conversation".to_owned()),
    ]);
    if let Some(channel_id) = target.channel_id.as_deref() {
        metadata.insert("channel".to_owned(), channel_id.to_owned());
    }
    if let Some(channel_identity) = target.channel_path.first() {
        metadata.insert("channel_identity".to_owned(), channel_identity.to_owned());
    }
    if !target.channel_path.is_empty() {
        metadata.insert("channel_scope".to_owned(), target.channel_path.join(":"));
        metadata.insert(
            "channel_scope_depth".to_owned(),
            target.channel_path.len().to_string(),
        );
    }
    if let Some(account_id) = target.account_id.as_deref() {
        metadata.insert("channel_account_id".to_owned(), account_id.to_owned());
    }
    if let Some(conversation_id) = target.conversation_id.as_deref() {
        metadata.insert(
            "channel_conversation_id".to_owned(),
            conversation_id.to_owned(),
        );
    }
    if let Some(thread_id) = target.thread_id.as_deref() {
        metadata.insert("channel_thread_id".to_owned(), thread_id.to_owned());
    }
    let binding = binding_scope_from_dispatch_target(&target);

    Ok(AcpConversationRoute {
        conversation_id: conversation_id.to_owned(),
        agent_id,
        session_key,
        binding,
        metadata,
    })
}

fn binding_scope_from_dispatch_target(
    target: &AcpConversationDispatchTarget,
) -> Option<AcpSessionBindingScope> {
    target.channel_id.as_ref()?;
    Some(AcpSessionBindingScope {
        route_session_id: target.route_session_id.clone(),
        channel_id: target.channel_id.clone(),
        account_id: target.account_id.clone(),
        conversation_id: target.conversation_id.clone(),
        thread_id: target.thread_id.clone(),
    })
}

fn session_key_agent_id(session_key: &str) -> Option<&str> {
    parse_agent_prefixed_route_session_id(session_key).map(|(agent, _rest)| agent)
}

fn automatic_routing_origin_for_target(target: &AcpConversationDispatchTarget) -> AcpRoutingOrigin {
    if target.prefixed_agent_id.is_some() {
        AcpRoutingOrigin::AutomaticAgentPrefixed
    } else {
        AcpRoutingOrigin::AutomaticDispatch
    }
}

fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn insert_trimmed_metadata(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    value: Option<&str>,
) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    metadata.insert(key.to_owned(), value.to_owned());
}

fn parse_agent_prefixed_route_session_id(session_id: &str) -> Option<(&str, &str)> {
    session_id
        .strip_prefix("agent:")
        .and_then(|remainder| remainder.split_once(':'))
        .map(|(agent, route_session_id)| (agent.trim(), route_session_id.trim()))
        .filter(|(agent, route_session_id)| !agent.is_empty() && !route_session_id.is_empty())
}

pub fn shared_acp_session_manager(config: &LoongClawConfig) -> CliResult<Arc<AcpSessionManager>> {
    #[cfg(feature = "memory-sqlite")]
    {
        let sqlite_path = config.memory.resolved_sqlite_path();
        let key = sqlite_path.display().to_string();
        if let Some(existing) = manager_registry()
            .read()
            .map_err(|_error| "ACP session manager registry lock poisoned".to_owned())?
            .get(&key)
            .cloned()
        {
            return Ok(existing);
        }

        let manager = Arc::new(AcpSessionManager::new(Arc::new(
            AcpSqliteSessionStore::new(Some(sqlite_path)),
        )));
        let mut guard = manager_registry()
            .write()
            .map_err(|_error| "ACP session manager registry lock poisoned".to_owned())?;
        Ok(guard.entry(key).or_insert_with(|| manager.clone()).clone())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let key = "in-memory".to_owned();
        if let Some(existing) = manager_registry()
            .read()
            .map_err(|_error| "ACP session manager registry lock poisoned".to_owned())?
            .get(&key)
            .cloned()
        {
            return Ok(existing);
        }

        let manager = Arc::new(AcpSessionManager::new(Arc::new(
            InMemoryAcpSessionStore::default(),
        )));
        let mut guard = manager_registry()
            .write()
            .map_err(|_error| "ACP session manager registry lock poisoned".to_owned())?;
        Ok(guard.entry(key).or_insert_with(|| manager.clone()).clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;
    use crate::config::{AcpConfig, AcpConversationRoutingMode, ConversationConfig};
    use crate::conversation::ConversationSessionAddress;

    fn acp_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_acp_backend_selection_uses_default_when_unset() {
        let _env_lock = acp_env_lock().lock().expect("env lock");
        super::super::registry::clear_acp_backend_env_override();

        let config = LoongClawConfig::default();
        let selection = resolve_acp_backend_selection(&config);

        assert_eq!(selection.id, DEFAULT_ACP_BACKEND_ID);
        assert_eq!(selection.source, AcpBackendSelectionSource::Default);
    }

    #[test]
    fn resolve_acp_backend_selection_prefers_env_over_config() {
        let _env_lock = acp_env_lock().lock().expect("env lock");
        super::super::registry::set_acp_backend_env_override(Some("env-backend"));

        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("config-backend".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let selection = resolve_acp_backend_selection(&config);
        assert_eq!(selection.id, "env-backend");
        assert_eq!(selection.source, AcpBackendSelectionSource::Env);
        super::super::registry::clear_acp_backend_env_override();
    }

    #[test]
    fn resolve_acp_backend_selection_uses_config_when_env_missing() {
        let _env_lock = acp_env_lock().lock().expect("env lock");
        super::super::registry::clear_acp_backend_env_override();

        let config = LoongClawConfig {
            acp: AcpConfig {
                backend: Some("config-backend".to_owned()),
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let selection = resolve_acp_backend_selection(&config);
        assert_eq!(selection.id, "config-backend");
        assert_eq!(selection.source, AcpBackendSelectionSource::Config);
    }

    #[test]
    fn collect_acp_runtime_snapshot_reports_control_plane_defaults() {
        let _env_lock = acp_env_lock().lock().expect("env lock");
        super::super::registry::clear_acp_backend_env_override();

        let config = LoongClawConfig {
            conversation: ConversationConfig::default(),
            acp: AcpConfig {
                enabled: true,
                default_agent: Some("claude".to_owned()),
                allowed_agents: vec!["codex".to_owned(), "claude".to_owned()],
                max_concurrent_sessions: Some(6),
                session_idle_ttl_ms: Some(45_000),
                startup_timeout_ms: Some(10_000),
                turn_timeout_ms: Some(90_000),
                queue_owner_ttl_ms: Some(8_000),
                bindings_enabled: true,
                emit_runtime_events: true,
                allow_mcp_server_injection: true,
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let snapshot = collect_acp_runtime_snapshot(&config).expect("collect ACP snapshot");
        assert!(snapshot.control_plane.enabled);
        assert!(snapshot.control_plane.dispatch_enabled);
        assert_eq!(
            snapshot.control_plane.conversation_routing,
            AcpConversationRoutingMode::AgentPrefixedOnly
        );
        assert!(snapshot.control_plane.allowed_channels.is_empty());
        assert!(snapshot.control_plane.allowed_account_ids.is_empty());
        assert_eq!(
            snapshot.control_plane.thread_routing,
            AcpDispatchThreadRoutingMode::All
        );
        assert!(snapshot.control_plane.bootstrap_mcp_servers.is_empty());
        assert_eq!(snapshot.control_plane.max_concurrent_sessions, 6);
        assert_eq!(snapshot.control_plane.session_idle_ttl_ms, 45_000);
        assert_eq!(snapshot.control_plane.startup_timeout_ms, 10_000);
        assert_eq!(snapshot.control_plane.turn_timeout_ms, 90_000);
        assert_eq!(snapshot.control_plane.queue_owner_ttl_ms, 8_000);
        assert!(snapshot.control_plane.bindings_enabled);
        assert!(snapshot.control_plane.emit_runtime_events);
        assert!(snapshot.control_plane.allow_mcp_server_injection);
        assert_eq!(snapshot.control_plane.default_agent, "claude");
        assert_eq!(
            snapshot.control_plane.allowed_agents,
            vec!["codex".to_owned(), "claude".to_owned()]
        );
        assert_eq!(snapshot.selected.id, DEFAULT_ACP_BACKEND_ID);
        assert!(snapshot.mcp.servers.is_empty());
        assert!(snapshot.mcp.missing_selected_servers.is_empty());
    }

    #[test]
    fn derive_acp_conversation_route_wraps_non_agent_session_ids() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                default_agent: Some("claude".to_owned()),
                allowed_agents: vec!["claude".to_owned()],
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let route = derive_acp_conversation_route(&config, "telegram:42")
            .expect("derive ACP conversation route");
        assert_eq!(route.conversation_id, "telegram:42");
        assert_eq!(route.session_key, "agent:claude:telegram:42");
        assert_eq!(route.agent_id, "claude");
        assert_eq!(
            route.metadata.get("channel").map(String::as_str),
            Some("telegram")
        );
        assert_eq!(
            route.metadata.get("channel_identity").map(String::as_str),
            Some("42")
        );
        assert_eq!(
            route.metadata.get("acp_agent").map(String::as_str),
            Some("claude")
        );
    }

    #[test]
    fn derive_acp_conversation_route_preserves_agent_session_keys() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                default_agent: Some("claude".to_owned()),
                allowed_agents: vec!["claude".to_owned()],
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let route = derive_acp_conversation_route(&config, "agent:claude:review-thread")
            .expect("derive ACP route for agent-prefixed session");
        assert_eq!(route.conversation_id, "agent:claude:review-thread");
        assert_eq!(route.session_key, "agent:claude:review-thread");
        assert_eq!(route.agent_id, "claude");
    }

    #[test]
    fn derive_acp_conversation_route_rejects_disallowed_agent_prefix() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                allowed_agents: vec!["codex".to_owned()],
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let error = derive_acp_conversation_route(&config, "agent:claude:review-thread")
            .expect_err("disallowed ACP agent prefix must be rejected");
        assert!(error.contains("not in the allowed ACP agents"));
    }

    #[test]
    fn collect_acp_runtime_snapshot_reports_dispatch_policy() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    enabled: false,
                    conversation_routing: AcpConversationRoutingMode::AgentPrefixedOnly,
                    bootstrap_mcp_servers: vec![
                        " filesystem ".to_owned(),
                        "search".to_owned(),
                        "filesystem".to_owned(),
                    ],
                    working_directory: Some(" /workspace/dispatch ".to_owned()),
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        let snapshot = collect_acp_runtime_snapshot(&config).expect("collect ACP snapshot");
        assert!(!snapshot.control_plane.dispatch_enabled);
        assert_eq!(
            snapshot.control_plane.conversation_routing,
            AcpConversationRoutingMode::AgentPrefixedOnly
        );
        assert!(snapshot.control_plane.allowed_channels.is_empty());
        assert!(snapshot.control_plane.allowed_account_ids.is_empty());
        assert_eq!(
            snapshot.control_plane.thread_routing,
            AcpDispatchThreadRoutingMode::All
        );
        assert_eq!(
            snapshot.control_plane.bootstrap_mcp_servers,
            vec!["filesystem".to_owned(), "search".to_owned()]
        );
        assert_eq!(
            snapshot.control_plane.working_directory.as_deref(),
            Some("/workspace/dispatch")
        );
        assert!(snapshot.mcp.servers.is_empty());
        assert_eq!(
            snapshot.mcp.missing_selected_servers,
            vec!["filesystem".to_owned(), "search".to_owned()]
        );
    }

    #[test]
    fn should_route_conversation_turn_via_acp_respects_dispatch_policy() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    enabled: true,
                    conversation_routing: AcpConversationRoutingMode::AgentPrefixedOnly,
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        assert!(
            !should_route_conversation_turn_via_acp(&config, "telegram:42")
                .expect("dispatch evaluation should succeed")
        );
        assert!(
            should_route_conversation_turn_via_acp(&config, "agent:codex:review-thread")
                .expect("dispatch evaluation should succeed")
        );

        let decision = evaluate_acp_conversation_dispatch_for_address(
            &config,
            &ConversationSessionAddress::from_session_id("agent:codex:review-thread"),
        )
        .expect("dispatch decision should succeed");
        assert_eq!(
            decision.automatic_routing_origin,
            Some(super::super::AcpRoutingOrigin::AutomaticAgentPrefixed)
        );
    }

    #[test]
    fn should_route_conversation_turn_via_acp_defaults_to_agent_prefixed_only() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        assert!(
            !should_route_conversation_turn_via_acp(&config, "telegram:42")
                .expect("non-prefixed session should stay on provider path by default")
        );
        assert!(
            should_route_conversation_turn_via_acp(&config, "agent:codex:review-thread")
                .expect("agent-prefixed session should still route through ACP")
        );
    }

    #[test]
    fn derive_automatic_acp_routing_origin_for_address_distinguishes_prefixed_and_plain_routes() {
        assert_eq!(
            derive_automatic_acp_routing_origin_for_address(
                &ConversationSessionAddress::from_session_id("agent:codex:review-thread")
            )
            .expect("prefixed origin"),
            super::super::AcpRoutingOrigin::AutomaticAgentPrefixed
        );
        assert_eq!(
            derive_automatic_acp_routing_origin_for_address(
                &ConversationSessionAddress::from_session_id("telegram:42")
            )
            .expect("plain origin"),
            super::super::AcpRoutingOrigin::AutomaticDispatch
        );
    }

    #[test]
    fn prepare_acp_conversation_turn_records_explicit_routing_origin_and_provenance() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    enabled: true,
                    conversation_routing: AcpConversationRoutingMode::All,
                    bootstrap_mcp_servers: vec!["filesystem".to_owned()],
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let address = ConversationSessionAddress::from_session_id("opaque-session")
            .with_channel_scope("telegram", "42")
            .with_account_id("ops-bot");
        let options = super::super::AcpConversationTurnOptions {
            routing_intent: super::super::AcpRoutingIntent::Explicit,
            provenance: super::super::AcpTurnProvenance {
                trace_id: Some("trace-1"),
                source_message_id: Some("msg-1"),
                ack_cursor: Some("ack-1"),
            },
            ..super::super::AcpConversationTurnOptions::default()
        };

        let prepared =
            prepare_acp_conversation_turn_for_address(&config, &address, "hello", &options)
                .expect("prepare ACP conversation turn");

        assert_eq!(
            prepared.routing_origin,
            super::super::AcpRoutingOrigin::ExplicitRequest
        );
        assert_eq!(
            prepared
                .bootstrap
                .metadata
                .get(ACP_SESSION_METADATA_ACTIVATION_ORIGIN)
                .map(String::as_str),
            Some("explicit_request")
        );
        assert_eq!(
            prepared
                .request
                .metadata
                .get(ACP_TURN_METADATA_ROUTING_INTENT)
                .map(String::as_str),
            Some("explicit")
        );
        assert_eq!(
            prepared
                .request
                .metadata
                .get(ACP_TURN_METADATA_ROUTING_ORIGIN)
                .map(String::as_str),
            Some("explicit_request")
        );
        assert_eq!(
            prepared
                .request
                .metadata
                .get("loongclaw.trace_id")
                .map(String::as_str),
            Some("trace-1")
        );
        assert_eq!(
            prepared.bootstrap.mcp_servers,
            vec!["filesystem".to_owned()]
        );
    }

    #[test]
    fn prepare_acp_conversation_turn_uses_automatic_origin_for_agent_prefixed_routes() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    enabled: true,
                    conversation_routing: AcpConversationRoutingMode::AgentPrefixedOnly,
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let address = ConversationSessionAddress::from_session_id("agent:codex:review-thread");

        let prepared = prepare_acp_conversation_turn_for_address(
            &config,
            &address,
            "hello",
            &super::super::AcpConversationTurnOptions::default(),
        )
        .expect("prepare ACP conversation turn");

        assert_eq!(
            prepared.routing_origin,
            super::super::AcpRoutingOrigin::AutomaticAgentPrefixed
        );
        assert_eq!(
            prepared
                .request
                .metadata
                .get(ACP_TURN_METADATA_ROUTING_INTENT)
                .map(String::as_str),
            Some("automatic")
        );
        assert_eq!(
            prepared
                .request
                .metadata
                .get(ACP_TURN_METADATA_ROUTING_ORIGIN)
                .map(String::as_str),
            Some("automatic_agent_prefixed")
        );
    }

    #[test]
    fn describe_acp_conversation_dispatch_target_parses_agent_prefixed_channel_scope() {
        let target = describe_acp_conversation_dispatch_target(
            "agent:claude:feishu:lark_cli_a1b2c3:oc_123:om_thread_1",
        )
        .expect("describe ACP dispatch target");

        assert_eq!(
            target.original_session_id,
            "agent:claude:feishu:lark_cli_a1b2c3:oc_123:om_thread_1"
        );
        assert_eq!(
            target.route_session_id,
            "feishu:lark_cli_a1b2c3:oc_123:om_thread_1"
        );
        assert_eq!(target.prefixed_agent_id.as_deref(), Some("claude"));
        assert_eq!(target.channel_id.as_deref(), Some("feishu"));
        assert_eq!(
            target.channel_path,
            vec![
                "lark_cli_a1b2c3".to_owned(),
                "oc_123".to_owned(),
                "om_thread_1".to_owned()
            ]
        );
    }

    #[test]
    fn describe_acp_conversation_dispatch_target_prefers_structured_channel_address() {
        let address = ConversationSessionAddress::from_session_id("agent:claude:opaque-session")
            .with_channel_scope("feishu", "oc_123")
            .with_account_id("lark_cli_a1b2c3")
            .with_thread_id("om_thread_1");

        let target = describe_acp_conversation_dispatch_target_for_address(&address)
            .expect("describe ACP dispatch target");

        assert_eq!(target.original_session_id, "agent:claude:opaque-session");
        assert_eq!(
            target.route_session_id,
            "feishu:lark_cli_a1b2c3:oc_123:om_thread_1"
        );
        assert_eq!(target.prefixed_agent_id.as_deref(), Some("claude"));
        assert_eq!(target.channel_id.as_deref(), Some("feishu"));
        assert_eq!(target.account_id.as_deref(), Some("lark_cli_a1b2c3"));
        assert_eq!(target.conversation_id.as_deref(), Some("oc_123"));
        assert_eq!(target.thread_id.as_deref(), Some("om_thread_1"));
        assert_eq!(
            target.channel_path,
            vec![
                "lark_cli_a1b2c3".to_owned(),
                "oc_123".to_owned(),
                "om_thread_1".to_owned()
            ]
        );
    }

    #[test]
    fn derive_acp_conversation_route_exposes_explicit_binding_scope() {
        let config = LoongClawConfig::default();
        let address = ConversationSessionAddress::from_session_id("opaque-session")
            .with_channel_scope("feishu", "oc_123")
            .with_account_id("lark-prod")
            .with_thread_id("om_thread_1");

        let route = derive_acp_conversation_route_for_address(&config, &address)
            .expect("route should derive");

        assert_eq!(
            route
                .binding
                .as_ref()
                .map(|binding| binding.route_session_id.as_str()),
            Some("feishu:lark-prod:oc_123:om_thread_1")
        );
        assert_eq!(
            route
                .binding
                .as_ref()
                .and_then(|binding| binding.account_id.as_deref()),
            Some("lark-prod")
        );
        assert_eq!(
            route
                .binding
                .as_ref()
                .and_then(|binding| binding.thread_id.as_deref()),
            Some("om_thread_1")
        );
    }

    #[test]
    fn should_route_conversation_turn_via_acp_respects_channel_allowlist() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    conversation_routing: AcpConversationRoutingMode::All,
                    allowed_channels: vec!["telegram".to_owned()],
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };

        assert!(
            should_route_conversation_turn_via_acp(&config, "telegram:42")
                .expect("telegram channel should be allowed")
        );
        assert!(
            !should_route_conversation_turn_via_acp(&config, "feishu:oc_123")
                .expect("feishu channel should be blocked")
        );
        assert!(
            !should_route_conversation_turn_via_acp(&config, "default")
                .expect("non-channel sessions should be blocked by channel allowlist")
        );
    }

    #[test]
    fn should_route_conversation_turn_via_acp_uses_structured_channel_hint_for_opaque_session_id() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    conversation_routing: AcpConversationRoutingMode::All,
                    allowed_channels: vec!["telegram".to_owned()],
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let address = ConversationSessionAddress::from_session_id("opaque-session")
            .with_channel_scope("telegram", "chat_42");

        assert!(
            should_route_conversation_turn_via_acp_for_address(&config, &address)
                .expect("structured telegram hint should be allowed")
        );
    }

    #[test]
    fn evaluate_acp_conversation_dispatch_respects_account_allowlist() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    allowed_account_ids: vec!["ops-bot".to_owned()],
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let address = ConversationSessionAddress::from_session_id("opaque-session")
            .with_channel_scope("telegram", "100")
            .with_account_id("work-bot");

        let decision = evaluate_acp_conversation_dispatch_for_address(&config, &address)
            .expect("dispatch evaluation should succeed");

        assert!(!decision.route_via_acp);
        assert_eq!(
            decision.reason,
            AcpConversationDispatchReason::AccountNotAllowed
        );
    }

    #[test]
    fn evaluate_acp_conversation_dispatch_respects_thread_routing_policy() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    thread_routing: crate::config::AcpDispatchThreadRoutingMode::ThreadOnly,
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let root_address = ConversationSessionAddress::from_session_id("telegram:100")
            .with_channel_scope("telegram", "100");

        let decision = evaluate_acp_conversation_dispatch_for_address(&config, &root_address)
            .expect("dispatch evaluation should succeed");

        assert!(!decision.route_via_acp);
        assert_eq!(
            decision.reason,
            AcpConversationDispatchReason::ThreadRequired
        );
    }

    #[test]
    fn evaluate_acp_conversation_turn_entry_rejects_explicit_request_when_acp_disabled() {
        let config = LoongClawConfig::default();
        let address = ConversationSessionAddress::from_session_id("telegram:42");
        let options = super::super::AcpConversationTurnOptions::explicit();

        assert_eq!(
            evaluate_acp_conversation_turn_entry_for_address(&config, &address, &options)
                .expect("evaluate ACP turn entry"),
            AcpConversationTurnEntryDecision::RejectExplicitWhenDisabled
        );
    }

    #[test]
    fn evaluate_acp_conversation_turn_entry_routes_explicit_request_even_when_dispatch_disabled() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    enabled: false,
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let address = ConversationSessionAddress::from_session_id("telegram:42");
        let options = super::super::AcpConversationTurnOptions::explicit();

        assert_eq!(
            evaluate_acp_conversation_turn_entry_for_address(&config, &address, &options)
                .expect("evaluate ACP turn entry"),
            AcpConversationTurnEntryDecision::RouteViaAcp
        );
    }

    #[test]
    fn evaluate_acp_conversation_turn_entry_keeps_automatic_turns_on_provider_when_dispatch_blocks()
    {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    conversation_routing: AcpConversationRoutingMode::AgentPrefixedOnly,
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let address = ConversationSessionAddress::from_session_id("telegram:42");
        let options = super::super::AcpConversationTurnOptions::automatic();

        assert_eq!(
            evaluate_acp_conversation_turn_entry_for_address(&config, &address, &options)
                .expect("evaluate ACP turn entry"),
            AcpConversationTurnEntryDecision::StayOnProvider
        );
    }

    #[test]
    fn evaluate_acp_conversation_turn_entry_routes_automatic_turns_when_dispatch_allows() {
        let config = LoongClawConfig {
            acp: AcpConfig {
                enabled: true,
                dispatch: crate::config::AcpDispatchConfig {
                    conversation_routing: AcpConversationRoutingMode::All,
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongClawConfig::default()
        };
        let address = ConversationSessionAddress::from_session_id("telegram:42");
        let options = super::super::AcpConversationTurnOptions::automatic();

        let decision =
            evaluate_acp_conversation_turn_entry_for_address(&config, &address, &options)
                .expect("evaluate ACP turn entry");
        assert!(decision.routes_via_acp());
        assert_eq!(decision, AcpConversationTurnEntryDecision::RouteViaAcp);
    }

    #[test]
    fn shared_acp_session_manager_reuses_manager_for_same_memory_path() {
        let mut config = LoongClawConfig::default();
        config.memory.sqlite_path = std::env::temp_dir()
            .join("loongclaw-acp-runtime-shared.sqlite3")
            .display()
            .to_string();

        let first = shared_acp_session_manager(&config).expect("first shared ACP manager");
        let second = shared_acp_session_manager(&config).expect("second shared ACP manager");

        assert!(
            Arc::ptr_eq(&first, &second),
            "shared ACP session manager should be reused for the same memory path"
        );
    }
}
