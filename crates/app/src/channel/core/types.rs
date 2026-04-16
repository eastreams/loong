use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::{fmt, str::FromStr};

use serde::Serialize;
use serde_json::Value;

use crate::CliResult;
use crate::conversation::{ConversationSessionAddress, encode_route_session_segment};

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
        if let Some(participant_id) = self.participant_id.as_deref() {
            address = address.with_participant_id(participant_id);
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
    pub feishu_reply_chat_id: Option<String>,
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

    pub fn with_feishu_reply_chat_id(mut self, chat_id: impl Into<String>) -> Self {
        self.options.feishu_reply_chat_id = Some(chat_id.into());
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

    pub fn feishu_reply_chat_id(&self) -> Option<&str> {
        self.options
            .feishu_reply_chat_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

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
pub(in crate::channel) struct ChannelResolvedAcpTurnHints {
    pub(in crate::channel) bootstrap_mcp_servers: Vec<String>,
    pub(in crate::channel) working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelOutboundMessage {
    Text(String),
    MarkdownCard(String),
    Post(Value),
    Image { image_key: String },
    File { file_key: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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

pub(in crate::channel) type ChannelProcessFuture =
    Pin<Box<dyn Future<Output = CliResult<String>> + Send>>;

pub(in crate::channel) type ChannelCommandFuture<'a> =
    Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'a>>;
