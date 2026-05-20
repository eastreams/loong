use crate::{
    CliResult,
    channel::types::{
        ChannelSendReceipt, KnownChannelSessionSendTarget, parse_known_channel_session_send_target,
    },
    config::LoongConfig,
};

use super::super::{ChannelOutboundTargetKind, FeishuChannelSendRequest};

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
pub(crate) async fn send_text_to_known_session(
    config: &LoongConfig,
    session_id: &str,
    text: &str,
) -> CliResult<ChannelSendReceipt> {
    match parse_known_channel_session_send_target(config, session_id)? {
        KnownChannelSessionSendTarget::Telegram {
            account_id,
            chat_id,
            thread_id,
        } => {
            #[cfg(not(feature = "channel-telegram"))]
            {
                let _ = (config, account_id, chat_id, thread_id, text);
                Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned())
            }

            #[cfg(feature = "channel-telegram")]
            {
                let resolved = config
                    .telegram
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: telegram channel is disabled by config"
                            .to_owned(),
                    );
                }
                let allowed_chat_id = chat_id.parse::<i64>().map_err(|error| {
                    format!("sessions_send_invalid_telegram_target: `{chat_id}`: {error}")
                })?;
                if !resolved.allowed_chat_ids.contains(&allowed_chat_id) {
                    return Err(format!(
                        "sessions_send_target_not_allowed: telegram target `{allowed_chat_id}` is not present in telegram.allowed_chat_ids"
                    ));
                }
                let token = resolved.bot_token().ok_or_else(|| {
                    "telegram bot token missing (set telegram.bot_token or env)".to_owned()
                })?;
                let target = match thread_id {
                    Some(thread_id) => format!("{chat_id}:topic:{thread_id}"),
                    None => chat_id,
                };
                super::super::telegram::run_telegram_send(
                    &resolved,
                    token,
                    ChannelOutboundTargetKind::Conversation,
                    target.as_str(),
                    text,
                )
                .await?;
                Ok(ChannelSendReceipt {
                    channel: "telegram",
                    target,
                })
            }
        }
        KnownChannelSessionSendTarget::Feishu {
            account_id,
            conversation_id,
            reply_message_id,
        } => {
            #[cfg(not(feature = "channel-feishu"))]
            {
                let _ = (config, account_id, conversation_id, reply_message_id, text);
                Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned())
            }

            #[cfg(feature = "channel-feishu")]
            {
                let resolved = config
                    .feishu
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: feishu channel is disabled by config"
                            .to_owned(),
                    );
                }
                let target_allowed = crate::channel::feishu::feishu_allowlist_allows_chat(
                    resolved.allowed_chat_ids.iter(),
                    conversation_id.as_str(),
                );
                if !target_allowed {
                    return Err(format!(
                        "sessions_send_target_not_allowed: feishu target `{conversation_id}` is not present in feishu.allowed_chat_ids"
                    ));
                }
                let (target_kind, target) = match reply_message_id {
                    Some(message_id) => (ChannelOutboundTargetKind::MessageReply, message_id),
                    None => (ChannelOutboundTargetKind::ReceiveId, conversation_id),
                };
                let request = match target_kind {
                    ChannelOutboundTargetKind::MessageReply => FeishuChannelSendRequest {
                        receive_id: target.clone(),
                        text: Some(text.to_owned()),
                        ..FeishuChannelSendRequest::default()
                    },
                    ChannelOutboundTargetKind::ReceiveId
                    | ChannelOutboundTargetKind::Conversation
                    | ChannelOutboundTargetKind::Address => FeishuChannelSendRequest {
                        receive_id: target.clone(),
                        receive_id_type: Some("chat_id".to_owned()),
                        text: Some(text.to_owned()),
                        ..FeishuChannelSendRequest::default()
                    },
                    ChannelOutboundTargetKind::Endpoint => {
                        return Err(
                            "sessions_send_invalid_target_kind: feishu session sends do not support endpoint targets"
                                .to_owned(),
                        );
                    }
                };
                crate::channel::feishu::run_feishu_send(&resolved, &request).await?;
                Ok(ChannelSendReceipt {
                    channel: "feishu",
                    target,
                })
            }
        }
        KnownChannelSessionSendTarget::Line {
            account_id,
            address,
        } => {
            #[cfg(not(feature = "channel-line"))]
            {
                let _ = (config, account_id, address, text);
                Err("line channel is disabled (enable feature `channel-line`)".to_owned())
            }

            #[cfg(feature = "channel-line")]
            {
                let resolved = config
                    .line
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: line channel is disabled by config"
                            .to_owned(),
                    );
                }

                crate::channel::line::run_line_send(
                    &resolved,
                    ChannelOutboundTargetKind::Address,
                    address.as_str(),
                    text,
                    crate::channel::http::outbound_http_policy_from_config(config),
                )
                .await?;

                Ok(ChannelSendReceipt {
                    channel: "line",
                    target: address,
                })
            }
        }
        KnownChannelSessionSendTarget::Matrix {
            account_id,
            room_id,
        } => {
            #[cfg(not(feature = "channel-matrix"))]
            {
                let _ = (config, account_id, room_id, text);
                Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned())
            }

            #[cfg(feature = "channel-matrix")]
            {
                let resolved = config
                    .matrix
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: matrix channel is disabled by config"
                            .to_owned(),
                    );
                }
                if !resolved
                    .allowed_room_ids
                    .iter()
                    .any(|allowed| allowed.trim() == room_id)
                {
                    return Err(format!(
                        "sessions_send_target_not_allowed: matrix target `{room_id}` is not present in matrix.allowed_room_ids"
                    ));
                }
                let token = resolved.access_token().ok_or_else(|| {
                    "matrix access token missing (set matrix.access_token or env)".to_owned()
                })?;
                crate::channel::matrix::run_matrix_send(
                    &resolved,
                    token,
                    ChannelOutboundTargetKind::Conversation,
                    room_id.as_str(),
                    text,
                )
                .await?;
                Ok(ChannelSendReceipt {
                    channel: "matrix",
                    target: room_id,
                })
            }
        }
        KnownChannelSessionSendTarget::Wecom {
            account_id,
            conversation_id,
            chat_type,
        } => {
            #[cfg(not(feature = "channel-wecom"))]
            {
                let _ = (config, account_id, conversation_id, chat_type, text);
                Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned())
            }

            #[cfg(feature = "channel-wecom")]
            {
                let resolved = config
                    .wecom
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: wecom channel is disabled by config"
                            .to_owned(),
                    );
                }
                let is_allowed = resolved
                    .allowed_conversation_ids
                    .iter()
                    .any(|allowed| allowed.trim() == conversation_id);
                if !is_allowed {
                    return Err(format!(
                        "sessions_send_target_not_allowed: wecom target `{conversation_id}` is not present in wecom.allowed_conversation_ids"
                    ));
                }
                crate::channel::wecom::send_wecom_text(
                    &resolved,
                    conversation_id.as_str(),
                    chat_type,
                    text,
                )
                .await?;
                Ok(ChannelSendReceipt {
                    channel: "wecom",
                    target: conversation_id,
                })
            }
        }
        KnownChannelSessionSendTarget::WhatsApp {
            account_id,
            address,
        } => {
            #[cfg(not(feature = "channel-whatsapp"))]
            {
                let _ = (config, account_id, address, text);
                Err("whatsapp channel is disabled (enable feature `channel-whatsapp`)".to_owned())
            }

            #[cfg(feature = "channel-whatsapp")]
            {
                let resolved = config
                    .whatsapp
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: whatsapp channel is disabled by config"
                            .to_owned(),
                    );
                }

                crate::channel::whatsapp::run_whatsapp_send(
                    &resolved,
                    ChannelOutboundTargetKind::Address,
                    address.as_str(),
                    text,
                    crate::channel::http::outbound_http_policy_from_config(config),
                )
                .await?;

                Ok(ChannelSendReceipt {
                    channel: "whatsapp",
                    target: address,
                })
            }
        }
    }
}

#[cfg(not(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
)))]
pub(crate) async fn send_text_to_known_session(
    _config: &crate::config::LoongConfig,
    session_id: &str,
    _text: &str,
) -> CliResult<ChannelSendReceipt> {
    Err(format!("sessions_send_channel_unsupported: `{session_id}`"))
}
