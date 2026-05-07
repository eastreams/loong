use std::{collections::BTreeMap, path::Path};

use crate::{
    CliResult, KernelContext,
    acp::AcpConversationTurnOptions,
    config::LoongConfig,
    conversation::{
        ConversationIngressChannel, ConversationIngressContext, ConversationIngressDelivery,
        ConversationIngressDeliveryResource, ConversationIngressFeishuCallbackContext,
        ConversationIngressPrivateContext, ConversationRuntime, ConversationRuntimeBinding,
        ConversationSessionAddress, ConversationTurnCoordinator, ProviderErrorMode,
    },
    session::repository::{NewSessionRecord, SessionKind, SessionRepository, SessionState},
    session::store::SessionStoreConfig,
};

use super::{
    ChannelDeliveryFeishuCallback, ChannelDeliveryResource, ChannelInboundMessage, ChannelPlatform,
    ChannelSession,
    runtime::turn_feedback::{ChannelTurnFeedbackCapture, ChannelTurnFeedbackPolicy},
    types::ChannelResolvedAcpTurnHints,
};

#[derive(Debug, Clone)]
struct PreparedChannelInboundTurn {
    address: ConversationSessionAddress,
    acp_turn_hints: ChannelResolvedAcpTurnHints,
    ingress: Option<ConversationIngressContext>,
    feedback_capture: ChannelTurnFeedbackCapture,
}

fn prepare_channel_inbound_turn(
    config: &LoongConfig,
    message: &ChannelInboundMessage,
    feedback_policy: ChannelTurnFeedbackPolicy,
) -> CliResult<PreparedChannelInboundTurn> {
    let address = resolve_channel_conversation_address(config, &message.session)?;
    let acp_turn_hints = resolve_channel_acp_turn_hints(config, &message.session)?;
    let ingress = channel_message_ingress_context(message);
    let feedback_capture = ChannelTurnFeedbackCapture::new(feedback_policy);

    Ok(PreparedChannelInboundTurn {
        address,
        acp_turn_hints,
        ingress,
        feedback_capture,
    })
}

fn resolve_channel_conversation_address(
    config: &LoongConfig,
    session: &ChannelSession,
) -> CliResult<ConversationSessionAddress> {
    let route_address = session.conversation_address();
    let route_session_id = route_address.session_id.trim();
    if route_session_id.is_empty() {
        return Err("channel conversation route requires a non-empty session id".to_owned());
    }

    let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;
    let active_session_id = match repo.load_session_route_binding(route_session_id)? {
        Some(binding) => binding.active_session_id,
        None => {
            let active_session_id = route_session_id.to_owned();
            let _ = repo.ensure_session(NewSessionRecord {
                session_id: active_session_id.clone(),
                kind: SessionKind::Root,
                parent_session_id: None,
                label: Some(route_session_id.to_owned()),
                state: SessionState::Ready,
            })?;
            let _ =
                repo.upsert_session_route_binding(route_session_id, active_session_id.as_str())?;
            active_session_id
        }
    };

    Ok(build_effective_channel_conversation_address(
        active_session_id.as_str(),
        session,
    ))
}

fn build_effective_channel_conversation_address(
    session_id: &str,
    session: &ChannelSession,
) -> ConversationSessionAddress {
    let mut effective = ConversationSessionAddress::from_session_id(session_id)
        .with_channel_scope(session.platform.as_str(), session.conversation_id.clone());
    if let Some(account_id) = session.account_id.as_deref() {
        effective = effective.with_account_id(account_id);
    }
    if session.identity_participant_scoped
        && let Some(participant_id) = session.participant_id.as_deref()
    {
        effective = effective.with_participant_id(participant_id);
    }
    if session.identity_thread_scoped
        && let Some(thread_id) = session.thread_id.as_deref()
    {
        effective = effective.with_thread_id(thread_id);
    }
    effective
}

