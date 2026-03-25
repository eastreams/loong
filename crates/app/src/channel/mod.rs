#[cfg(feature = "channel-telegram")]
use std::time::Duration;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use std::{
    collections::BTreeSet,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use std::{fmt, str::FromStr};

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use async_trait::async_trait;
use serde::Serialize;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use serde_json::Value;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use tokio::sync::Notify;
#[cfg(feature = "channel-telegram")]
use tokio::time::sleep;

use crate::CliResult;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use crate::KernelContext;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use crate::acp::{AcpConversationTurnOptions, AcpTurnProvenance};
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context_with_config};

#[cfg(feature = "channel-feishu")]
use super::config::ResolvedFeishuChannelConfig;
#[cfg(feature = "channel-matrix")]
use super::config::ResolvedMatrixChannelConfig;
#[cfg(feature = "channel-telegram")]
use super::config::ResolvedTelegramChannelConfig;
#[cfg(feature = "channel-wecom")]
use super::config::ResolvedWecomChannelConfig;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use super::config::{ChannelResolvedAccountRoute, LoongClawConfig, normalize_channel_account_id};
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use super::conversation::{
    ConversationIngressChannel, ConversationIngressContext, ConversationIngressDelivery,
    ConversationIngressDeliveryResource, ConversationIngressFeishuCallbackContext,
    ConversationIngressPrivateContext, ConversationSessionAddress, encode_route_session_segment,
    parse_route_session_id,
};
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
use super::conversation::{ConversationTurnCoordinator, ProviderErrorMode};

#[cfg(feature = "channel-feishu")]
mod feishu;
#[cfg(feature = "channel-matrix")]
mod matrix;
mod registry;
mod runtime_state;
#[cfg(feature = "channel-telegram")]
mod telegram;
#[cfg(feature = "channel-wecom")]
mod wecom;

pub use registry::{
    CHANNEL_OPERATION_SEND_ID, CHANNEL_OPERATION_SERVE_ID, ChannelCapability,
    ChannelCatalogCommandFamilyDescriptor, ChannelCatalogEntry, ChannelCatalogImplementationStatus,
    ChannelCatalogOperation, ChannelCatalogOperationAvailability,
    ChannelCatalogOperationRequirement, ChannelCommandFamilyDescriptor, ChannelDoctorCheckSpec,
    ChannelDoctorCheckTrigger, ChannelDoctorOperationSpec, ChannelInventory,
    ChannelOnboardingDescriptor, ChannelOnboardingStrategy, ChannelOperationDescriptor,
    ChannelOperationHealth, ChannelOperationStatus, ChannelRuntimeCommandDescriptor,
    ChannelStatusSnapshot, ChannelSurface, FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    FEISHU_COMMAND_FAMILY_DESCRIPTOR, FEISHU_RUNTIME_COMMAND_DESCRIPTOR,
    MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR, MATRIX_COMMAND_FAMILY_DESCRIPTOR,
    MATRIX_RUNTIME_COMMAND_DESCRIPTOR, TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    TELEGRAM_COMMAND_FAMILY_DESCRIPTOR, TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
    WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR, WECOM_COMMAND_FAMILY_DESCRIPTOR,
    WECOM_RUNTIME_COMMAND_DESCRIPTOR, catalog_only_channel_entries, channel_inventory,
    channel_status_snapshots, list_channel_catalog, normalize_channel_catalog_id,
    normalize_channel_platform, resolve_channel_catalog_command_family_descriptor,
    resolve_channel_catalog_entry, resolve_channel_catalog_operation,
    resolve_channel_command_family_descriptor, resolve_channel_doctor_operation_spec,
    resolve_channel_onboarding_descriptor, resolve_channel_operation_descriptor,
    resolve_channel_runtime_command_descriptor,
};
pub use runtime_state::ChannelOperationRuntime;
use runtime_state::ChannelOperationRuntimeTracker;

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
}

impl ChannelPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Feishu => "feishu",
            Self::Matrix => "matrix",
            Self::Wecom => "wecom",
        }
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSession {
    pub platform: ChannelPlatform,
    pub configured_account_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: String,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
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

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOutboundTargetKind {
    Conversation,
    MessageReply,
    ReceiveId,
    Address,
    Endpoint,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
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

pub use self::ChannelOutboundTargetKind as ChannelCatalogTargetKind;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelOutboundDeliveryOptions {
    pub idempotency_key: Option<String>,
    pub feishu_receive_id_type: Option<String>,
    pub feishu_reply_in_thread: Option<bool>,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelOutboundTarget {
    pub platform: ChannelPlatform,
    pub kind: ChannelOutboundTargetKind,
    pub id: String,
    pub options: ChannelOutboundDeliveryOptions,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
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
    feature = "channel-wecom"
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
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ChannelResolvedAcpTurnHints {
    bootstrap_mcp_servers: Vec<String>,
    working_directory: Option<PathBuf>,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
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
    feature = "channel-wecom"
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
    feature = "channel-wecom"
))]
#[allow(dead_code)]
#[async_trait]
pub trait ChannelAdapter {
    fn name(&self) -> &str;
    fn streaming_mode(&self) -> ChannelStreamingMode {
        ChannelStreamingMode::Off
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
    feature = "channel-wecom"
))]
type ChannelProcessFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send>>;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
type ChannelCommandFuture<'a> = Pin<Box<dyn Future<Output = CliResult<()>> + Send + 'a>>;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum KnownChannelSessionSendTarget {
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
fn parse_known_channel_session_send_target(
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

async fn process_channel_batch<A, F>(
    adapter: &mut A,
    batch: Vec<ChannelInboundMessage>,
    runtime: Option<&ChannelOperationRuntimeTracker>,
    mut process: F,
) -> CliResult<bool>
where
    A: ChannelAdapter + Send + ?Sized,
    F: FnMut(ChannelInboundMessage) -> ChannelProcessFuture,
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
            let reply = process(message.clone()).await?;
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

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone)]
struct ChannelCommandContext<R> {
    resolved_path: PathBuf,
    config: LoongClawConfig,
    resolved: R,
    route: ChannelResolvedAccountRoute,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
impl<R> ChannelCommandContext<R> {
    fn emit_route_notice(&self, platform: ChannelPlatform) {
        if let Some(notice) = render_channel_route_notice(platform, &self.route) {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("warning: {notice}");
            }
        }
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
trait ChannelResolvedRuntimeAccount {
    fn runtime_account_id(&self) -> &str;
    fn runtime_account_label(&self) -> &str;
}

#[cfg(feature = "channel-telegram")]
impl ChannelResolvedRuntimeAccount for ResolvedTelegramChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-feishu")]
impl ChannelResolvedRuntimeAccount for ResolvedFeishuChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-matrix")]
impl ChannelResolvedRuntimeAccount for ResolvedMatrixChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(feature = "channel-wecom")]
impl ChannelResolvedRuntimeAccount for ResolvedWecomChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
async fn run_channel_send_command<R, F, G>(
    context: ChannelCommandContext<R>,
    spec: ChannelSendCommandSpec,
    send: F,
    render_success: G,
) -> CliResult<()>
where
    F: for<'a> FnOnce(&'a ChannelCommandContext<R>) -> ChannelCommandFuture<'a>,
    G: FnOnce(&ChannelCommandContext<R>) -> String,
{
    crate::runtime_env::initialize_runtime_environment(
        &context.config,
        Some(context.resolved_path.as_path()),
    );
    context.emit_route_notice(spec.family.runtime.platform);
    send(&context).await?;

    #[allow(clippy::print_stdout)]
    {
        println!("{}", render_success(&context));
    }
    Ok(())
}
#[cfg(feature = "channel-telegram")]
fn load_telegram_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let (resolved_path, config) = super::config::load(config_path)?;
    build_telegram_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-telegram")]
fn build_telegram_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let resolved = config.telegram.resolve_account(account_id)?;
    let route = config
        .telegram
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "telegram account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }
    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(feature = "channel-feishu")]
