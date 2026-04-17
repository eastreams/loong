#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use std::collections::BTreeSet;

use async_trait::async_trait;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-signal",
    feature = "channel-slack",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use serde::Serialize;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use super::state::ChannelOperationRuntimeTracker;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use super::turn_feedback::ChannelTurnFeedbackPolicy;
use crate::CliResult;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use crate::config::LoongClawConfig;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use crate::config::normalize_channel_account_id;
use crate::conversation::parse_route_session_id;

// Re-export core types for convenience
pub use super::super::core::types::*;

// ============================================================================
// ChannelAdapter trait (runtime-coupled due to ChannelTurnFeedbackPolicy)
// ============================================================================

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
#[allow(dead_code)]
#[async_trait]
pub trait ChannelAdapter {
    fn name(&self) -> &str;
    fn streaming_mode(&self) -> ChannelStreamingMode {
        ChannelStreamingMode::Off
    }
    fn turn_feedback_policy(&self) -> ChannelTurnFeedbackPolicy {
        ChannelTurnFeedbackPolicy::final_trace_significant()
    }
    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>>;
    async fn send_message(
        &self,
        target: &ChannelOutboundTarget,
        message: &ChannelOutboundMessage,
    ) -> CliResult<()>;
    async fn send_message_streaming(
        &mut self,
        target: &ChannelOutboundTarget,
        message: &ChannelOutboundMessage,
        _streaming_mode: ChannelStreamingMode,
    ) -> CliResult<()> {
        self.send_message(target, message).await
    }
    async fn send_text(&self, target: &ChannelOutboundTarget, text: &str) -> CliResult<()> {
        let message = ChannelOutboundMessage::Text(text.to_owned());
        self.send_message(target, &message).await
    }
    async fn ack_inbound(&mut self, _message: &ChannelInboundMessage) -> CliResult<()> {
        Ok(())
    }
    async fn complete_batch(&mut self) -> CliResult<()> {
        Ok(())
    }
}