fn unix_time_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) async fn process_inbound_with_runtime_and_feedback<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    message: &ChannelInboundMessage,
    binding: ConversationRuntimeBinding<'_>,
    feedback_policy: ChannelTurnFeedbackPolicy,
) -> CliResult<String> {
    process_inbound_with_runtime_and_feedback_and_error_mode(
        config,
        runtime,
        message,
        binding,
        feedback_policy,
        ProviderErrorMode::Propagate,
        None,
    )
    .await
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
#[cfg_attr(not(test), allow(dead_code))]
pub async fn process_inbound_with_runtime_and_feedback_and_error_mode<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongConfig,
    runtime: &R,
    message: &ChannelInboundMessage,
    binding: ConversationRuntimeBinding<'_>,
    feedback_policy: ChannelTurnFeedbackPolicy,
    error_mode: ProviderErrorMode,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> CliResult<String> {
    let prepared = prepare_channel_inbound_turn(config, message, feedback_policy)?;
    let provenance = channel_message_turn_provenance(message);
    let acp_options = AcpConversationTurnOptions::automatic()
        .with_additional_bootstrap_mcp_servers(&prepared.acp_turn_hints.bootstrap_mcp_servers)
        .with_working_directory(prepared.acp_turn_hints.working_directory.as_deref())
        .with_provenance(provenance.as_acp_turn_provenance());
    let observer = prepared.feedback_capture.observer_handle();
    let reply = ConversationTurnCoordinator::new()
        .handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            &prepared.address,
            &message.text,
            error_mode,
            runtime,
            &acp_options,
            binding,
            prepared.ingress.as_ref(),
            observer,
            retry_progress,
            None,
        )
        .await?;
    Ok(prepared.feedback_capture.render_reply(reply))
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub async fn process_inbound_with_provider(
    config: &LoongConfig,
    resolved_path: Option<&Path>,
    message: &ChannelInboundMessage,
    kernel_ctx: &KernelContext,
    feedback_policy: ChannelTurnFeedbackPolicy,
) -> CliResult<String> {
    process_inbound_with_provider_and_error_mode_and_retry_progress(
        config,
        resolved_path,
        message,
        kernel_ctx,
        feedback_policy,
        ProviderErrorMode::Propagate,
        None,
    )
    .await
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub async fn process_inbound_with_provider_and_error_mode_and_retry_progress(
    config: &LoongConfig,
    resolved_path: Option<&Path>,
    message: &ChannelInboundMessage,
    kernel_ctx: &KernelContext,
    feedback_policy: ChannelTurnFeedbackPolicy,
    error_mode: ProviderErrorMode,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> CliResult<String> {
    process_inbound_with_provider_and_error_mode(
        config,
        resolved_path,
        message,
        kernel_ctx,
        feedback_policy,
        error_mode,
        retry_progress,
    )
    .await
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub async fn process_inbound_with_provider_and_error_mode(
    config: &LoongConfig,
    resolved_path: Option<&Path>,
    message: &ChannelInboundMessage,
    kernel_ctx: &KernelContext,
    feedback_policy: ChannelTurnFeedbackPolicy,
    error_mode: ProviderErrorMode,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> CliResult<String> {
    if let Some(reply) = maybe_reset_channel_session(config, message).await? {
        return Ok(reply);
    }

    let started_at = std::time::Instant::now();
    let result = match reload_channel_turn_config(config, resolved_path) {
        Ok(turn_config) => {
            let prepared = prepare_channel_inbound_turn(&turn_config, message, feedback_policy)?;
            let turn_request = crate::turn_gateway::TurnGatewayRequest {
                address: prepared.address,
                message: message.text.clone(),
                metadata: BTreeMap::new(),
                turn_mode: crate::agent_runtime::AgentTurnMode::Oneshot,
                acp: false,
                acp_event_stream: false,
                acp_bootstrap_mcp_servers: prepared.acp_turn_hints.bootstrap_mcp_servers.clone(),
                acp_cwd: prepared
                    .acp_turn_hints
                    .working_directory
                    .as_ref()
                    .map(|path: &std::path::PathBuf| path.display().to_string()),
                live_surface_enabled: false,
                ingress: prepared.ingress,
                observer: prepared.feedback_capture.observer_handle(),
                provenance: channel_message_turn_provenance(message),
                provider_error_mode: error_mode,
                retry_progress,
            };
            let execution = crate::turn_gateway::TurnGatewayExecution {
                resolved_path: resolved_path.map(Path::to_path_buf).unwrap_or_default(),
                config: turn_config,
                kernel_ctx: Some(kernel_ctx.clone()),
                acp_manager: None,
                event_sink: None,
                initialize_runtime_environment: true,
            };
            let result = crate::turn_gateway::run_turn_gateway(execution, turn_request).await?;
            Ok(prepared.feedback_capture.render_reply(result.output_text))
        }
        Err(error) => Err(error),
    };
    let duration_ms = started_at.elapsed().as_millis();
    match &result {
        Ok(reply) => {
            let has_conversation_id = !message.session.conversation_id.trim().is_empty();
            let has_configured_account_id = message
                .session
                .configured_account_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_account_id = message
                .session
                .account_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_source_message_id = message
                .delivery
                .source_message_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_ack_cursor = message
                .delivery
                .ack_cursor
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            tracing::debug!(
                target: "loong.channel",
                platform = %message.session.platform.as_str(),
                has_conversation_id,
                has_configured_account_id,
                has_account_id,
                has_source_message_id,
                has_ack_cursor,
                text_len = message.text.chars().count(),
                reply_len = reply.chars().count(),
                duration_ms,
                "channel inbound processed"
            );
        }
        Err(error) => {
            let has_conversation_id = !message.session.conversation_id.trim().is_empty();
            let has_configured_account_id = message
                .session
                .configured_account_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_account_id = message
                .session
                .account_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_source_message_id = message
                .delivery
                .source_message_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_ack_cursor = message
                .delivery
                .ack_cursor
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            tracing::warn!(
                target: "loong.channel",
                platform = %message.session.platform.as_str(),
                has_conversation_id,
                has_configured_account_id,
                has_account_id,
                has_source_message_id,
                has_ack_cursor,
                text_len = message.text.chars().count(),
                duration_ms,
                error = %crate::observability::summarize_error(error),
                "channel inbound failed"
            );
        }
    }
    result
}

async fn maybe_reset_channel_session(
    config: &LoongConfig,
    message: &ChannelInboundMessage,
) -> CliResult<Option<String>> {
    if !is_channel_session_reset_command(message.text.as_str()) {
        return Ok(None);
    }

    let route_address = message.session.conversation_address();
    let route_session_id = route_address.session_id.trim();
    if route_session_id.is_empty() {
        return Err("channel session reset requires a stable route session id".to_owned());
    }

    let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;
    let prior_binding = repo.load_session_route_binding(route_session_id)?;

    if config.acp.enabled
        && config.acp.dispatch_enabled()
        && let Some(binding) = prior_binding.as_ref()
    {
        let manager = crate::acp::shared_acp_session_manager(config)?;
        let active_address = build_effective_channel_conversation_address(
            binding.active_session_id.as_str(),
            &message.session,
        );
        if let Ok(route) =
            crate::acp::derive_acp_conversation_route_for_address(config, &active_address)
        {
            let _ = manager.close(config, route.session_key.as_str()).await;
        }
    }

    let next_session_id = fresh_local_session_id(route_session_id);
    let _ = repo.ensure_session(NewSessionRecord {
        session_id: next_session_id.clone(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some(route_session_id.to_owned()),
        state: SessionState::Ready,
    })?;
    let _ = repo.upsert_session_route_binding(route_session_id, next_session_id.as_str())?;

    Ok(Some(format!(
        "Started a new session for this conversation. Session ID: {}",
        next_session_id
    )))
}

fn is_channel_session_reset_command(input: &str) -> bool {
    let trimmed = input.trim();
    let command = trimmed.split_whitespace().next().unwrap_or_default().trim();
    if command != trimmed {
        return false;
    }
    let normalized = command
        .split_once('@')
        .map(|(prefix, _)| prefix)
        .unwrap_or(command);
    matches!(normalized, "/new" | ":new" | "/reset" | ":reset")
}

fn fresh_local_session_id(route_session_id: &str) -> String {
    let mut sanitized = String::with_capacity(route_session_id.len());
    for ch in route_session_id.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    format!("{sanitized}__new__{}", unix_time_ms_now())
}

pub(super) fn reload_channel_turn_config(
    config: &LoongConfig,
    resolved_path: Option<&Path>,
) -> CliResult<LoongConfig> {
    match resolved_path {
        Some(path) => config.reload_provider_runtime_state_from_path(path),
        None => Ok(config.clone()),
    }
}

fn resolve_channel_acp_turn_hints(
    config: &LoongConfig,
    session: &ChannelSession,
) -> CliResult<ChannelResolvedAcpTurnHints> {
    match session.platform {
        ChannelPlatform::Telegram => {
            let resolved = config
                .telegram
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Feishu => {
            let resolved = config
                .feishu
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Line => Ok(ChannelResolvedAcpTurnHints::default()),
        ChannelPlatform::Matrix => {
            let resolved = config
                .matrix
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Wecom => {
            let resolved = config
                .wecom
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Webhook => Ok(ChannelResolvedAcpTurnHints::default()),
        ChannelPlatform::Weixin => Ok(ChannelResolvedAcpTurnHints::default()),
        ChannelPlatform::Qqbot => Ok(ChannelResolvedAcpTurnHints::default()),
        ChannelPlatform::Onebot => Ok(ChannelResolvedAcpTurnHints::default()),
        ChannelPlatform::WhatsApp => Ok(ChannelResolvedAcpTurnHints::default()),
        ChannelPlatform::Irc => Ok(ChannelResolvedAcpTurnHints::default()),
    }
}

fn channel_message_turn_provenance(
    message: &ChannelInboundMessage,
) -> crate::turn_gateway::TurnGatewayProvenance {
    crate::turn_gateway::TurnGatewayProvenance {
        trace_id: None,
        source_message_id: message.delivery.source_message_id.clone(),
        ack_cursor: message.delivery.ack_cursor.clone(),
    }
}

pub(super) fn channel_message_ingress_context(
    message: &ChannelInboundMessage,
) -> Option<ConversationIngressContext> {
    let participant_id = trimmed_non_empty(message.session.participant_id.as_deref());
    let thread_id = trimmed_non_empty(message.session.thread_id.as_deref());
    let resources = message
        .delivery
        .resources
        .iter()
        .filter_map(normalized_channel_delivery_resource)
        .collect::<Vec<_>>();
    let delivery = ConversationIngressDelivery {
        source_message_id: trimmed_non_empty(message.delivery.source_message_id.as_deref()),
        sender_identity_key: trimmed_non_empty(message.delivery.sender_principal_key.as_deref()),
        thread_root_id: trimmed_non_empty(message.delivery.thread_root_id.as_deref()),
        parent_message_id: trimmed_non_empty(message.delivery.parent_message_id.as_deref()),
        resources,
    };
    let has_contextual_hints = participant_id.is_some()
        || thread_id.is_some()
        || delivery != ConversationIngressDelivery::default();
    if !has_contextual_hints {
        return None;
    }

    let conversation_id = message.session.conversation_id.trim();
    if conversation_id.is_empty() {
        return None;
    }

    Some(ConversationIngressContext {
        channel: ConversationIngressChannel {
            platform: message.session.platform.as_str().to_owned(),
            configured_account_id: trimmed_non_empty(
                message.session.configured_account_id.as_deref(),
            ),
            account_id: trimmed_non_empty(message.session.account_id.as_deref()),
            conversation_id: conversation_id.to_owned(),
            participant_id,
            thread_id,
        },
        delivery,
        private: ConversationIngressPrivateContext {
            feishu_callback: normalized_feishu_callback_context(
                message.delivery.feishu_callback.as_ref(),
            ),
        },
    })
}

fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalized_channel_delivery_resource(
    resource: &ChannelDeliveryResource,
) -> Option<ConversationIngressDeliveryResource> {
    let resource_type = resource.resource_type.trim();
    let file_key = resource.file_key.trim();
    if resource_type.is_empty() || file_key.is_empty() {
        return None;
    }

    Some(ConversationIngressDeliveryResource {
        resource_type: resource_type.to_owned(),
        file_key: file_key.to_owned(),
        file_name: trimmed_non_empty(resource.file_name.as_deref()),
    })
}

fn normalized_feishu_callback_context(
    callback: Option<&ChannelDeliveryFeishuCallback>,
) -> Option<ConversationIngressFeishuCallbackContext> {
    let callback = callback?;
    let normalized = ConversationIngressFeishuCallbackContext {
        callback_token: trimmed_non_empty(callback.callback_token.as_deref()),
        open_message_id: trimmed_non_empty(callback.open_message_id.as_deref()),
        open_chat_id: trimmed_non_empty(callback.open_chat_id.as_deref()),
        operator_open_id: trimmed_non_empty(callback.operator_open_id.as_deref()),
        deferred_context_id: trimmed_non_empty(callback.deferred_context_id.as_deref()),
    };
    if normalized.callback_token.is_none()
        && normalized.open_message_id.is_none()
        && normalized.open_chat_id.is_none()
        && normalized.operator_open_id.is_none()
        && normalized.deferred_context_id.is_none()
    {
        return None;
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::{maybe_reset_channel_session, resolve_channel_conversation_address};
    use crate::channel::{ChannelPlatform, ChannelSession};
    use crate::config::LoongConfig;
    use crate::session::repository::SessionRepository;
    use crate::session::store::SessionStoreConfig;

    fn isolated_config(test_name: &str) -> LoongConfig {
        let sqlite_path = std::env::temp_dir().join(format!(
            "loong-im-route-binding-{test_name}-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&sqlite_path);
        let mut config = LoongConfig::default();
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config
    }

    #[test]
    fn resolve_channel_conversation_address_reuses_existing_route_binding() {
        let config = isolated_config("reuse");
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "lark_cli_a1b2c3", "oc_123")
                .with_configured_account_id("work")
                .with_participant_id("ou_sender_1")
                .with_thread_id("om_thread_1");

        let first = resolve_channel_conversation_address(&config, &session)
            .expect("resolve first route address");
        let second = resolve_channel_conversation_address(&config, &session)
            .expect("resolve second route address");

        assert_eq!(first.session_id, second.session_id);
        assert_eq!(first.session_id, "feishu:cfg=work:lark_cli_a1b2c3:oc_123");
        assert_eq!(first.channel_id.as_deref(), Some("feishu"));
        assert_eq!(first.account_id.as_deref(), Some("lark_cli_a1b2c3"));
        assert_eq!(first.conversation_id.as_deref(), Some("oc_123"));
        assert!(first.participant_id.is_none());
        assert!(first.thread_id.is_none());

        let repo = SessionRepository::new(&SessionStoreConfig::from_memory_config(&config.memory))
            .expect("session repository");
        let binding = repo
            .load_session_route_binding("feishu:cfg=work:lark_cli_a1b2c3:oc_123")
            .expect("load route binding")
            .expect("route binding exists");
        assert_eq!(binding.active_session_id, first.session_id);
    }

    #[test]
    fn resolve_channel_conversation_address_preserves_opt_in_thread_scope() {
        let config = isolated_config("thread-scope");
        let session = ChannelSession::with_account(ChannelPlatform::Telegram, "bot_123456", "42")
            .with_thread_id("7")
            .with_identity_thread_scoped(true);

        let address = resolve_channel_conversation_address(&config, &session)
            .expect("resolve thread-scoped route address");

        assert_eq!(address.session_id, "telegram:bot_123456:42:7");
        assert_eq!(address.channel_id.as_deref(), Some("telegram"));
        assert_eq!(address.account_id.as_deref(), Some("bot_123456"));
        assert_eq!(address.conversation_id.as_deref(), Some("42"));
        assert_eq!(address.thread_id.as_deref(), Some("7"));
        assert!(address.participant_id.is_none());

        let repo = SessionRepository::new(&SessionStoreConfig::from_memory_config(&config.memory))
            .expect("session repository");
        let binding = repo
            .load_session_route_binding("telegram:bot_123456:42:7")
            .expect("load route binding")
            .expect("route binding exists");
        assert_eq!(binding.active_session_id, address.session_id);
    }

    #[tokio::test]
    async fn reset_command_rotates_route_binding_to_new_local_session() {
        let config = isolated_config("reset");
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "lark_cli_a1b2c3", "oc_123")
                .with_configured_account_id("work");
        let message = crate::channel::ChannelInboundMessage {
            session: session.clone(),
            reply_target: crate::channel::ChannelOutboundTarget::feishu_receive_id("oc_123"),
            text: "/new".to_owned(),
            delivery: crate::channel::ChannelDelivery::default(),
        };

        let before = resolve_channel_conversation_address(&config, &session)
            .expect("resolve initial route address");
        let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
        crate::session::store::append_session_turn_direct(
            before.session_id.as_str(),
            "user",
            "before reset",
            &memory_config,
        )
        .expect("seed prior session history");
        let reply = maybe_reset_channel_session(&config, &message)
            .await
            .expect("reset channel session")
            .expect("reset reply");
        let after = resolve_channel_conversation_address(&config, &session)
            .expect("resolve route address after reset");

        assert!(reply.contains("Started a new session"));
        assert_ne!(before.session_id, after.session_id);
        assert!(!after.session_id.contains(':'));

        let repo = SessionRepository::new(&memory_config).expect("session repository");
        let binding = repo
            .load_session_route_binding("feishu:cfg=work:lark_cli_a1b2c3:oc_123")
            .expect("load route binding")
            .expect("route binding exists");
        assert_eq!(binding.active_session_id, after.session_id);
        assert!(
            repo.load_session(before.session_id.as_str())
                .expect("load prior session")
                .is_some()
        );

        let resumed = crate::chat::initialize_cli_turn_runtime_with_loaded_config(
            std::path::PathBuf::from("/tmp/loong.toml"),
            config.clone(),
            Some(after.session_id.as_str()),
            &crate::chat::CliChatOptions::default(),
            "cli-chat-im-reset-resume-test",
            crate::chat::CliSessionRequirement::AllowImplicitDefault,
            false,
        )
        .expect("reopen rotated IM local session");
        assert_eq!(resumed.session_id, after.session_id);

        let prior_resumed = crate::chat::initialize_cli_turn_runtime_with_loaded_config(
            std::path::PathBuf::from("/tmp/loong.toml"),
            config.clone(),
            Some(before.session_id.as_str()),
            &crate::chat::CliChatOptions::default(),
            "cli-chat-im-prior-session-test",
            crate::chat::CliSessionRequirement::AllowImplicitDefault,
            false,
        )
        .expect("reopen prior IM local session");
        assert_eq!(prior_resumed.session_id, before.session_id);
    }

    #[test]
    fn reset_command_parser_accepts_exact_commands_and_optional_mentions_only() {
        assert!(super::is_channel_session_reset_command("/new"));
        assert!(super::is_channel_session_reset_command("/new@LoongBot"));
        assert!(super::is_channel_session_reset_command("/reset"));
        assert!(!super::is_channel_session_reset_command("/new please"));
        assert!(!super::is_channel_session_reset_command("please /new"));
    }
}