fn load_feishu_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedFeishuChannelConfig>> {
    let (resolved_path, config) = super::config::load(config_path)?;
    build_feishu_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-feishu")]
fn build_feishu_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedFeishuChannelConfig>> {
    let resolved = crate::feishu::resolve_requested_feishu_account(
        &config.feishu,
        account_id,
        "rerun with `--account <configured_account_id>` using one of those configured accounts",
    )?;
    let route = config
        .feishu
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "feishu account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }
    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(feature = "channel-matrix")]
fn load_matrix_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMatrixChannelConfig>> {
    let (resolved_path, config) = super::config::load(config_path)?;
    build_matrix_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-matrix")]
fn build_matrix_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMatrixChannelConfig>> {
    let resolved = config.matrix.resolve_account(account_id)?;
    let route = config
        .matrix
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "matrix account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }
    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(feature = "channel-wecom")]
fn load_wecom_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWecomChannelConfig>> {
    let (resolved_path, config) = super::config::load(config_path)?;
    build_wecom_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-wecom")]
fn build_wecom_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWecomChannelConfig>> {
    let resolved = config.wecom.resolve_account(account_id)?;
    let route = config
        .wecom
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "wecom account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }
    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, Copy)]
struct ChannelSendCommandSpec {
    family: ChannelCommandFamilyDescriptor,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, Copy)]
struct ChannelServeRuntimeSpec<'a> {
    platform: ChannelPlatform,
    operation_id: &'static str,
    account_id: &'a str,
    account_label: &'a str,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone, Copy)]
struct ChannelServeCommandSpec {
    family: ChannelCommandFamilyDescriptor,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
#[derive(Debug, Clone)]
pub struct ChannelServeStopHandle {
    requested: Arc<AtomicBool>,
    stop: Arc<Notify>,
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
impl ChannelServeStopHandle {
    pub fn new() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(Notify::new()),
        }
    }

    pub fn request_stop(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.stop.notify_waiters();
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    async fn wait(&self) {
        if self.is_requested() {
            return;
        }
        let notified = self.stop.notified();
        tokio::pin!(notified);
        if self.is_requested() {
            return;
        }
        notified.await;
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn channel_runtime_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn ensure_channel_operation_runtime_slot_available_in_dir(
    runtime_dir: &std::path::Path,
    spec: ChannelServeRuntimeSpec<'_>,
) -> CliResult<()> {
    let now = channel_runtime_now_ms();
    runtime_state::prune_inactive_channel_operation_runtime_files_for_account_from_dir(
        runtime_dir,
        spec.platform,
        spec.operation_id,
        Some(spec.account_id),
        now,
    )?;
    let Some(runtime) = runtime_state::load_channel_operation_runtime_for_account_from_dir(
        runtime_dir,
        spec.platform,
        spec.operation_id,
        spec.account_id,
        now,
    ) else {
        return Ok(());
    };
    if runtime.running_instances == 0 {
        return Ok(());
    }

    let pid = runtime
        .pid
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    Err(format!(
        "{} account `{}` already has an active {} runtime (pid={}, running_instances={}); stop the existing instance or wait for it to become stale before restarting",
        spec.platform.as_str(),
        spec.account_id,
        spec.operation_id,
        pid,
        runtime.running_instances
    ))
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
async fn with_channel_serve_runtime<T, F, Fut>(
    spec: ChannelServeRuntimeSpec<'_>,
    run: F,
) -> CliResult<T>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>) -> Fut,
    Fut: Future<Output = CliResult<T>>,
{
    ensure_channel_operation_runtime_slot_available_in_dir(
        runtime_state::default_channel_runtime_state_dir().as_path(),
        spec,
    )?;
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
        )
        .await?,
    );
    let result = run(runtime.clone()).await;
    let shutdown_result = runtime.shutdown().await;
    match result {
        Err(error) => Err(error),
        Ok(value) => {
            shutdown_result?;
            Ok(value)
        }
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
async fn with_channel_serve_runtime_with_stop<F, Fut>(
    spec: ChannelServeRuntimeSpec<'_>,
    stop: ChannelServeStopHandle,
    run: F,
) -> CliResult<()>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>, ChannelServeStopHandle) -> Fut,
    Fut: Future<Output = CliResult<()>>,
{
    with_channel_serve_runtime(spec, move |runtime| run(runtime, stop)).await
}

#[cfg(test)]
async fn with_channel_serve_runtime_with_stop_in_dir<F, Fut>(
    runtime_dir: &std::path::Path,
    process_id: u32,
    spec: ChannelServeRuntimeSpec<'_>,
    stop: ChannelServeStopHandle,
    run: F,
) -> CliResult<()>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>, ChannelServeStopHandle) -> Fut,
    Fut: Future<Output = CliResult<()>>,
{
    with_channel_serve_runtime_in_dir(runtime_dir, process_id, spec, move |runtime| {
        run(runtime, stop)
    })
    .await
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
async fn run_channel_serve_command_with_stop<R, V, F>(
    context: ChannelCommandContext<R>,
    spec: ChannelServeCommandSpec,
    validate: V,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
    run: F,
) -> CliResult<()>
where
    R: ChannelResolvedRuntimeAccount,
    V: FnOnce(&R) -> CliResult<()>,
    F: for<'a> FnOnce(
        &'a ChannelCommandContext<R>,
        KernelContext,
        Arc<ChannelOperationRuntimeTracker>,
        ChannelServeStopHandle,
    ) -> ChannelCommandFuture<'a>,
{
    validate(&context.resolved)?;
    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(
            &context.config,
            Some(context.resolved_path.as_path()),
        );
    }
    let kernel_ctx = bootstrap_kernel_context_with_config(
        spec.family.runtime.serve_bootstrap_agent_id,
        DEFAULT_TOKEN_TTL_S,
        &context.config,
    )?;
    let runtime_account_id = context.resolved.runtime_account_id().to_owned();
    let runtime_account_label = context.resolved.runtime_account_label().to_owned();

    with_channel_serve_runtime_with_stop(
        ChannelServeRuntimeSpec {
            platform: spec.family.runtime.platform,
            operation_id: spec.family.serve().id,
            account_id: runtime_account_id.as_str(),
            account_label: runtime_account_label.as_str(),
        },
        stop,
        move |runtime, stop| async move {
            context.emit_route_notice(spec.family.runtime.platform);
            run(&context, kernel_ctx, runtime, stop).await
        },
    )
    .await
}

#[cfg(feature = "channel-telegram")]
#[allow(clippy::print_stdout)] // CLI startup banner
async fn run_telegram_channel_with_context(
    context: ChannelCommandContext<ResolvedTelegramChannelConfig>,
    once: bool,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    validate_telegram_security_config(&context.resolved)?;
    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(
            &context.config,
            Some(context.resolved_path.as_path()),
        );
    }
    let kernel_ctx = bootstrap_kernel_context_with_config(
        "channel-telegram",
        DEFAULT_TOKEN_TTL_S,
        &context.config,
    )?;
    let token = context
        .resolved
        .bot_token()
        .ok_or_else(|| "telegram bot token missing (set telegram.bot_token or env)".to_owned())?;
    let route = context.route.clone();
    let resolved_path = context.resolved_path.clone();
    let resolved = context.resolved.clone();
    let batch_config = context.config.clone();
    let batch_kernel_ctx = Arc::new(crate::KernelContext {
        kernel: kernel_ctx.kernel.clone(),
        token: kernel_ctx.token.clone(),
    });
    let runtime_account_id = resolved.account.id.clone();
    let runtime_account_label = resolved.account.label.clone();

    with_channel_serve_runtime_with_stop(
        ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Telegram,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: runtime_account_id.as_str(),
            account_label: runtime_account_label.as_str(),
        },
        stop,
        move |runtime, stop| async move {
            let mut adapter = telegram::TelegramAdapter::new(&resolved, token);
            context.emit_route_notice(ChannelPlatform::Telegram);

            println!(
                "{} channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, timeout={}s)",
                adapter.name(),
                resolved_path.display(),
                resolved.configured_account_id,
                resolved.account.label,
                route.selected_by_default(),
                route.default_account_source.as_str(),
                resolved.polling_timeout_s
            );

            loop {
                let batch = tokio::select! {
                    _ = stop.wait() => break,
                    batch = adapter.receive_batch() => batch?,
                };
                let config = batch_config.clone();
                let kernel_ctx = batch_kernel_ctx.clone();
                let had_messages = process_channel_batch(
                    &mut adapter,
                    batch,
                    Some(runtime.as_ref()),
                    |message| {
                        let config = config.clone();
                        let kernel_ctx = kernel_ctx.clone();
                        let resolved_path = resolved_path.clone();
                        Box::pin(async move {
                            process_inbound_with_provider(
                                &config,
                                Some(resolved_path.as_path()),
                                &message,
                                Some(kernel_ctx.as_ref()),
                            )
                            .await
                        })
                    },
                )
                .await?;
                if !had_messages && once {
                    break;
                }
                if once {
                    break;
                }
                tokio::select! {
                    _ = stop.wait() => break,
                    _ = sleep(Duration::from_millis(250)) => {}
                }
            }
            Ok(())
        },
    )
    .await
}