// ============================================================================
// KnownChannelSessionSendTarget and parse functions (runtime-coupled due to LoongClawConfig)
// ============================================================================

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::channel) enum KnownChannelSessionSendTarget {
    Telegram {
        account_id: Option<String>,
        chat_id: String,
        thread_id: Option<String>,
    },
    Feishu {
        account_id: Option<String>,
        conversation_id: String,
        reply_message_id: Option<String>,
    },
    Line {
        account_id: Option<String>,
        address: String,
    },
    Matrix {
        account_id: Option<String>,
        room_id: String,
    },
    Wecom {
        account_id: Option<String>,
        conversation_id: String,
        chat_type: Option<u8>,
    },
    WhatsApp {
        account_id: Option<String>,
        address: String,
    },
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedKnownChannelSessionTarget {
    pub route_session_id: String,
    pub channel_id: String,
    pub account_id: Option<String>,
    pub session_shape: &'static str,
    pub target_kind: ChannelOutboundTargetKind,
    pub target_id: String,
    pub raw_scope: Vec<String>,
    pub conversation_id: Option<String>,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
    pub reply_message_id: Option<String>,
    pub chat_type: Option<u8>,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(in crate::channel) fn parse_known_channel_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
) -> CliResult<KnownChannelSessionSendTarget> {
    let (channel, scope) = parse_route_session_id(session_id)?
        .ok_or_else(|| format!("sessions_send_channel_unsupported: `{session_id}`"))?;

    match channel.as_str() {
        "telegram" => parse_telegram_session_send_target(config, session_id, scope.as_slice()),
        "feishu" | "lark" => parse_feishu_session_send_target(config, session_id, scope.as_slice()),
        "line" => parse_line_session_send_target(config, session_id, scope.as_slice()),
        "matrix" => parse_matrix_session_send_target(config, session_id, scope.as_slice()),
        "wecom" | "wechat-work" | "qywx" => {
            parse_wecom_session_send_target(config, session_id, scope.as_slice())
        }
        "whatsapp" => parse_whatsapp_session_send_target(config, session_id, scope.as_slice()),
        _ => Err(format!("sessions_send_channel_unsupported: `{session_id}`")),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
pub fn resolve_known_channel_session_target(
    config: &LoongClawConfig,
    session_id: &str,
) -> CliResult<ResolvedKnownChannelSessionTarget> {
    let (channel, scope) = parse_route_session_id(session_id)?
        .ok_or_else(|| format!("sessions_send_channel_unsupported: `{session_id}`"))?;

    match channel.as_str() {
        "telegram" => resolve_telegram_known_session_target(config, session_id, scope.as_slice()),
        "feishu" | "lark" => {
            resolve_feishu_known_session_target(config, session_id, scope.as_slice())
        }
        "matrix" => resolve_matrix_known_session_target(config, session_id, scope.as_slice()),
        "wecom" | "wechat-work" | "qywx" => {
            resolve_wecom_known_session_target(config, session_id, scope.as_slice())
        }
        _ => Err(format!("sessions_send_channel_unsupported: `{session_id}`")),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn parse_telegram_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<KnownChannelSessionSendTarget> {
    let configured_account_ids = config.telegram.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .telegram
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let Some(chat_id) = scoped_path
        .first()
        .map(String::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err(format!("sessions_send_channel_unsupported: `{session_id}`"));
    };

    let thread_id = scoped_path
        .last()
        .filter(|_| scoped_path.len() >= 2)
        .cloned();
    Ok(KnownChannelSessionSendTarget::Telegram {
        account_id,
        chat_id: chat_id.to_owned(),
        thread_id,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn resolve_telegram_known_session_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<ResolvedKnownChannelSessionTarget> {
    let configured_account_ids = config.telegram.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .telegram
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let parsed = parse_telegram_session_send_target(config, session_id, scope)?;
    let (chat_id, thread_id) = match parsed {
        KnownChannelSessionSendTarget::Telegram {
            chat_id, thread_id, ..
        } => (chat_id, thread_id),
        KnownChannelSessionSendTarget::Feishu { .. }
        | KnownChannelSessionSendTarget::Line { .. }
        | KnownChannelSessionSendTarget::Matrix { .. }
        | KnownChannelSessionSendTarget::Wecom { .. }
        | KnownChannelSessionSendTarget::WhatsApp { .. } => {
            return Err(format!(
                "sessions_send_channel_unsupported: `{session_id}` resolved to a non-telegram target"
            ));
        }
    };
    let target_id = match thread_id.as_deref() {
        Some(thread_id) => format!("{chat_id}:{thread_id}"),
        None => chat_id.clone(),
    };
    let session_shape = if thread_id.is_some() {
        "telegram_thread"
    } else {
        "telegram_chat"
    };

    Ok(ResolvedKnownChannelSessionTarget {
        route_session_id: session_id.trim().to_owned(),
        channel_id: "telegram".to_owned(),
        account_id,
        session_shape,
        target_kind: ChannelOutboundTargetKind::Conversation,
        target_id,
        raw_scope: scoped_path.to_vec(),
        conversation_id: Some(chat_id),
        participant_id: None,
        thread_id,
        reply_message_id: None,
        chat_type: None,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn parse_feishu_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<KnownChannelSessionSendTarget> {
    let configured_account_ids = config.feishu.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .feishu
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let Some(conversation_id) = scoped_path
        .first()
        .map(String::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err(format!("sessions_send_channel_unsupported: `{session_id}`"));
    };

    let reply_message_id = parse_feishu_session_reply_message_id(scoped_path);

    Ok(KnownChannelSessionSendTarget::Feishu {
        account_id,
        conversation_id: conversation_id.to_owned(),
        reply_message_id,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn parse_line_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<KnownChannelSessionSendTarget> {
    let configured_account_ids = config.line.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .line
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let split_scope = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let account_id = split_scope.0;
    let scoped_path = split_scope.1;
    let address = scoped_path
        .first()
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("sessions_send_channel_unsupported: `{session_id}`"))?;

    Ok(KnownChannelSessionSendTarget::Line {
        account_id,
        address: address.to_owned(),
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn resolve_feishu_known_session_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<ResolvedKnownChannelSessionTarget> {
    let configured_account_ids = config.feishu.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .feishu
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let parsed = parse_feishu_session_send_target(config, session_id, scope)?;
    let (conversation_id, reply_message_id) = match parsed {
        KnownChannelSessionSendTarget::Feishu {
            conversation_id,
            reply_message_id,
            ..
        } => (conversation_id, reply_message_id),
        KnownChannelSessionSendTarget::Telegram { .. }
        | KnownChannelSessionSendTarget::Line { .. }
        | KnownChannelSessionSendTarget::Matrix { .. }
        | KnownChannelSessionSendTarget::Wecom { .. }
        | KnownChannelSessionSendTarget::WhatsApp { .. } => {
            return Err(format!(
                "sessions_send_channel_unsupported: `{session_id}` resolved to a non-feishu target"
            ));
        }
    };
    let target_kind = if reply_message_id.is_some() {
        ChannelOutboundTargetKind::MessageReply
    } else {
        ChannelOutboundTargetKind::ReceiveId
    };
    let target_id = reply_message_id
        .clone()
        .unwrap_or_else(|| conversation_id.clone());
    let session_shape = if reply_message_id.is_some() {
        "feishu_reply"
    } else {
        "feishu_chat"
    };
    let participant_id = scoped_path
        .get(1)
        .filter(|segment| !looks_like_feishu_message_id(segment.as_str()))
        .cloned();
    let thread_id = if reply_message_id.is_some() && scoped_path.len() >= 4 {
        scoped_path.get(scoped_path.len() - 2).cloned()
    } else if scoped_path.len() >= 3 {
        scoped_path.last().cloned()
    } else if scoped_path.len() == 2 {
        let candidate_thread_id = scoped_path.get(1);
        let candidate_thread_id =
            candidate_thread_id.filter(|value| looks_like_feishu_message_id(value.as_str()));
        candidate_thread_id.cloned()
    } else {
        None
    };

    Ok(ResolvedKnownChannelSessionTarget {
        route_session_id: session_id.trim().to_owned(),
        channel_id: "feishu".to_owned(),
        account_id,
        session_shape,
        target_kind,
        target_id,
        raw_scope: scoped_path.to_vec(),
        conversation_id: Some(conversation_id),
        participant_id,
        thread_id,
        reply_message_id,
        chat_type: None,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn parse_matrix_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<KnownChannelSessionSendTarget> {
    let configured_account_ids = config.matrix.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .matrix
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let Some(room_id) = scoped_path
        .first()
        .map(String::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err(format!("sessions_send_channel_unsupported: `{session_id}`"));
    };

    Ok(KnownChannelSessionSendTarget::Matrix {
        account_id,
        room_id: room_id.to_owned(),
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn resolve_matrix_known_session_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<ResolvedKnownChannelSessionTarget> {
    let configured_account_ids = config.matrix.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .matrix
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let parsed = parse_matrix_session_send_target(config, session_id, scope)?;
    let room_id = match parsed {
        KnownChannelSessionSendTarget::Matrix { room_id, .. } => room_id,
        KnownChannelSessionSendTarget::Telegram { .. }
        | KnownChannelSessionSendTarget::Feishu { .. }
        | KnownChannelSessionSendTarget::Line { .. }
        | KnownChannelSessionSendTarget::Wecom { .. }
        | KnownChannelSessionSendTarget::WhatsApp { .. } => {
            return Err(format!(
                "sessions_send_channel_unsupported: `{session_id}` resolved to a non-matrix target"
            ));
        }
    };
    let participant_id = scoped_path.get(1).cloned();
    let thread_id = scoped_path.get(2).cloned();

    Ok(ResolvedKnownChannelSessionTarget {
        route_session_id: session_id.trim().to_owned(),
        channel_id: "matrix".to_owned(),
        account_id,
        session_shape: "matrix_room",
        target_kind: ChannelOutboundTargetKind::Conversation,
        target_id: room_id.clone(),
        raw_scope: scoped_path.to_vec(),
        conversation_id: Some(room_id),
        participant_id,
        thread_id,
        reply_message_id: None,
        chat_type: None,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn parse_wecom_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<KnownChannelSessionSendTarget> {
    let configured_account_ids = config.wecom.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .wecom
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let Some(conversation_id) = scoped_path
        .first()
        .map(String::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err(format!("sessions_send_channel_unsupported: `{session_id}`"));
    };

    let chat_type = if scoped_path.len() >= 2 {
        Some(2)
    } else {
        None
    };
    Ok(KnownChannelSessionSendTarget::Wecom {
        account_id,
        conversation_id: conversation_id.to_owned(),
        chat_type,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn parse_whatsapp_session_send_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<KnownChannelSessionSendTarget> {
    let configured_account_ids = config.whatsapp.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .whatsapp
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let split_scope = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let account_id = split_scope.0;
    let scoped_path = split_scope.1;
    let maybe_address = scoped_path.first();
    let address = maybe_address
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("sessions_send_channel_unsupported: `{session_id}`"))?;

    Ok(KnownChannelSessionSendTarget::WhatsApp {
        account_id,
        address: address.to_owned(),
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn resolve_wecom_known_session_target(
    config: &LoongClawConfig,
    session_id: &str,
    scope: &[String],
) -> CliResult<ResolvedKnownChannelSessionTarget> {
    let configured_account_ids = config.wecom.configured_account_ids();
    let runtime_account_ids = configured_runtime_account_ids(
        configured_account_ids.as_slice(),
        |configured_account_id| {
            config
                .wecom
                .resolve_account(Some(configured_account_id))
                .map(|resolved| resolved.account.id)
        },
    );
    let (account_id, scoped_path) = split_known_channel_account_and_scope(
        scope,
        configured_account_ids.as_slice(),
        runtime_account_ids.as_slice(),
    );
    let parsed = parse_wecom_session_send_target(config, session_id, scope)?;
    let (conversation_id, chat_type) = match parsed {
        KnownChannelSessionSendTarget::Wecom {
            conversation_id,
            chat_type,
            ..
        } => (conversation_id, chat_type),
        KnownChannelSessionSendTarget::Telegram { .. }
        | KnownChannelSessionSendTarget::Feishu { .. }
        | KnownChannelSessionSendTarget::Line { .. }
        | KnownChannelSessionSendTarget::Matrix { .. }
        | KnownChannelSessionSendTarget::WhatsApp { .. } => {
            return Err(format!(
                "sessions_send_channel_unsupported: `{session_id}` resolved to a non-wecom target"
            ));
        }
    };
    let participant_id = scoped_path.get(1).cloned();
    let session_shape = if chat_type == Some(2) {
        "wecom_group"
    } else {
        "wecom_conversation"
    };

    Ok(ResolvedKnownChannelSessionTarget {
        route_session_id: session_id.trim().to_owned(),
        channel_id: "wecom".to_owned(),
        account_id,
        session_shape,
        target_kind: ChannelOutboundTargetKind::Conversation,
        target_id: conversation_id.clone(),
        raw_scope: scoped_path.to_vec(),
        conversation_id: Some(conversation_id),
        participant_id,
        thread_id: None,
        reply_message_id: None,
        chat_type,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn configured_runtime_account_ids(
    configured_account_ids: &[String],
    resolve_runtime_account_id: impl Fn(&str) -> CliResult<String>,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut runtime_account_ids = Vec::new();
    for configured_account_id in configured_account_ids {
        let Ok(runtime_account_id) = resolve_runtime_account_id(configured_account_id.as_str())
        else {
            continue;
        };
        let runtime_account_id = runtime_account_id.trim();
        if runtime_account_id.is_empty() || !seen.insert(runtime_account_id.to_owned()) {
            continue;
        }
        runtime_account_ids.push(runtime_account_id.to_owned());
    }
    runtime_account_ids
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn split_known_channel_account_and_scope<'a>(
    scope: &'a [String],
    configured_account_ids: &[String],
    runtime_account_ids: &[String],
) -> (Option<String>, &'a [String]) {
    let mut scoped_path = scope;
    let configured_account_id = scoped_path
        .first()
        .and_then(|segment| segment.strip_prefix("cfg="))
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && configured_account_ids
                    .iter()
                    .any(|known| known.trim() == *value)
        })
        .map(str::to_owned);
    if configured_account_id.is_some() {
        scoped_path = scoped_path.get(1..).unwrap_or_default();
    }

    let runtime_account_id = scoped_path.first().and_then(|value| {
        let normalized_requested = normalize_channel_account_id(value);
        runtime_account_ids.iter().find_map(|known| {
            (normalize_channel_account_id(known) == normalized_requested).then(|| known.clone())
        })
    });
    if runtime_account_id.is_some() {
        scoped_path = scoped_path.get(1..).unwrap_or_default();
    }

    (configured_account_id.or(runtime_account_id), scoped_path)
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn parse_feishu_session_reply_message_id(scope: &[String]) -> Option<String> {
    if scope.len() <= 3 {
        return None;
    }

    let reply_message_id = scope.last()?;
    if !looks_like_feishu_message_id(reply_message_id.as_str()) {
        return None;
    }

    Some(reply_message_id.clone())
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn looks_like_feishu_message_id(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("om_")
}

// ============================================================================
// process_channel_batch (runtime-coupled due to ChannelOperationRuntimeTracker and ChannelTurnFeedbackPolicy)
// ============================================================================

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(in crate::channel) async fn process_channel_batch<A, F>(
    adapter: &mut A,
    batch: Vec<ChannelInboundMessage>,
    runtime: Option<&ChannelOperationRuntimeTracker>,
    mut process: F,
) -> CliResult<bool>
where
    A: ChannelAdapter + Send + ?Sized,
    F: FnMut(ChannelInboundMessage, ChannelTurnFeedbackPolicy) -> ChannelProcessFuture,
{
    if batch.is_empty() {
        adapter.complete_batch().await?;
        return Ok(false);
    }

    for message in &batch {
        if let Some(runtime) = runtime {
            runtime.mark_run_start().await?;
        }

        let result = async {
            let turn_feedback_policy = adapter.turn_feedback_policy();
            let reply = process(message.clone(), turn_feedback_policy).await?;
            let outbound = ChannelOutboundMessage::Text(reply);
            let streaming_mode = adapter.streaming_mode();
            adapter
                .send_message_streaming(&message.reply_target, &outbound, streaming_mode)
                .await?;
            adapter.ack_inbound(message).await?;
            Ok::<(), String>(())
        }
        .await;

        if let Some(runtime) = runtime {
            runtime.mark_run_end().await?;
        }

        result?;
    }

    adapter.complete_batch().await?;
    Ok(true)
}
