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
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use std::future::Future;
use std::path::PathBuf;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use std::pin::Pin;
use std::{fmt, str::FromStr};

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
use serde::Serialize;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use serde_json::Value;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
use super::runtime_state::ChannelOperationRuntimeTracker;
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
use crate::conversation::{
    ConversationSessionAddress, encode_route_session_segment, parse_route_session_id,
};

#[derive(Debug, Clone, Default)]
pub struct ChannelDelivery {
    #[allow(dead_code)]
    pub ack_cursor: Option<String>,
    #[allow(dead_code)]
    pub source_message_id: Option<String>,
    #[allow(dead_code)]
    pub sender_principal_key: Option<String>,
    #[allow(dead_code)]
    pub thread_root_id: Option<String>,
    #[allow(dead_code)]
    pub parent_message_id: Option<String>,
    #[allow(dead_code)]
    pub resources: Vec<ChannelDeliveryResource>,
    #[allow(dead_code)]
    pub feishu_callback: Option<ChannelDeliveryFeishuCallback>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelDeliveryResource {
    pub resource_type: String,
    pub file_key: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelDeliveryFeishuCallback {
    pub callback_token: Option<String>,
    pub open_message_id: Option<String>,
    pub open_chat_id: Option<String>,
    pub operator_open_id: Option<String>,
    pub deferred_context_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelSendReceipt {
    pub channel: &'static str,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChannelPlatform {
    Telegram,
    Feishu,
    Matrix,
    Wecom,
    WhatsApp,
    Irc,
}

impl ChannelPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Feishu => "feishu",
            Self::Matrix => "matrix",
            Self::Wecom => "wecom",
            Self::WhatsApp => "whatsapp",
            Self::Irc => "irc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSession {
    pub platform: ChannelPlatform,
    pub configured_account_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: String,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
}

impl ChannelSession {
    pub fn new(platform: ChannelPlatform, conversation_id: impl Into<String>) -> Self {
        Self {
            platform,
            configured_account_id: None,
            account_id: None,
            conversation_id: conversation_id.into(),
            participant_id: None,
            thread_id: None,
        }
    }

    pub fn with_account(
        platform: ChannelPlatform,
        account_id: impl Into<String>,
        conversation_id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            configured_account_id: None,
            account_id: Some(account_id.into()),
            conversation_id: conversation_id.into(),
            participant_id: None,
            thread_id: None,
        }
    }

    pub fn with_thread(
        platform: ChannelPlatform,
        conversation_id: impl Into<String>,
        thread_id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            configured_account_id: None,
            account_id: None,
            conversation_id: conversation_id.into(),
            participant_id: None,
            thread_id: Some(thread_id.into()),
        }
    }

    pub fn with_account_and_thread(
        platform: ChannelPlatform,
        account_id: impl Into<String>,
        conversation_id: impl Into<String>,
        thread_id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            configured_account_id: None,
            account_id: Some(account_id.into()),
            conversation_id: conversation_id.into(),
            participant_id: None,
            thread_id: Some(thread_id.into()),
        }
    }

    pub fn with_configured_account_id(mut self, configured_account_id: impl Into<String>) -> Self {
        self.configured_account_id = Some(configured_account_id.into());
        self
    }

    pub fn with_participant_id(mut self, participant_id: impl Into<String>) -> Self {
        self.participant_id = Some(participant_id.into());
        self
    }

    pub fn with_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    pub fn session_key(&self) -> String {
        let configured_account_id = self
            .configured_account_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let account_id = self
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let conversation_id = self.conversation_id.trim();
        let participant_id = self
            .participant_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let thread_id = self
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let mut parts = vec![self.platform.as_str().to_owned()];
        if let Some(configured_account_id) =
            configured_account_id.filter(|value| Some(*value) != account_id)
        {
            parts.push(format!("cfg={configured_account_id}"));
        }
        if let Some(account_id) = account_id {
            parts.push(encode_route_session_segment(account_id));
        }
        parts.push(encode_route_session_segment(conversation_id));
        if let Some(participant_id) = participant_id {
            parts.push(encode_route_session_segment(participant_id));
        }
        if let Some(thread_id) = thread_id {
            parts.push(encode_route_session_segment(thread_id));
        }
        parts.join(":")
    }

    pub fn conversation_address(&self) -> ConversationSessionAddress {
        let mut address = ConversationSessionAddress::from_session_id(self.session_key())
            .with_channel_scope(self.platform.as_str(), self.conversation_id.clone());
        if let Some(account_id) = self.account_id.as_deref() {
            address = address.with_account_id(account_id);
        }
        if let Some(thread_id) = self.thread_id.as_deref() {
            address = address.with_thread_id(thread_id);
        }
        address
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOutboundTargetKind {
    Conversation,
    MessageReply,
    ReceiveId,
    Address,
    Endpoint,
}

impl ChannelOutboundTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::MessageReply => "message_reply",
            Self::ReceiveId => "receive_id",
            Self::Address => "address",
            Self::Endpoint => "endpoint",
        }
    }
}

