#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use std::collections::BTreeSet;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-signal",
    feature = "channel-slack",
    feature = "channel-wecom"
))]
use async_trait::async_trait;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use super::state::ChannelOperationRuntimeTracker;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use super::turn_feedback::ChannelTurnFeedbackPolicy;
use crate::CliResult;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use crate::config::LoongClawConfig;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
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
    feature = "channel-matrix",
    feature = "channel-wecom"
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
    Matrix {
        account_id: Option<String>,
        room_id: String,
    },
    Wecom {
        account_id: Option<String>,
        conversation_id: String,
        chat_type: Option<u8>,
    },
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
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
        "matrix" => parse_matrix_session_send_target(config, session_id, scope.as_slice()),
        "wecom" | "wechat-work" | "qywx" => {
            parse_wecom_session_send_target(config, session_id, scope.as_slice())
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

    let reply_message_id = match scoped_path {
        [_conversation_id] => None,
        [_conversation_id, trailing] if looks_like_feishu_message_id(trailing.as_str()) => {
            Some(trailing.clone())
        }
        [_conversation_id, _participant_id] => None,
        _ => scoped_path.last().cloned(),
    };

    Ok(KnownChannelSessionSendTarget::Feishu {
        account_id,
        conversation_id: conversation_id.to_owned(),
        reply_message_id,
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
    feature = "channel-matrix",
    feature = "channel-wecom"
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