#[cfg(feature = "channel-telegram")]
pub async fn run_telegram_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    once: bool,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_telegram_command_context(resolved_path, config, account_id)?;
    run_telegram_channel_with_context(context, once, stop, initialize_runtime_environment).await
}

#[cfg(test)]
async fn with_channel_serve_runtime_in_dir<T, F, Fut>(
    runtime_dir: &std::path::Path,
    process_id: u32,
    spec: ChannelServeRuntimeSpec<'_>,
    run: F,
) -> CliResult<T>
where
    F: FnOnce(Arc<ChannelOperationRuntimeTracker>) -> Fut,
    Fut: Future<Output = CliResult<T>>,
{
    ensure_channel_operation_runtime_slot_available_in_dir(runtime_dir, spec)?;
    let runtime = Arc::new(
        runtime_state::start_channel_operation_runtime_tracker_for_test(
            runtime_dir,
            spec.platform,
            spec.operation_id,
            spec.account_id,
            spec.account_label,
            process_id,
        )
        .await?,
    );
    let result = run(runtime.clone()).await;
    let shutdown_result = runtime.shutdown().await;
    match result {
        Err(error) => Err(error),
        Ok(value) => {
            shutdown_result?;
            Ok(value)
        }
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_telegram_channel(
    config_path: Option<&str>,
    once: bool,
    account_id: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-telegram") {
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(not(feature = "channel-telegram"))]
    {
        let _ = (config_path, once, account_id);
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(feature = "channel-telegram")]
    {
        let context = load_telegram_command_context(config_path, account_id)?;
        run_telegram_channel_with_context(context, once, ChannelServeStopHandle::new(), true).await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_telegram_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-telegram") {
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(not(feature = "channel-telegram"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(feature = "channel-telegram")]
    {
        let context = load_telegram_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                family: TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
            },
            |context| {
                Box::pin(async move {
                    let token = context.resolved.bot_token().ok_or_else(|| {
                        "telegram bot token missing (set telegram.bot_token or env)".to_owned()
                    })?;
                    telegram::run_telegram_send(
                        &context.resolved,
                        token,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "telegram message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_feishu_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    request: &FeishuChannelSendRequest,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, account_id, request);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let context = load_feishu_command_context(config_path, account_id)?;
        let request = request.clone();
        let success_receive_id_type = request
            .receive_id_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
            },
            |context| {
                Box::pin(async move { feishu::run_feishu_send(&context.resolved, &request).await })
            },
            |context| {
                format!(
                    "feishu message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, receive_id_type={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    success_receive_id_type
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(context.resolved.receive_id_type.as_str())
                )
            },
        )
        .await
    }
}

pub async fn run_feishu_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, account_id, bind_override, path_override);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let context = load_feishu_command_context(config_path, account_id)?;
        run_feishu_channel_with_context(
            context,
            bind_override,
            path_override,
            ChannelServeStopHandle::new(),
            true,
        )
        .await
    }
}

#[cfg(feature = "channel-feishu")]
async fn run_feishu_channel_with_context(
    context: ChannelCommandContext<ResolvedFeishuChannelConfig>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let bind_override = bind_override.map(str::to_owned);
    let path_override = path_override.map(str::to_owned);
    run_channel_serve_command_with_stop(
        context,
        ChannelServeCommandSpec {
            family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_feishu_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();
                feishu::run_feishu_channel(
                    &config,
                    &resolved,
                    &resolved_path,
                    route.selected_by_default(),
                    route.default_account_source,
                    bind_override.as_deref(),
                    path_override.as_deref(),
                    kernel_ctx,
                    runtime,
                    stop,
                )
                .await
            })
        },
    )
    .await
}

#[cfg(feature = "channel-feishu")]
pub async fn run_feishu_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_feishu_command_context(resolved_path, config, account_id)?;
    run_feishu_channel_with_context(
        context,
        bind_override,
        path_override,
        stop,
        initialize_runtime_environment,
    )
    .await
}

#[doc(hidden)]
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
pub async fn run_channel_serve_runtime_probe_for_test(
    platform: ChannelPlatform,
    account_id: &str,
    account_label: &str,
    stop: ChannelServeStopHandle,
    entered: Arc<Notify>,
) -> CliResult<()> {
    with_channel_serve_runtime_with_stop(
        ChannelServeRuntimeSpec {
            platform,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id,
            account_label,
        },
        stop,
        move |_runtime, stop| async move {
            entered.notify_one();
            stop.wait().await;
            Ok(())
        },
    )
    .await
}