impl fmt::Display for ChannelOutboundTargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ChannelOutboundTargetKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
        match normalized.as_str() {
            "conversation" => Ok(Self::Conversation),
            "message_reply" => Ok(Self::MessageReply),
            "receive_id" => Ok(Self::ReceiveId),
            "address" => Ok(Self::Address),
            "endpoint" => Ok(Self::Endpoint),
            _ => Err(format!(
                "unsupported channel target kind `{value}`; expected conversation, message_reply, receive_id, address, or endpoint"
            )),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelOutboundDeliveryOptions {
    pub idempotency_key: Option<String>,
    pub feishu_receive_id_type: Option<String>,
    pub feishu_reply_in_thread: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelOutboundTarget {
    pub platform: ChannelPlatform,
    pub kind: ChannelOutboundTargetKind,
    pub id: String,
    pub options: ChannelOutboundDeliveryOptions,
}

impl ChannelOutboundTarget {
    pub fn new(
        platform: ChannelPlatform,
        kind: ChannelOutboundTargetKind,
        id: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            kind,
            id: id.into(),
            options: ChannelOutboundDeliveryOptions::default(),
        }
    }

    pub fn telegram_chat(chat_id: i64) -> Self {
        Self::new(
            ChannelPlatform::Telegram,
            ChannelOutboundTargetKind::Conversation,
            chat_id.to_string(),
        )
    }

    pub fn feishu_message_reply(message_id: impl Into<String>) -> Self {
        Self::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::MessageReply,
            message_id,
        )
    }

    pub fn feishu_receive_id(receive_id: impl Into<String>) -> Self {
        Self::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::ReceiveId,
            receive_id,
        )
    }

    pub fn trimmed_id(&self) -> CliResult<&str> {
        let id = self.id.trim();
        if id.is_empty() {
            return Err(format!(
                "channel target id is empty for {} {}",
                self.platform.as_str(),
                self.kind.as_str()
            ));
        }
        Ok(id)
    }

    pub fn with_feishu_receive_id_type(mut self, receive_id_type: impl Into<String>) -> Self {
        self.options.feishu_receive_id_type = Some(receive_id_type.into());
        self
    }

    pub fn with_idempotency_key(mut self, idempotency_key: impl Into<String>) -> Self {
        self.options.idempotency_key = Some(idempotency_key.into());
        self
    }

    pub fn feishu_receive_id_type(&self) -> Option<&str> {
        self.options
            .feishu_receive_id_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn with_feishu_reply_in_thread(mut self, reply_in_thread: bool) -> Self {
        self.options.feishu_reply_in_thread = Some(reply_in_thread);
        self
    }

    pub fn idempotency_key(&self) -> Option<&str> {
        self.options
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn feishu_reply_in_thread(&self) -> Option<bool> {
        self.options.feishu_reply_in_thread
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
#[derive(Debug, Clone)]
pub struct ChannelInboundMessage {
    pub session: ChannelSession,
    pub reply_target: ChannelOutboundTarget,
    pub text: String,
    pub delivery: ChannelDelivery,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ChannelResolvedAcpTurnHints {
    pub(super) bootstrap_mcp_servers: Vec<String>,
    pub(super) working_directory: Option<PathBuf>,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelOutboundMessage {
    Text(String),
    MarkdownCard(String),
    Post(Value),
    Image { image_key: String },
    File { file_key: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub enum ChannelStreamingMode {
    #[default]
    Off,
    Draft,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeishuChannelSendRequest {
    pub receive_id: String,
    pub receive_id_type: Option<String>,
    pub text: Option<String>,
    pub post_json: Option<String>,
    pub image_key: Option<String>,
    pub file_key: Option<String>,
    pub image_path: Option<String>,
    pub file_path: Option<String>,
    pub file_type: Option<String>,
    pub card: bool,
    pub uuid: Option<String>,
}

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

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(super) type ChannelProcessFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send>>;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-dingtalk",
    feature = "channel-email",
    feature = "channel-feishu",
    feature = "channel-google-chat",
    feature = "channel-webhook",
    feature = "channel-nostr",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-mattermost",
    feature = "channel-nextcloud-talk",
    feature = "channel-signal",
    feature = "channel-slack",
    feature = "channel-synology-chat",
    feature = "channel-irc",
    feature = "channel-twitch",
    feature = "channel-teams",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-imessage"
))]
pub(super) type ChannelCommandFuture<'a> = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'a>>;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum KnownChannelSessionSendTarget {
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
pub(super) fn parse_known_channel_session_send_target(
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

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(super) async fn process_channel_batch<A, F>(
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
