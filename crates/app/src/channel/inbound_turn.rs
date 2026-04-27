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
    let address = message.session.conversation_address();
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