#[doc(hidden)]
pub fn load_channel_operation_runtime_for_account_from_dir_for_test(
    runtime_dir: &std::path::Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: &str,
    now_ms: u64,
) -> Option<ChannelOperationRuntime> {
    runtime_state::load_channel_operation_runtime_for_account_from_dir(
        runtime_dir,
        platform,
        operation_id,
        account_id,
        now_ms,
    )
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_matrix_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-matrix") {
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(not(feature = "channel-matrix"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(feature = "channel-matrix")]
    {
        let context = load_matrix_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                family: MATRIX_COMMAND_FAMILY_DESCRIPTOR,
            },
            |context| {
                Box::pin(async move {
                    let token = context.resolved.access_token().ok_or_else(|| {
                        "matrix access token missing (set matrix.access_token or env)".to_owned()
                    })?;
                    matrix::run_matrix_send(
                        &context.resolved,
                        token,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "matrix message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_matrix_channel(
    config_path: Option<&str>,
    once: bool,
    account_id: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-matrix") {
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(not(feature = "channel-matrix"))]
    {
        let _ = (config_path, once, account_id);
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(feature = "channel-matrix")]
    {
        let context = load_matrix_command_context(config_path, account_id)?;
        run_matrix_channel_with_context(context, once, ChannelServeStopHandle::new(), true).await
    }
}

#[cfg(feature = "channel-matrix")]
#[allow(clippy::print_stdout)]
async fn run_matrix_channel_with_context(
    context: ChannelCommandContext<ResolvedMatrixChannelConfig>,
    once: bool,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    run_channel_serve_command_with_stop(
        context,
        ChannelServeCommandSpec {
            family: MATRIX_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_matrix_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();
                let batch_kernel_ctx = Arc::new(crate::KernelContext {
                    kernel: kernel_ctx.kernel.clone(),
                    token: kernel_ctx.token.clone(),
                });
                let token = resolved.access_token().ok_or_else(|| {
                    "matrix access token missing (set matrix.access_token or env)".to_owned()
                })?;
                let mut adapter = matrix::MatrixAdapter::new(&resolved, token);

                println!(
                    "{} channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, timeout={}s)",
                    adapter.name(),
                    resolved_path.display(),
                    resolved.configured_account_id,
                    resolved.account.label,
                    route.selected_by_default(),
                    route.default_account_source.as_str(),
                    resolved.sync_timeout_s
                );

                loop {
                    let batch = tokio::select! {
                        _ = stop.wait() => break,
                        batch = adapter.receive_batch() => batch?,
                    };
                    let had_messages = process_channel_batch(
                        &mut adapter,
                        batch,
                        Some(runtime.as_ref()),
                        |message| {
                            let config = config.clone();
                            let kernel_ctx = batch_kernel_ctx.clone();
                            let resolved_path = resolved_path.clone();
                            Box::pin(async move {
                                process_inbound_with_provider(
                                    &config,
                                    Some(resolved_path.as_path()),
                                    &message,
                                    Some(kernel_ctx.as_ref()),
                                )
                                .await
                            })
                        },
                    )
                    .await?;
                    if !had_messages && once {
                        break;
                    }
                    if once {
                        break;
                    }
                }
                Ok(())
            })
        },
    )
    .await
}

#[cfg(feature = "channel-matrix")]
pub async fn run_matrix_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    once: bool,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_matrix_command_context(resolved_path, config, account_id)?;
    run_matrix_channel_with_context(context, once, stop, initialize_runtime_environment).await
}

#[allow(clippy::print_stdout)]
pub async fn run_wecom_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-wecom") {
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(not(feature = "channel-wecom"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(feature = "channel-wecom")]
    {
        let context = load_wecom_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                family: WECOM_COMMAND_FAMILY_DESCRIPTOR,
            },
            |context| {
                Box::pin(async move {
                    wecom::run_wecom_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "wecom message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)]
pub async fn run_wecom_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-wecom") {
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(not(feature = "channel-wecom"))]
    {
        let _ = (config_path, account_id);
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(feature = "channel-wecom")]
    {
        let context = load_wecom_command_context(config_path, account_id)?;
        run_wecom_channel_with_context(context, ChannelServeStopHandle::new(), true).await
    }
}

#[cfg(feature = "channel-wecom")]
async fn run_wecom_channel_with_context(
    context: ChannelCommandContext<ResolvedWecomChannelConfig>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    run_channel_serve_command_with_stop(
        context,
        ChannelServeCommandSpec {
            family: WECOM_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_wecom_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();
                wecom::run_wecom_channel(
                    &config,
                    &resolved,
                    &resolved_path,
                    route.selected_by_default(),
                    route.default_account_source,
                    kernel_ctx,
                    runtime,
                    stop,
                )
                .await
            })
        },
    )
    .await
}

#[cfg(feature = "channel-wecom")]
pub async fn run_wecom_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_wecom_command_context(resolved_path, config, account_id)?;
    run_wecom_channel_with_context(context, stop, initialize_runtime_environment).await
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
pub(crate) async fn send_text_to_known_session(
    config: &LoongClawConfig,
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
                telegram::run_telegram_send(
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
                if !resolved
                    .allowed_chat_ids
                    .iter()
                    .any(|allowed| allowed.trim() == conversation_id)
                {
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
                feishu::run_feishu_send(&resolved, &request).await?;
                Ok(ChannelSendReceipt {
                    channel: "feishu",
                    target,
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
                matrix::run_matrix_send(
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
                wecom::send_wecom_text(&resolved, conversation_id.as_str(), chat_type, text)
                    .await?;
                Ok(ChannelSendReceipt {
                    channel: "wecom",
                    target: conversation_id,
                })
            }
        }
    }
}

#[cfg(not(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
)))]
pub(crate) async fn send_text_to_known_session(
    _config: &LoongClawConfig,
    session_id: &str,
    _text: &str,
) -> CliResult<ChannelSendReceipt> {
    Err(format!("sessions_send_channel_unsupported: `{session_id}`"))
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
pub(super) async fn process_inbound_with_provider(
    config: &LoongClawConfig,
    resolved_path: Option<&std::path::Path>,
    message: &ChannelInboundMessage,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    let turn_config = reload_channel_turn_config(config, resolved_path)?;
    let address = message.session.conversation_address();
    let acp_turn_hints = resolve_channel_acp_turn_hints(&turn_config, &message.session)?;
    let acp_options = AcpConversationTurnOptions::automatic()
        .with_additional_bootstrap_mcp_servers(&acp_turn_hints.bootstrap_mcp_servers)
        .with_working_directory(acp_turn_hints.working_directory.as_deref())
        .with_provenance(channel_message_acp_turn_provenance(message));
    let ingress = channel_message_ingress_context(message);
    ConversationTurnCoordinator::new()
        .handle_turn_with_address_and_acp_options_and_ingress(
            &turn_config,
            &address,
            &message.text,
            ProviderErrorMode::Propagate,
            &acp_options,
            crate::conversation::ConversationRuntimeBinding::from_optional_kernel_context(
                kernel_ctx,
            ),
            ingress.as_ref(),
        )
        .await
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn reload_channel_turn_config(
    config: &LoongClawConfig,
    resolved_path: Option<&std::path::Path>,
) -> CliResult<LoongClawConfig> {
    match resolved_path {
        Some(path) => config.reload_provider_runtime_state_from_path(path),
        None => Ok(config.clone()),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn resolve_channel_acp_turn_hints(
    config: &LoongClawConfig,
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
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn channel_message_acp_turn_provenance(message: &ChannelInboundMessage) -> AcpTurnProvenance<'_> {
    AcpTurnProvenance {
        trace_id: None,
        source_message_id: message.delivery.source_message_id.as_deref(),
        ack_cursor: message.delivery.ack_cursor.as_deref(),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn channel_message_ingress_context(
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

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
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

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
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

fn render_channel_route_notice(
    platform: ChannelPlatform,
    route: &ChannelResolvedAccountRoute,
) -> Option<String> {
    if !route.uses_implicit_fallback_default() {
        return None;
    }
    Some(format!(
        "{} omitted --account and routed to configured account `{}` via fallback default selection; set {}.default_account or pass --account to avoid routing surprises",
        platform.as_str(),
        route.selected_configured_account_id,
        platform.as_str()
    ))
}

#[cfg(feature = "channel-telegram")]
fn validate_telegram_security_config(config: &ResolvedTelegramChannelConfig) -> CliResult<()> {
    if config.allowed_chat_ids.is_empty() {
        return Err(
            "telegram.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "channel-feishu")]
fn validate_feishu_security_config(config: &ResolvedFeishuChannelConfig) -> CliResult<()> {
    let has_allowlist = config
        .allowed_chat_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "feishu.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }

    if config.mode != crate::config::FeishuChannelServeMode::Webhook {
        return Ok(());
    }

    let has_verification_token = config
        .verification_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_verification_token {
        return Err(
            "feishu.verification_token is missing; configure token or verification_token_env"
                .to_owned(),
        );
    }

    let has_encrypt_key = config
        .encrypt_key()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_encrypt_key {
        return Err("feishu.encrypt_key is missing; configure key or encrypt_key_env".to_owned());
    }

    Ok(())
}

#[cfg(feature = "channel-matrix")]
fn validate_matrix_security_config(config: &ResolvedMatrixChannelConfig) -> CliResult<()> {
    let has_allowlist = config
        .allowed_room_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "matrix.allowed_room_ids is empty; configure at least one trusted room id".to_owned(),
        );
    }

    let base_url = config.resolved_base_url().unwrap_or_default();
    matrix::build_matrix_client_url(base_url.as_str())?;

    let has_access_token = config
        .access_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_access_token {
        return Err(
            "matrix.access_token is missing; configure access_token or access_token_env".to_owned(),
        );
    }

    let has_user_id = config
        .user_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if config.ignore_self_messages && !has_user_id {
        return Err(
            "matrix.user_id is missing; configure user_id when ignore_self_messages is enabled"
                .to_owned(),
        );
    }

    Ok(())
}

#[cfg(feature = "channel-wecom")]
fn validate_wecom_security_config(config: &ResolvedWecomChannelConfig) -> CliResult<()> {
    let has_allowlist = config
        .allowed_conversation_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "wecom.allowed_conversation_ids is empty; configure at least one trusted conversation id"
                .to_owned(),
        );
    }

    let websocket_url = config.resolved_websocket_url();
    let parsed_url = reqwest::Url::parse(websocket_url.as_str())
        .map_err(|error| format!("invalid wecom.websocket_url: {error}"))?;
    let scheme = parsed_url.scheme();
    if scheme != "ws" && scheme != "wss" {
        return Err("wecom.websocket_url must use ws or wss".to_owned());
    }

    let has_bot_id = config
        .bot_id()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_bot_id {
        return Err("wecom.bot_id is missing; configure bot_id or bot_id_env".to_owned());
    }

    let has_secret = config
        .secret()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_secret {
        return Err("wecom.secret is missing; configure secret or secret_env".to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_runtime_dir(suffix: &str) -> PathBuf {
        let unique = format!(
            "loongclaw-channel-mod-{suffix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn now_ms_for_test() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[derive(Default)]
    struct RecordingAdapter {
        sent: Arc<Mutex<Vec<(ChannelOutboundTarget, ChannelOutboundMessage)>>>,
        acked: Arc<Mutex<Vec<Option<String>>>>,
        completed_batches: Arc<Mutex<usize>>,
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording"
        }

        async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
            Ok(Vec::new())
        }

        async fn send_message(
            &self,
            target: &ChannelOutboundTarget,
            message: &ChannelOutboundMessage,
        ) -> CliResult<()> {
            self.sent
                .lock()
                .expect("sent log")
                .push((target.clone(), message.clone()));
            Ok(())
        }

        async fn ack_inbound(&mut self, message: &ChannelInboundMessage) -> CliResult<()> {
            self.acked
                .lock()
                .expect("ack log")
                .push(message.delivery.ack_cursor.clone());
            Ok(())
        }

        async fn complete_batch(&mut self) -> CliResult<()> {
            *self
                .completed_batches
                .lock()
                .expect("completed batch count") += 1;
            Ok(())
        }
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn channel_adapter_default_ack_hooks_are_noop() {
        #[derive(Default)]
        struct NoopAdapter;

        #[async_trait]
        impl ChannelAdapter for NoopAdapter {
            fn name(&self) -> &str {
                "noop"
            }

            async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
                Ok(Vec::new())
            }

            async fn send_message(
                &self,
                _target: &ChannelOutboundTarget,
                _message: &ChannelOutboundMessage,
            ) -> CliResult<()> {
                Ok(())
            }
        }

        let mut adapter = NoopAdapter;
        let message = ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("2".to_owned()),
                source_message_id: Some("42".to_owned()),
                sender_principal_key: None,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
            },
        };

        adapter
            .ack_inbound(&message)
            .await
            .expect("default ack hook should succeed");
        adapter
            .complete_batch()
            .await
            .expect("default batch completion hook should succeed");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn process_channel_batch_acknowledges_after_successful_delivery() {
        let mut adapter = RecordingAdapter::default();
        let batch = vec![ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("101".to_owned()),
                source_message_id: Some("55".to_owned()),
                sender_principal_key: None,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
            },
        }];

        let had_messages = process_channel_batch(
            &mut adapter,
            batch,
            None,
            |message: ChannelInboundMessage| {
                Box::pin(async move { Ok(format!("reply: {}", message.text)) })
            },
        )
        .await
        .expect("batch should process");

        assert!(had_messages);
        assert_eq!(
            adapter.sent.lock().expect("sent log").as_slice(),
            &[(
                ChannelOutboundTarget::telegram_chat(1),
                ChannelOutboundMessage::Text("reply: hello".to_owned()),
            )]
        );
        assert_eq!(
            adapter.acked.lock().expect("ack log").as_slice(),
            &[Some("101".to_owned())]
        );
        assert_eq!(*adapter.completed_batches.lock().expect("completed"), 1);
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn channel_adapter_send_text_defaults_to_text_outbound_message() {
        let adapter = RecordingAdapter::default();
        let target = ChannelOutboundTarget::telegram_chat(1);

        adapter
            .send_text(&target, "hello default wrapper")
            .await
            .expect("default send_text wrapper should succeed");

        assert_eq!(
            adapter.sent.lock().expect("sent log").as_slice(),
            &[(
                ChannelOutboundTarget::telegram_chat(1),
                ChannelOutboundMessage::Text("hello default wrapper".to_owned()),
            )]
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_outbound_target_tracks_delivery_options() {
        let target = ChannelOutboundTarget::feishu_receive_id("ou_demo")
            .with_feishu_receive_id_type("open_id")
            .with_idempotency_key("send-uuid-1")
            .with_feishu_reply_in_thread(true);

        assert_eq!(target.feishu_receive_id_type(), Some("open_id"));
        assert_eq!(target.idempotency_key(), Some("send-uuid-1"));
        assert_eq!(target.feishu_reply_in_thread(), Some(true));
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn reload_channel_turn_config_refreshes_provider_state_without_mutating_channel_settings() {
        let path = std::env::temp_dir().join(format!(
            "loongclaw-channel-provider-reload-{}.toml",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let path_string = path.display().to_string();

        let mut in_memory = LoongClawConfig::default();
        in_memory.telegram.enabled = true;
        in_memory.telegram.allowed_chat_ids = vec![1001];
        let mut openai =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai.model = "gpt-5".to_owned();
        in_memory.set_active_provider_profile(
            "openai-gpt-5",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai,
            },
        );

        let mut on_disk = in_memory.clone();
        on_disk.telegram.allowed_chat_ids = vec![2002];
        let mut deepseek =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
        deepseek.model = "deepseek-chat".to_owned();
        on_disk.providers.insert(
            "deepseek-chat".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: deepseek.clone(),
            },
        );
        on_disk.provider = deepseek;
        on_disk.active_provider = Some("deepseek-chat".to_owned());
        crate::config::write(Some(&path_string), &on_disk, true).expect("write config fixture");

        let reloaded =
            reload_channel_turn_config(&in_memory, Some(path.as_path())).expect("reload");
        assert_eq!(reloaded.active_provider_id(), Some("deepseek-chat"));
        assert_eq!(reloaded.provider.model, "deepseek-chat");
        assert_eq!(reloaded.telegram.allowed_chat_ids, vec![1001]);

        let _ = std::fs::remove_file(path);
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_is_stable() {
        let session = ChannelSession::new(ChannelPlatform::Telegram, "123");
        assert_eq!(session.session_key(), "telegram:123");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_thread_id_when_present() {
        let session = ChannelSession::with_thread(ChannelPlatform::Feishu, "oc_123", "om_thread_1");
        assert_eq!(session.session_key(), "feishu:oc_123:om_thread_1");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_account_identity_when_present() {
        let session = ChannelSession::with_account(ChannelPlatform::Telegram, "bot_123456", "123");
        assert_eq!(session.session_key(), "telegram:bot_123456:123");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_configured_account_identity_when_present() {
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "feishu_shared", "oc_123")
                .with_configured_account_id("work");
        assert_eq!(
            session.session_key(),
            "feishu:cfg=work:feishu_shared:oc_123"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_participant_id_when_present() {
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "lark_cli_a1b2c3", "oc_123")
                .with_participant_id("ou_sender_1");
        assert_eq!(
            session.session_key(),
            "feishu:lark_cli_a1b2c3:oc_123:ou_sender_1"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_account_identity_and_thread_when_present() {
        let session = ChannelSession::with_account_and_thread(
            ChannelPlatform::Feishu,
            "lark_cli_a1b2c3",
            "oc_123",
            "om_thread_1",
        );
        assert_eq!(
            session.session_key(),
            "feishu:lark_cli_a1b2c3:oc_123:om_thread_1"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_account_participant_and_thread_when_present() {
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "lark_cli_a1b2c3", "oc_123")
                .with_participant_id("ou_sender_1")
                .with_thread_id("om_root_1");
        assert_eq!(
            session.session_key(),
            "feishu:lark_cli_a1b2c3:oc_123:ou_sender_1:om_root_1"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_message_ingress_context_preserves_sender_and_thread_metadata() {
        let message = ChannelInboundMessage {
            session: ChannelSession::with_account(
                ChannelPlatform::Feishu,
                "lark_cli_a1b2c3",
                "oc_123",
            )
            .with_configured_account_id("work")
            .with_participant_id("ou_sender_1")
            .with_thread_id("om_root_1"),
            reply_target: ChannelOutboundTarget::feishu_message_reply("om_message_9"),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: None,
                source_message_id: Some("om_message_9".to_owned()),
                sender_principal_key: Some("feishu:user:ou_sender_1".to_owned()),
                thread_root_id: Some("om_root_1".to_owned()),
                parent_message_id: Some("om_parent_1".to_owned()),
                resources: vec![
                    ChannelDeliveryResource {
                        resource_type: "image".to_owned(),
                        file_key: "img_v2_123".to_owned(),
                        file_name: None,
                    },
                    ChannelDeliveryResource {
                        resource_type: "file".to_owned(),
                        file_key: "file_v2_456".to_owned(),
                        file_name: Some("report.pdf".to_owned()),
                    },
                ],
                feishu_callback: None,
            },
        };

        let ingress = channel_message_ingress_context(&message).expect("ingress context");
        assert_eq!(ingress.channel.platform, "feishu");
        assert_eq!(
            ingress.channel.account_id.as_deref(),
            Some("lark_cli_a1b2c3")
        );
        assert_eq!(
            ingress.channel.configured_account_id.as_deref(),
            Some("work")
        );
        assert_eq!(ingress.channel.conversation_id, "oc_123");
        assert_eq!(
            ingress.channel.participant_id.as_deref(),
            Some("ou_sender_1")
        );
        assert_eq!(ingress.channel.thread_id.as_deref(), Some("om_root_1"));
        assert_eq!(
            ingress.delivery.sender_identity_key.as_deref(),
            Some("feishu:user:ou_sender_1")
        );
        assert_eq!(
            ingress.delivery.thread_root_id.as_deref(),
            Some("om_root_1")
        );
        assert_eq!(
            ingress.delivery.parent_message_id.as_deref(),
            Some("om_parent_1")
        );
        assert_eq!(ingress.delivery.resources.len(), 2);
        assert_eq!(ingress.delivery.resources[0].resource_type, "image");
        assert_eq!(ingress.delivery.resources[0].file_key, "img_v2_123");
        assert_eq!(ingress.delivery.resources[1].resource_type, "file");
        assert_eq!(ingress.delivery.resources[1].file_key, "file_v2_456");
        assert_eq!(
            ingress.delivery.resources[1].file_name.as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            ingress.private,
            ConversationIngressPrivateContext::default()
        );
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_message_ingress_context_preserves_private_feishu_callback_metadata() {
        let message = ChannelInboundMessage {
            session: ChannelSession::with_account(
                ChannelPlatform::Feishu,
                "feishu_main",
                "oc_demo",
            )
            .with_configured_account_id("work")
            .with_participant_id("ou_operator_1")
            .with_thread_id("om_callback_1"),
            reply_target: ChannelOutboundTarget::feishu_message_reply("om_callback_1"),
            text: "[feishu_card_callback]".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: None,
                source_message_id: Some("om_callback_1".to_owned()),
                sender_principal_key: Some("feishu:user:ou_operator_1".to_owned()),
                thread_root_id: Some("om_callback_1".to_owned()),
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: Some(ChannelDeliveryFeishuCallback {
                    callback_token: Some("callback-secret-2".to_owned()),
                    open_message_id: Some("om_callback_1".to_owned()),
                    open_chat_id: Some("oc_demo".to_owned()),
                    operator_open_id: Some("ou_operator_1".to_owned()),
                    deferred_context_id: Some("evt_callback_2".to_owned()),
                }),
            },
        };

        let ingress = channel_message_ingress_context(&message).expect("ingress context");

        assert_eq!(
            ingress
                .private
                .feishu_callback
                .as_ref()
                .and_then(|value| value.callback_token.as_deref()),
            Some("callback-secret-2")
        );
        assert_eq!(
            ingress
                .private
                .feishu_callback
                .as_ref()
                .and_then(|value| value.operator_open_id.as_deref()),
            Some("ou_operator_1")
        );
        assert_eq!(
            ingress
                .private
                .feishu_callback
                .as_ref()
                .and_then(|value| value.deferred_context_id.as_deref()),
            Some("evt_callback_2")
        );
        assert!(
            !ingress
                .as_event_payload()
                .to_string()
                .contains("callback-secret-2")
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_outbound_target_preserves_platform_kind_and_id() {
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::MessageReply,
            "om_123",
        );
        assert_eq!(target.platform, ChannelPlatform::Feishu);
        assert_eq!(target.kind, ChannelOutboundTargetKind::MessageReply);
        assert_eq!(target.id, "om_123");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn render_channel_route_notice_warns_on_implicit_multi_account_fallback() {
        let route = crate::config::ChannelResolvedAccountRoute {
            requested_account_id: None,
            configured_account_count: 2,
            selected_configured_account_id: "alerts".to_owned(),
            default_account_source: crate::config::ChannelDefaultAccountSelectionSource::Fallback,
        };

        let rendered = render_channel_route_notice(ChannelPlatform::Telegram, &route)
            .expect("fallback route should warn");

        assert!(rendered.contains("telegram"));
        assert!(rendered.contains("alerts"));
        assert!(rendered.contains("--account"));
        assert!(rendered.contains("telegram.default_account"));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn render_channel_route_notice_is_silent_for_explicit_account_selection() {
        let route = crate::config::ChannelResolvedAccountRoute {
            requested_account_id: Some("work".to_owned()),
            configured_account_count: 2,
            selected_configured_account_id: "work".to_owned(),
            default_account_source: crate::config::ChannelDefaultAccountSelectionSource::Fallback,
        };

        assert!(render_channel_route_notice(ChannelPlatform::Telegram, &route).is_none());
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_command_context_preserves_route_metadata() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "bot_token": "123456:work-token",
                        "allowed_chat_ids": [1001]
                    },
                    "Alerts": {
                        "bot_token": "654321:alerts-token",
                        "allowed_chat_ids": [2002]
                    }
                }
            }
        }))
        .expect("deserialize telegram context config");

        let context =
            build_telegram_command_context(PathBuf::from("/tmp/loongclaw.toml"), config, None)
                .expect("build telegram command context");

        assert_eq!(context.resolved_path, PathBuf::from("/tmp/loongclaw.toml"));
        assert_eq!(context.resolved.configured_account_id, "alerts");
        assert!(context.route.selected_by_default());
        assert!(context.route.uses_implicit_fallback_default());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_command_context_rejects_disabled_resolved_account() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "enabled": true,
                "accounts": {
                    "Primary": {
                        "enabled": false,
                        "app_id": "cli_primary",
                        "app_secret": "secret"
                    }
                }
            }
        }))
        .expect("deserialize feishu context config");

        let error = build_feishu_command_context(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("Primary"),
        )
        .expect_err("disabled feishu account should fail");

        assert!(error.contains("disabled"));
        assert!(error.contains("primary"));
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_command_context_accepts_unique_runtime_account_alias() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "account_id": "feishu_shared",
                        "app_id": "cli_work",
                        "app_secret": "secret"
                    }
                }
            }
        }))
        .expect("deserialize feishu context config");

        let context = build_feishu_command_context(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("feishu_shared"),
        )
        .expect("unique runtime account alias should resolve");

        assert_eq!(context.resolved.configured_account_id, "work");
        assert_eq!(context.resolved.account.id, "feishu_shared");
        assert_eq!(
            context.route.requested_account_id.as_deref(),
            Some("feishu_shared")
        );
        assert!(!context.route.selected_by_default());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_command_context_reports_ambiguous_runtime_account_alias() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "account_id": "feishu_shared",
                        "app_id": "cli_work",
                        "app_secret": "secret-work"
                    },
                    "Alerts": {
                        "account_id": "feishu_shared",
                        "app_id": "cli_alerts",
                        "app_secret": "secret-alerts"
                    }
                }
            }
        }))
        .expect("deserialize feishu context config");

        let error = build_feishu_command_context(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("feishu_shared"),
        )
        .expect_err("ambiguous runtime account alias should fail");

        let error = error.to_ascii_lowercase();
        assert!(error.contains("requested feishu runtime account `feishu_shared` is ambiguous"));
        assert!(error.contains("work"));
        assert!(error.contains("alerts"));
        assert!(error.contains("use configured_account_id `alerts` or `work` to disambiguate"));
        assert!(error.contains("--account"));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_tracks_running_state_and_shutdown() {
        let runtime_dir = temp_runtime_dir("serve-runtime-wrapper");
        let runtime_dir_for_body = runtime_dir.clone();
        let operation = ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Telegram,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: "bot_123456",
            account_label: "bot:123456",
        };

        let result = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            operation,
            |runtime| async move {
                let live = runtime_state::load_channel_operation_runtime_for_account_from_dir(
                    runtime_dir_for_body.as_path(),
                    ChannelPlatform::Telegram,
                    "serve",
                    "bot_123456",
                    0,
                )
                .expect("runtime should exist while serve body is running");
                assert!(live.running);
                assert_eq!(live.pid, Some(9191));
                assert_eq!(live.account_id.as_deref(), Some("bot_123456"));
                assert_eq!(live.account_label.as_deref(), Some("bot:123456"));
                assert!(
                    Arc::strong_count(&runtime) >= 1,
                    "runtime handle should stay alive in serve body"
                );
                Ok::<_, String>("ok".to_owned())
            },
        )
        .await
        .expect("serve runtime wrapper should succeed");

        assert_eq!(result, "ok");

        let finished = runtime_state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            0,
        )
        .expect("runtime should remain readable after shutdown");
        assert!(!finished.running);
        assert_eq!(finished.pid, Some(9191));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_shuts_down_cleanly_after_cooperative_stop() {
        let runtime_dir = temp_runtime_dir("serve-runtime-cooperative-stop");
        let runtime_dir_for_wrapper = runtime_dir.clone();
        let runtime_dir_for_body = runtime_dir.clone();
        let entered = Arc::new(tokio::sync::Notify::new());
        let entered_for_body = entered.clone();
        let stop = ChannelServeStopHandle::new();
        let stop_for_body = stop.clone();
        let operation = ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Telegram,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: "bot_123456",
            account_label: "bot:123456",
        };

        let wrapper = tokio::spawn(async move {
            with_channel_serve_runtime_with_stop_in_dir(
                runtime_dir_for_wrapper.as_path(),
                9191,
                operation,
                stop_for_body,
                move |_runtime, stop| {
                    let runtime_dir_for_body = runtime_dir_for_body.clone();
                    async move {
                        let live =
                            runtime_state::load_channel_operation_runtime_for_account_from_dir(
                                runtime_dir_for_body.as_path(),
                                ChannelPlatform::Telegram,
                                "serve",
                                "bot_123456",
                                0,
                            )
                            .expect("runtime should exist while serve body is running");
                        assert!(live.running);
                        assert_eq!(live.pid, Some(9191));
                        entered_for_body.notify_one();
                        stop.wait().await;
                        Ok(())
                    }
                },
            )
            .await
        });

        entered.notified().await;
        stop.request_stop();

        wrapper
            .await
            .expect("cooperative stop wrapper join should succeed")
            .expect("cooperative stop wrapper should succeed");

        let finished = runtime_state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            0,
        )
        .expect("runtime should remain readable after shutdown");
        assert!(!finished.running);
        assert_eq!(finished.pid, Some(9191));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_rejects_duplicate_running_instance() {
        let runtime_dir = temp_runtime_dir("serve-runtime-duplicate");
        let now = now_ms_for_test();
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            7001,
            true,
            true,
            1,
            Some(now),
            Some(now),
            Some(7001),
        )
        .expect("seed running runtime state");

        let error = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect_err("duplicate running instance should be rejected");

        assert!(error.contains("already has an active serve runtime"));
        assert!(error.contains("bot_123456"));
        assert!(error.contains("7001"));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_allows_takeover_when_previous_instance_is_stale() {
        let runtime_dir = temp_runtime_dir("serve-runtime-stale-takeover");
        let now = now_ms_for_test();
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            7001,
            true,
            true,
            1,
            Some(now.saturating_sub(60_000)),
            Some(now.saturating_sub(60_000)),
            Some(7001),
        )
        .expect("seed stale runtime state");

        let result = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect("stale runtime should not block startup");

        assert_eq!(result, "ok");

        let runtime = runtime_state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            now_ms_for_test(),
        )
        .expect("runtime should remain readable after takeover");
        assert_eq!(runtime.pid, Some(9191));
        assert_eq!(runtime.instance_count, 1);
        assert_eq!(runtime.running_instances, 0);
        assert_eq!(runtime.stale_instances, 0);
        let entries = std::fs::read_dir(runtime_dir.as_path())
            .expect("list runtime dir after stale takeover")
            .map(|entry| {
                entry
                    .expect("runtime entry after stale takeover")
                    .file_name()
                    .into_string()
                    .expect("utf-8 runtime file name after stale takeover")
            })
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_rejects_active_legacy_owner_after_inactive_account_prune() {
        let runtime_dir = temp_runtime_dir("serve-runtime-legacy-owner");
        let now = now_ms_for_test();
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            7001,
            false,
            false,
            0,
            Some(now.saturating_sub(5_000)),
            Some(now.saturating_sub(5_000)),
            Some(7001),
        )
        .expect("seed inactive account-scoped runtime state");
        runtime_state::write_runtime_state_for_test_with_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            8118,
            true,
            true,
            1,
            Some(now),
            Some(now),
            Some(8118),
        )
        .expect("seed active legacy runtime state");

        let error = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect_err("active legacy owner should still block startup");

        assert!(error.contains("already has an active serve runtime"));
        assert!(error.contains("8118"));
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_requires_allowlist() {
        let config = LoongClawConfig::default();
        let resolved = config
            .telegram
            .resolve_account(None)
            .expect("resolve telegram account");
        let error = validate_telegram_security_config(&resolved)
            .expect_err("empty allowlist must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_accepts_configured_allowlist() {
        let mut config = LoongClawConfig::default();
        config.telegram.allowed_chat_ids = vec![123_i64];
        let resolved = config
            .telegram
            .resolve_account(None)
            .expect("resolve telegram account");
        assert!(validate_telegram_security_config(&resolved).is_ok());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_requires_secrets_and_allowlist() {
        let config = LoongClawConfig::default();
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let error =
            validate_feishu_security_config(&resolved).expect_err("empty config must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_accepts_complete_configuration() {
        let mut config = LoongClawConfig::default();
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some(loongclaw_contracts::SecretRef::Inline(
            "token-123".to_owned(),
        ));
        config.feishu.verification_token_env = None;
        config.feishu.encrypt_key = Some(loongclaw_contracts::SecretRef::Inline(
            "encrypt-key-123".to_owned(),
        ));
        config.feishu.encrypt_key_env = None;

        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        assert!(validate_feishu_security_config(&resolved).is_ok());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_accepts_websocket_mode_without_webhook_secrets() {
        let mut config = LoongClawConfig::default();
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.mode = Some(crate::config::FeishuChannelServeMode::Websocket);

        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        assert!(validate_feishu_security_config(&resolved).is_ok());
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn channel_session_key_encodes_matrix_segments_with_colons() {
        let session = ChannelSession::with_account(
            ChannelPlatform::Matrix,
            "@ops-bot:example.org",
            "!ops:example.org",
        )
        .with_participant_id("@alice:example.org")
        .with_thread_id("$event:example.org");

        assert_eq!(
            session.session_key(),
            "matrix:~b64~QG9wcy1ib3Q6ZXhhbXBsZS5vcmc:~b64~IW9wczpleGFtcGxlLm9yZw:~b64~QGFsaWNlOmV4YW1wbGUub3Jn:~b64~JGV2ZW50OmV4YW1wbGUub3Jn"
        );
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn parse_known_channel_session_send_target_decodes_matrix_route_segments() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "matrix": {
                "enabled": true,
                "accounts": {
                    "Ops": {
                        "account_id": "@ops-bot:example.org",
                        "access_token": "matrix-token",
                        "base_url": "https://matrix.example.org",
                        "allowed_room_ids": ["!ops:example.org"]
                    }
                }
            }
        }))
        .expect("deserialize matrix config");
        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve default matrix account");
        let account_id = resolved.account.id;
        let session_id = ChannelSession::with_account(
            ChannelPlatform::Matrix,
            account_id.as_str(),
            "!ops:example.org",
        )
        .session_key();

        let parsed = parse_known_channel_session_send_target(&config, session_id.as_str())
            .expect("parse matrix session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Matrix {
                account_id: Some(account_id),
                room_id: "!ops:example.org".to_owned(),
            }
        );
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn parse_known_channel_session_send_target_accepts_legacy_matrix_account_aliases() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "matrix": {
                "enabled": true,
                "accounts": {
                    "Ops": {
                        "account_id": "@ops-bot:example.org",
                        "access_token": "matrix-token",
                        "base_url": "https://matrix.example.org",
                        "allowed_room_ids": ["!ops:example.org"]
                    }
                }
            }
        }))
        .expect("deserialize matrix config");

        let parsed = parse_known_channel_session_send_target(
            &config,
            "matrix:~b64~QG9wcy1ib3Q6ZXhhbXBsZS5vcmc:~b64~IW9wczpleGFtcGxlLm9yZw",
        )
        .expect("parse matrix session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Matrix {
                account_id: Some("ops-bot-example-org".to_owned()),
                room_id: "!ops:example.org".to_owned(),
            }
        );
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn parse_known_channel_session_send_target_matches_normalized_runtime_account_identity() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "ops": {
                        "account_id": "Ops-Bot",
                        "bot_token": "123456:telegram-test-token",
                        "allowed_chat_ids": [123]
                    }
                }
            }
        }))
        .expect("deserialize telegram config");

        let parsed = parse_known_channel_session_send_target(&config, "telegram:Ops-Bot:123")
            .expect("parse telegram session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Telegram {
                account_id: Some("ops-bot".to_owned()),
                chat_id: "123".to_owned(),
                thread_id: None,
            }
        );
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn parse_known_channel_session_send_target_treats_single_segment_telegram_scope_as_chat_id() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "bot_token": "123456:telegram-test-token",
                "allowed_chat_ids": [123]
            }
        }))
        .expect("deserialize telegram config");

        let parsed = parse_known_channel_session_send_target(&config, "telegram:123")
            .expect("parse telegram session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Telegram {
                account_id: None,
                chat_id: "123".to_owned(),
                thread_id: None,
            }
        );
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn parse_known_channel_session_send_target_honors_configured_account_marker() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "default_account": "work",
                "accounts": {
                    "work": {
                        "bot_token": "123456:work-token",
                        "allowed_chat_ids": [123]
                    },
                    "alerts": {
                        "bot_token": "654321:alerts-token",
                        "allowed_chat_ids": [456]
                    }
                }
            }
        }))
        .expect("deserialize telegram multi-account config");

        let parsed =
            parse_known_channel_session_send_target(&config, "telegram:cfg=alerts:bot_654321:456")
                .expect("parse telegram session with configured marker");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Telegram {
                account_id: Some("alerts".to_owned()),
                chat_id: "456".to_owned(),
                thread_id: None,
            }
        );
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn matrix_security_validation_requires_room_allowlist_and_transport() {
        let config = LoongClawConfig::default();
        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account");
        let error =
            validate_matrix_security_config(&resolved).expect_err("empty config must be rejected");
        assert!(error.contains("allowed_room_ids"));

        let mut config = LoongClawConfig::default();
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account with allowlist");
        let error = validate_matrix_security_config(&resolved)
            .expect_err("base url and token are still required");
        assert!(error.contains("base_url"));
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn matrix_security_validation_rejects_invalid_base_url() {
        let mut config = LoongClawConfig::default();
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        config.matrix.user_id = Some("@ops-bot:example.org".to_owned());
        config.matrix.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "matrix-token".to_owned(),
        ));
        config.matrix.base_url = Some("not a url".to_owned());

        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account with invalid base url");
        let error = validate_matrix_security_config(&resolved)
            .expect_err("invalid base url must be rejected");
        assert!(error.contains("invalid matrix base_url"));
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn matrix_security_validation_requires_user_id_when_ignoring_self_messages() {
        let mut config = LoongClawConfig::default();
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        config.matrix.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "matrix-token".to_owned(),
        ));
        config.matrix.base_url = Some("https://matrix.example.org".to_owned());
        config.matrix.ignore_self_messages = true;

        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account with self-filter enabled");
        let error = validate_matrix_security_config(&resolved)
            .expect_err("user_id is required when self-filtering is enabled");
        assert!(error.contains("matrix.user_id"));
    }

    #[test]
    fn channel_streaming_mode_default_is_off() {
        assert_eq!(ChannelStreamingMode::default(), ChannelStreamingMode::Off);
    }
}
