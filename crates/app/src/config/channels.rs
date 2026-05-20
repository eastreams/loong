use std::collections::BTreeMap;

use loong_contracts::SecretRef;
use serde::{Deserialize, Serialize};

use crate::CliResult;

use super::irc::{
    IRC_NICKNAME_ENV, IRC_PASSWORD_ENV, IRC_SERVER_ENV, default_irc_nickname_env,
    default_irc_password_env, default_irc_server_env, validate_irc_env_pointer,
    validate_irc_nickname_field, validate_irc_secret_ref_env_pointer, validate_irc_server_field,
};
use super::shared::{
    ConfigValidationCode, ConfigValidationIssue, ConfigValidationSeverity,
    EnvPointerValidationHint, validate_env_pointer_field, validate_secret_ref_env_pointer_field,
};
use crate::secrets::resolve_secret_with_legacy_env;

#[path = "channels_bridge.rs"]
pub(crate) mod bridge;
#[path = "channels_irc_impl.rs"]
mod irc_impl;
#[path = "channels_nostr_impl.rs"]
mod nostr_impl;
#[path = "channels_signal_impl.rs"]
mod signal_impl;
mod twitch;

#[allow(unused_imports)]
pub use self::twitch::{ResolvedTwitchChannelConfig, TwitchAccountConfig, TwitchChannelConfig};
#[path = "channels_account_resolution.rs"]
mod account_resolution;
#[path = "channels_defaults.rs"]
mod defaults;
#[path = "channels_email_support.rs"]
mod email_support;
#[path = "channels_shared_types.rs"]
mod shared_types;
mod tlon_support;

use self::defaults::*;
pub(crate) use self::email_support::parse_email_smtp_endpoint;
use self::email_support::*;
use account_resolution::*;
mod bridge_messaging_impl;
mod collab_impl;
mod comms_impl;
#[path = "channels_validation.rs"]
mod validation_support;

use self::validation_support::*;
pub use account_resolution::normalize_channel_account_id;
#[allow(unused_imports)]
pub use bridge::{
    OnebotAccountConfig, OnebotChannelConfig, ResolvedOnebotChannelConfig,
    ResolvedWeixinChannelConfig, ResolvedWhatsappPersonalChannelConfig, WeixinAccountConfig,
    WeixinChannelConfig, WhatsappPersonalAccountConfig, WhatsappPersonalChannelConfig,
};
pub use nostr_impl::{NostrAccountConfig, NostrChannelConfig, ResolvedNostrChannelConfig};
pub(crate) use nostr_impl::{parse_nostr_private_key_hex, parse_nostr_public_key_hex};
pub use shared_types::{
    ChannelAccountIdentity, ChannelAccountIdentitySource, ChannelAcpConfig,
    ChannelDefaultAccountSelection, ChannelDefaultAccountSelectionSource,
    ChannelResolvedAccountRoute, CliChannelConfig, FeishuDomain, TelegramStreamingMode,
    WebhookPayloadFormat,
};
pub(crate) use shared_types::{
    DINGTALK_SECRET_ENV, DINGTALK_WEBHOOK_URL_ENV, DISCORD_APPLICATION_ID_ENV,
    DISCORD_BOT_TOKEN_ENV, EMAIL_IMAP_PASSWORD_ENV, EMAIL_IMAP_USERNAME_ENV,
    EMAIL_SMTP_PASSWORD_ENV, EMAIL_SMTP_USERNAME_ENV, EmailSmtpEndpoint, FEISHU_APP_ID_ENV,
    FEISHU_APP_SECRET_ENV, FEISHU_ENCRYPT_KEY_ENV, FEISHU_VERIFICATION_TOKEN_ENV,
    GOOGLE_CHAT_WEBHOOK_URL_ENV, IMESSAGE_BRIDGE_TOKEN_ENV, IMESSAGE_BRIDGE_URL_ENV,
    LINE_CHANNEL_ACCESS_TOKEN_ENV, LINE_CHANNEL_SECRET_ENV, MATRIX_ACCESS_TOKEN_ENV,
    MATTERMOST_BOT_TOKEN_ENV, MATTERMOST_SERVER_URL_ENV, NEXTCLOUD_TALK_SERVER_URL_ENV,
    NEXTCLOUD_TALK_SHARED_SECRET_ENV, NOSTR_PRIVATE_KEY_ENV, NOSTR_RELAY_URLS_ENV,
    ONEBOT_ACCESS_TOKEN_ENV, ONEBOT_WEBSOCKET_URL_ENV, QQBOT_APP_ID_ENV, QQBOT_CLIENT_SECRET_ENV,
    SIGNAL_ACCOUNT_ENV, SIGNAL_SERVICE_URL_ENV, SLACK_BOT_TOKEN_ENV,
    SYNOLOGY_CHAT_INCOMING_URL_ENV, SYNOLOGY_CHAT_TOKEN_ENV, TEAMS_APP_ID_ENV,
    TEAMS_APP_PASSWORD_ENV, TEAMS_TENANT_ID_ENV, TEAMS_WEBHOOK_URL_ENV, TELEGRAM_BOT_TOKEN_ENV,
    TLON_CODE_ENV, TLON_SHIP_ENV, TLON_URL_ENV, TWITCH_ACCESS_TOKEN_ENV, WEBHOOK_AUTH_TOKEN_ENV,
    WEBHOOK_ENDPOINT_URL_ENV, WEBHOOK_SIGNING_SECRET_ENV, WECOM_BOT_ID_ENV, WECOM_SECRET_ENV,
    WEIXIN_BRIDGE_ACCESS_TOKEN_ENV, WEIXIN_BRIDGE_URL_ENV, WHATSAPP_ACCESS_TOKEN_ENV,
    WHATSAPP_APP_SECRET_ENV, WHATSAPP_PERSONAL_AUTH_DIR_ENV, WHATSAPP_PERSONAL_BRIDGE_URL_ENV,
    WHATSAPP_PHONE_NUMBER_ID_ENV, WHATSAPP_VERIFY_TOKEN_ENV,
};
use signal_impl::{
    default_signal_account_env, default_signal_service_url, default_signal_service_url_env,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
    #[serde(default = "default_telegram_base_url")]
    pub base_url: String,
    #[serde(default = "default_telegram_timeout_seconds")]
    pub polling_timeout_s: u64,
    #[serde(default)]
    pub allowed_chat_ids: Vec<i64>,
    #[serde(default)]
    pub allowed_sender_ids: Vec<i64>,
    #[serde(default)]
    pub require_mention: bool,
    #[serde(default)]
    pub acp: ChannelAcpConfig,
    #[serde(default)]
    pub streaming_mode: TelegramStreamingMode,
    #[serde(default = "default_true")]
    pub ack_reactions: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, TelegramAccountConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub polling_timeout_s: Option<u64>,
    #[serde(default)]
    pub allowed_chat_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub allowed_sender_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub require_mention: Option<bool>,
    #[serde(default)]
    pub acp: Option<ChannelAcpConfig>,
    #[serde(default)]
    pub streaming_mode: Option<TelegramStreamingMode>,
    #[serde(default)]
    pub ack_reactions: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTelegramChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bot_token: Option<SecretRef>,
    pub bot_token_env: Option<String>,
    pub base_url: String,
    pub polling_timeout_s: u64,
    pub allowed_chat_ids: Vec<i64>,
    pub allowed_sender_ids: Vec<i64>,
    pub require_mention: bool,
    pub acp: ChannelAcpConfig,
    pub streaming_mode: TelegramStreamingMode,
    pub ack_reactions: bool,
}

impl ResolvedTelegramChannelConfig {
    pub fn bot_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bot_token.as_ref(), self.bot_token_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct QqbotAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub app_id: Option<SecretRef>,
    #[serde(default)]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub client_secret: Option<SecretRef>,
    #[serde(default)]
    pub client_secret_env: Option<String>,
    #[serde(default)]
    pub allowed_peer_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedQqbotChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub app_id: Option<SecretRef>,
    pub app_id_env: Option<String>,
    pub client_secret: Option<SecretRef>,
    pub client_secret_env: Option<String>,
    pub allowed_peer_ids: Vec<String>,
}

impl ResolvedQqbotChannelConfig {
    pub fn app_id(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.app_id.as_ref(), self.app_id_env.as_deref())
    }

    pub fn client_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.client_secret.as_ref(),
            self.client_secret_env.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeishuAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub app_id: Option<SecretRef>,
    #[serde(default)]
    pub app_secret: Option<SecretRef>,
    #[serde(default)]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub app_secret_env: Option<String>,
    #[serde(default)]
    pub domain: Option<FeishuDomain>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub mode: Option<FeishuChannelServeMode>,
    #[serde(default)]
    pub receive_id_type: Option<String>,
    #[serde(default)]
    pub webhook_bind: Option<String>,
    #[serde(default)]
    pub webhook_path: Option<String>,
    #[serde(default)]
    pub verification_token: Option<SecretRef>,
    #[serde(default)]
    pub verification_token_env: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<SecretRef>,
    #[serde(default)]
    pub encrypt_key_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_sender_ids: Option<Vec<String>>,
    #[serde(default)]
    pub ack_reactions: Option<bool>,
    #[serde(default)]
    pub ignore_bot_messages: Option<bool>,
    #[serde(default)]
    pub require_mention: Option<bool>,
    #[serde(default)]
    pub acp: Option<ChannelAcpConfig>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeishuChannelServeMode {
    #[default]
    Webhook,
    Websocket,
}

impl FeishuChannelServeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Websocket => "websocket",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFeishuChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub app_id: Option<SecretRef>,
    pub app_secret: Option<SecretRef>,
    pub app_id_env: Option<String>,
    pub app_secret_env: Option<String>,
    pub domain: FeishuDomain,
    pub base_url: Option<String>,
    pub mode: FeishuChannelServeMode,
    pub receive_id_type: String,
    pub webhook_bind: String,
    pub webhook_path: String,
    pub verification_token: Option<SecretRef>,
    pub verification_token_env: Option<String>,
    pub encrypt_key: Option<SecretRef>,
    pub encrypt_key_env: Option<String>,
    pub allowed_chat_ids: Vec<String>,
    pub allowed_sender_ids: Vec<String>,
    pub ack_reactions: bool,
    pub ignore_bot_messages: bool,
    pub require_mention: bool,
    pub acp: ChannelAcpConfig,
}

impl ResolvedFeishuChannelConfig {
    pub fn app_id(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.app_id.as_ref(), self.app_id_env.as_deref())
    }

    pub fn app_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.app_secret.as_ref(), self.app_secret_env.as_deref())
    }

    pub fn verification_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.verification_token.as_ref(),
            self.verification_token_env.as_deref(),
        )
    }

    pub fn encrypt_key(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.encrypt_key.as_ref(), self.encrypt_key_env.as_deref())
    }

    pub fn resolved_base_url(&self) -> String {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.domain.default_base_url().to_owned())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatrixAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default)]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub sync_timeout_s: Option<u64>,
    #[serde(default)]
    pub allowed_room_ids: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_sender_ids: Option<Vec<String>>,
    #[serde(default)]
    pub require_mention: Option<bool>,
    #[serde(default)]
    pub ignore_self_messages: Option<bool>,
    #[serde(default)]
    pub acp: Option<ChannelAcpConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMatrixChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub user_id: Option<String>,
    pub access_token: Option<SecretRef>,
    pub access_token_env: Option<String>,
    pub base_url: Option<String>,
    pub sync_timeout_s: u64,
    pub allowed_room_ids: Vec<String>,
    pub allowed_sender_ids: Vec<String>,
    pub require_mention: bool,
    pub ignore_self_messages: bool,
    pub acp: ChannelAcpConfig,
}

impl ResolvedMatrixChannelConfig {
    pub fn access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.access_token.as_ref(), self.access_token_env.as_deref())
    }

    pub fn resolved_base_url(&self) -> Option<String> {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WecomAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bot_id: Option<SecretRef>,
    #[serde(default)]
    pub secret: Option<SecretRef>,
    #[serde(default)]
    pub bot_id_env: Option<String>,
    #[serde(default)]
    pub secret_env: Option<String>,
    #[serde(default)]
    pub websocket_url: Option<String>,
    #[serde(default)]
    pub ping_interval_s: Option<u64>,
    #[serde(default)]
    pub reconnect_interval_s: Option<u64>,
    #[serde(default)]
    pub allowed_conversation_ids: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_sender_ids: Option<Vec<String>>,
    #[serde(default)]
    pub acp: Option<ChannelAcpConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWecomChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bot_id: Option<SecretRef>,
    pub secret: Option<SecretRef>,
    pub bot_id_env: Option<String>,
    pub secret_env: Option<String>,
    pub websocket_url: Option<String>,
    pub ping_interval_s: u64,
    pub reconnect_interval_s: u64,
    pub allowed_conversation_ids: Vec<String>,
    pub allowed_sender_ids: Vec<String>,
    pub acp: ChannelAcpConfig,
}

impl ResolvedWecomChannelConfig {
    pub fn bot_id(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bot_id.as_ref(), self.bot_id_env.as_deref())
    }

    pub fn secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.secret.as_ref(), self.secret_env.as_deref())
    }

    pub fn resolved_websocket_url(&self) -> String {
        self.websocket_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_wecom_websocket_url)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeishuChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub app_id: Option<SecretRef>,
    #[serde(default)]
    pub app_secret: Option<SecretRef>,
    #[serde(default)]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub app_secret_env: Option<String>,
    #[serde(default)]
    pub domain: FeishuDomain,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub mode: Option<FeishuChannelServeMode>,
    #[serde(default = "default_feishu_receive_id_type")]
    pub receive_id_type: String,
    #[serde(default = "default_feishu_webhook_bind")]
    pub webhook_bind: String,
    #[serde(default = "default_feishu_webhook_path")]
    pub webhook_path: String,
    #[serde(default)]
    pub verification_token: Option<SecretRef>,
    #[serde(default)]
    pub verification_token_env: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<SecretRef>,
    #[serde(default)]
    pub encrypt_key_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Vec<String>,
    #[serde(default)]
    pub allowed_sender_ids: Vec<String>,
    #[serde(default = "default_true")]
    pub ack_reactions: bool,
    #[serde(default = "default_true")]
    pub ignore_bot_messages: bool,
    #[serde(default)]
    pub require_mention: bool,
    #[serde(default)]
    pub acp: ChannelAcpConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, FeishuAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatrixChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default)]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_matrix_sync_timeout_seconds")]
    pub sync_timeout_s: u64,
    #[serde(default)]
    pub allowed_room_ids: Vec<String>,
    #[serde(default)]
    pub allowed_sender_ids: Vec<String>,
    #[serde(default)]
    pub require_mention: bool,
    #[serde(default = "default_true")]
    pub ignore_self_messages: bool,
    #[serde(default)]
    pub acp: ChannelAcpConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, MatrixAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WecomChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub bot_id: Option<SecretRef>,
    #[serde(default)]
    pub secret: Option<SecretRef>,
    #[serde(default)]
    pub bot_id_env: Option<String>,
    #[serde(default)]
    pub secret_env: Option<String>,
    #[serde(default)]
    pub websocket_url: Option<String>,
    #[serde(default = "default_wecom_ping_interval_seconds")]
    pub ping_interval_s: u64,
    #[serde(default = "default_wecom_reconnect_interval_seconds")]
    pub reconnect_interval_s: u64,
    #[serde(default)]
    pub allowed_conversation_ids: Vec<String>,
    #[serde(default)]
    pub allowed_sender_ids: Vec<String>,
    #[serde(default)]
    pub acp: ChannelAcpConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, WecomAccountConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub channel_access_token: Option<SecretRef>,
    #[serde(default)]
    pub channel_access_token_env: Option<String>,
    #[serde(default)]
    pub channel_secret: Option<SecretRef>,
    #[serde(default)]
    pub channel_secret_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLineChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub channel_access_token: Option<SecretRef>,
    pub channel_access_token_env: Option<String>,
    pub channel_secret: Option<SecretRef>,
    pub channel_secret_env: Option<String>,
    pub api_base_url: Option<String>,
}

impl ResolvedLineChannelConfig {
    pub fn channel_access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.channel_access_token.as_ref(),
            self.channel_access_token_env.as_deref(),
        )
    }

    pub fn channel_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.channel_secret.as_ref(),
            self.channel_secret_env.as_deref(),
        )
    }

    pub fn resolved_api_base_url(&self) -> String {
        self.api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_line_api_base_url)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DingtalkAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<SecretRef>,
    #[serde(default)]
    pub webhook_url_env: Option<String>,
    #[serde(default)]
    pub secret: Option<SecretRef>,
    #[serde(default)]
    pub secret_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDingtalkChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub webhook_url: Option<SecretRef>,
    pub webhook_url_env: Option<String>,
    pub secret: Option<SecretRef>,
    pub secret_env: Option<String>,
}

impl ResolvedDingtalkChannelConfig {
    pub fn webhook_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.webhook_url.as_ref(), self.webhook_url_env.as_deref())
    }

    pub fn secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.secret.as_ref(), self.secret_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub endpoint_url: Option<SecretRef>,
    #[serde(default)]
    pub endpoint_url_env: Option<String>,
    #[serde(default)]
    pub auth_token: Option<SecretRef>,
    #[serde(default)]
    pub auth_token_env: Option<String>,
    #[serde(default)]
    pub auth_header_name: Option<String>,
    #[serde(default)]
    pub auth_token_prefix: Option<String>,
    #[serde(default)]
    pub payload_format: Option<WebhookPayloadFormat>,
    #[serde(default)]
    pub payload_text_field: Option<String>,
    #[serde(default)]
    pub public_base_url: Option<String>,
    #[serde(default)]
    pub signing_secret: Option<SecretRef>,
    #[serde(default)]
    pub signing_secret_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWebhookChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub endpoint_url: Option<SecretRef>,
    pub endpoint_url_env: Option<String>,
    pub auth_token: Option<SecretRef>,
    pub auth_token_env: Option<String>,
    pub auth_header_name: String,
    pub auth_token_prefix: String,
    pub payload_format: WebhookPayloadFormat,
    pub payload_text_field: String,
    pub public_base_url: Option<String>,
    pub signing_secret: Option<SecretRef>,
    pub signing_secret_env: Option<String>,
}

impl ResolvedWebhookChannelConfig {
    pub fn endpoint_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.endpoint_url.as_ref(), self.endpoint_url_env.as_deref())
    }

    pub fn auth_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.auth_token.as_ref(), self.auth_token_env.as_deref())
    }

    pub fn signing_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.signing_secret.as_ref(),
            self.signing_secret_env.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub smtp_host: Option<String>,
    #[serde(default)]
    pub smtp_username: Option<SecretRef>,
    #[serde(default)]
    pub smtp_username_env: Option<String>,
    #[serde(default)]
    pub smtp_password: Option<SecretRef>,
    #[serde(default)]
    pub smtp_password_env: Option<String>,
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub imap_host: Option<String>,
    #[serde(default)]
    pub imap_username: Option<SecretRef>,
    #[serde(default)]
    pub imap_username_env: Option<String>,
    #[serde(default)]
    pub imap_password: Option<SecretRef>,
    #[serde(default)]
    pub imap_password_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEmailChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub smtp_host: Option<String>,
    pub smtp_username: Option<SecretRef>,
    pub smtp_username_env: Option<String>,
    pub smtp_password: Option<SecretRef>,
    pub smtp_password_env: Option<String>,
    pub from_address: Option<String>,
    pub imap_host: Option<String>,
    pub imap_username: Option<SecretRef>,
    pub imap_username_env: Option<String>,
    pub imap_password: Option<SecretRef>,
    pub imap_password_env: Option<String>,
}

impl ResolvedEmailChannelConfig {
    pub fn smtp_host(&self) -> Option<String> {
        self.smtp_host
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    pub fn smtp_username(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.smtp_username.as_ref(),
            self.smtp_username_env.as_deref(),
        )
    }

    pub fn smtp_password(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.smtp_password.as_ref(),
            self.smtp_password_env.as_deref(),
        )
    }

    pub fn from_address(&self) -> Option<String> {
        self.from_address
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    pub fn imap_host(&self) -> Option<String> {
        self.imap_host
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    pub fn imap_username(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.imap_username.as_ref(),
            self.imap_username_env.as_deref(),
        )
    }

    pub fn imap_password(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.imap_password.as_ref(),
            self.imap_password_env.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub application_id_env: Option<String>,
    #[serde(default)]
    pub allowed_guild_ids: Option<Vec<String>>,
    #[serde(default)]
    pub api_base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDiscordChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bot_token: Option<SecretRef>,
    pub bot_token_env: Option<String>,
    pub application_id: Option<String>,
    pub application_id_env: Option<String>,
    pub allowed_guild_ids: Vec<String>,
    pub api_base_url: Option<String>,
}

impl ResolvedDiscordChannelConfig {
    pub fn bot_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bot_token.as_ref(), self.bot_token_env.as_deref())
    }

    pub fn resolved_api_base_url(&self) -> String {
        self.api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_discord_api_base_url)
    }

    pub fn application_id(&self) -> Option<String> {
        resolve_string_with_legacy_env(
            self.application_id.as_deref(),
            self.application_id_env.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlackAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSlackChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bot_token: Option<SecretRef>,
    pub bot_token_env: Option<String>,
    pub api_base_url: Option<String>,
}

impl ResolvedSlackChannelConfig {
    pub fn bot_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bot_token.as_ref(), self.bot_token_env.as_deref())
    }

    pub fn resolved_api_base_url(&self) -> String {
        self.api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_slack_api_base_url)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoogleChatAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<SecretRef>,
    #[serde(default)]
    pub webhook_url_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGoogleChatChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub webhook_url: Option<SecretRef>,
    pub webhook_url_env: Option<String>,
}

impl ResolvedGoogleChatChannelConfig {
    pub fn webhook_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.webhook_url.as_ref(), self.webhook_url_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MattermostAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_url_env: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMattermostChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub server_url: Option<String>,
    pub server_url_env: Option<String>,
    pub bot_token: Option<SecretRef>,
    pub bot_token_env: Option<String>,
}

impl ResolvedMattermostChannelConfig {
    pub fn server_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.server_url.as_deref(), self.server_url_env.as_deref())
    }

    pub fn bot_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bot_token.as_ref(), self.bot_token_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NextcloudTalkAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_url_env: Option<String>,
    #[serde(default)]
    pub shared_secret: Option<SecretRef>,
    #[serde(default)]
    pub shared_secret_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNextcloudTalkChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub server_url: Option<String>,
    pub server_url_env: Option<String>,
    pub shared_secret: Option<SecretRef>,
    pub shared_secret_env: Option<String>,
}

impl ResolvedNextcloudTalkChannelConfig {
    pub fn server_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.server_url.as_deref(), self.server_url_env.as_deref())
    }

    pub fn shared_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.shared_secret.as_ref(),
            self.shared_secret_env.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SynologyChatAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub token: Option<SecretRef>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub incoming_url: Option<SecretRef>,
    #[serde(default)]
    pub incoming_url_env: Option<String>,
    #[serde(default)]
    pub allowed_user_ids: Option<Vec<u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSynologyChatChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub token: Option<SecretRef>,
    pub token_env: Option<String>,
    pub incoming_url: Option<SecretRef>,
    pub incoming_url_env: Option<String>,
    pub allowed_user_ids: Vec<u64>,
}

impl ResolvedSynologyChatChannelConfig {
    pub fn token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.token.as_ref(), self.token_env.as_deref())
    }

    pub fn incoming_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.incoming_url.as_ref(), self.incoming_url_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamsAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<SecretRef>,
    #[serde(default)]
    pub webhook_url_env: Option<String>,
    #[serde(default)]
    pub app_id: Option<SecretRef>,
    #[serde(default)]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub app_password: Option<SecretRef>,
    #[serde(default)]
    pub app_password_env: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub tenant_id_env: Option<String>,
    #[serde(default)]
    pub allowed_conversation_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTeamsChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub webhook_url: Option<SecretRef>,
    pub webhook_url_env: Option<String>,
    pub app_id: Option<SecretRef>,
    pub app_id_env: Option<String>,
    pub app_password: Option<SecretRef>,
    pub app_password_env: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_env: Option<String>,
    pub allowed_conversation_ids: Vec<String>,
}

impl ResolvedTeamsChannelConfig {
    pub fn webhook_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.webhook_url.as_ref(), self.webhook_url_env.as_deref())
    }

    pub fn app_id(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.app_id.as_ref(), self.app_id_env.as_deref())
    }

    pub fn app_password(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.app_password.as_ref(), self.app_password_env.as_deref())
    }

    pub fn tenant_id(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.tenant_id.as_deref(), self.tenant_id_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IrcAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub server_env: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub nickname_env: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub realname: Option<String>,
    #[serde(default)]
    pub password: Option<SecretRef>,
    #[serde(default)]
    pub password_env: Option<String>,
    #[serde(default)]
    pub channel_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIrcChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub server: Option<String>,
    pub server_env: Option<String>,
    pub nickname: Option<String>,
    pub nickname_env: Option<String>,
    pub username: Option<String>,
    pub realname: Option<String>,
    pub password: Option<SecretRef>,
    pub password_env: Option<String>,
    pub channel_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImessageAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bridge_url: Option<String>,
    #[serde(default)]
    pub bridge_url_env: Option<String>,
    #[serde(default)]
    pub bridge_token: Option<SecretRef>,
    #[serde(default)]
    pub bridge_token_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImessageChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bridge_url: Option<String>,
    pub bridge_url_env: Option<String>,
    pub bridge_token: Option<SecretRef>,
    pub bridge_token_env: Option<String>,
    pub allowed_chat_ids: Vec<String>,
}

impl ResolvedImessageChannelConfig {
    pub fn bridge_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.bridge_url.as_deref(), self.bridge_url_env.as_deref())
    }

    pub fn bridge_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bridge_token.as_ref(), self.bridge_token_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default, rename = "account")]
    pub signal_account: Option<String>,
    #[serde(default = "default_signal_account_env", rename = "account_env")]
    pub signal_account_env: Option<String>,
    #[serde(default)]
    pub service_url: Option<String>,
    #[serde(default = "default_signal_service_url_env")]
    pub service_url_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSignalChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub signal_account: Option<String>,
    pub signal_account_env: Option<String>,
    pub service_url: Option<String>,
    pub service_url_env: Option<String>,
}

impl ResolvedSignalChannelConfig {
    pub fn signal_account(&self) -> Option<String> {
        resolve_string_with_legacy_env(
            self.signal_account.as_deref(),
            self.signal_account_env.as_deref(),
        )
    }

    pub fn service_url(&self) -> Option<String> {
        let resolved_service_url = resolve_string_with_legacy_env(
            self.service_url.as_deref(),
            self.service_url_env.as_deref(),
        );
        let service_url = resolved_service_url.unwrap_or_else(default_signal_service_url);
        Some(service_url)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WhatsappAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default = "default_whatsapp_access_token_env")]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub phone_number_id: Option<String>,
    #[serde(default = "default_whatsapp_phone_number_id_env")]
    pub phone_number_id_env: Option<String>,
    #[serde(default)]
    pub verify_token: Option<SecretRef>,
    #[serde(default = "default_whatsapp_verify_token_env")]
    pub verify_token_env: Option<String>,
    #[serde(default)]
    pub app_secret: Option<SecretRef>,
    #[serde(default = "default_whatsapp_app_secret_env")]
    pub app_secret_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub webhook_bind: Option<String>,
    #[serde(default)]
    pub webhook_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWhatsappChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub access_token: Option<SecretRef>,
    pub access_token_env: Option<String>,
    pub phone_number_id: Option<String>,
    pub phone_number_id_env: Option<String>,
    pub verify_token: Option<SecretRef>,
    pub verify_token_env: Option<String>,
    pub app_secret: Option<SecretRef>,
    pub app_secret_env: Option<String>,
    pub api_base_url: Option<String>,
    pub webhook_bind: Option<String>,
    pub webhook_path: Option<String>,
}
impl ResolvedWhatsappChannelConfig {
    pub fn access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.access_token.as_ref(), self.access_token_env.as_deref())
    }

    pub fn phone_number_id(&self) -> Option<String> {
        resolve_string_with_legacy_env(
            self.phone_number_id.as_deref(),
            self.phone_number_id_env.as_deref(),
        )
    }

    pub fn verify_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.verify_token.as_ref(), self.verify_token_env.as_deref())
    }

    pub fn app_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.app_secret.as_ref(), self.app_secret_env.as_deref())
    }

    pub fn resolved_api_base_url(&self) -> String {
        self.api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_whatsapp_api_base_url)
    }

    pub fn resolved_webhook_bind(&self) -> String {
        self.webhook_bind
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "127.0.0.1:8080".to_owned())
    }

    pub fn resolved_webhook_path(&self) -> String {
        self.webhook_path
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "/webhook".to_owned())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TlonAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub ship: Option<String>,
    #[serde(default)]
    pub ship_env: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub url_env: Option<String>,
    #[serde(default)]
    pub code: Option<SecretRef>,
    #[serde(default)]
    pub code_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTlonChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub ship: Option<String>,
    pub ship_env: Option<String>,
    pub url: Option<String>,
    pub url_env: Option<String>,
    pub code: Option<SecretRef>,
    pub code_env: Option<String>,
}

impl ResolvedTlonChannelConfig {
    pub fn ship(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.ship.as_deref(), self.ship_env.as_deref())
    }

    pub fn url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.url.as_deref(), self.url_env.as_deref())
    }

    pub fn code(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.code.as_ref(), self.code_env.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DiscordChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default = "default_discord_bot_token_env")]
    pub bot_token_env: Option<String>,
    #[serde(default)]
    pub application_id: Option<String>,
    #[serde(default)]
    pub application_id_env: Option<String>,
    #[serde(default)]
    pub allowed_guild_ids: Vec<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, DiscordAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LineChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub channel_access_token: Option<SecretRef>,
    #[serde(default)]
    pub channel_access_token_env: Option<String>,
    #[serde(default)]
    pub channel_secret: Option<SecretRef>,
    #[serde(default)]
    pub channel_secret_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, LineAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct QqbotChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub managed_bridge_plugin_id: Option<String>,
    #[serde(default)]
    pub app_id: Option<SecretRef>,
    #[serde(default = "default_qqbot_app_id_env")]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub client_secret: Option<SecretRef>,
    #[serde(default = "default_qqbot_client_secret_env")]
    pub client_secret_env: Option<String>,
    #[serde(default)]
    pub allowed_peer_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, QqbotAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DingtalkChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<SecretRef>,
    #[serde(default)]
    pub webhook_url_env: Option<String>,
    #[serde(default)]
    pub secret: Option<SecretRef>,
    #[serde(default)]
    pub secret_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, DingtalkAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WebhookChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub endpoint_url: Option<SecretRef>,
    #[serde(default = "default_webhook_endpoint_url_env")]
    pub endpoint_url_env: Option<String>,
    #[serde(default)]
    pub auth_token: Option<SecretRef>,
    #[serde(default = "default_webhook_auth_token_env")]
    pub auth_token_env: Option<String>,
    #[serde(default = "default_webhook_auth_header_name")]
    pub auth_header_name: String,
    #[serde(default = "default_webhook_auth_token_prefix")]
    pub auth_token_prefix: String,
    #[serde(default)]
    pub payload_format: WebhookPayloadFormat,
    #[serde(default = "default_webhook_payload_text_field")]
    pub payload_text_field: String,
    #[serde(default)]
    pub public_base_url: Option<String>,
    #[serde(default)]
    pub signing_secret: Option<SecretRef>,
    #[serde(default = "default_webhook_signing_secret_env")]
    pub signing_secret_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, WebhookAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EmailChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub smtp_host: Option<String>,
    #[serde(default)]
    pub smtp_username: Option<SecretRef>,
    #[serde(default = "default_email_smtp_username_env")]
    pub smtp_username_env: Option<String>,
    #[serde(default)]
    pub smtp_password: Option<SecretRef>,
    #[serde(default = "default_email_smtp_password_env")]
    pub smtp_password_env: Option<String>,
    #[serde(default)]
    pub from_address: Option<String>,
    #[serde(default)]
    pub imap_host: Option<String>,
    #[serde(default)]
    pub imap_username: Option<SecretRef>,
    #[serde(default = "default_email_imap_username_env")]
    pub imap_username_env: Option<String>,
    #[serde(default)]
    pub imap_password: Option<SecretRef>,
    #[serde(default = "default_email_imap_password_env")]
    pub imap_password_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, EmailAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SlackChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default = "default_slack_bot_token_env")]
    pub bot_token_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, SlackAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GoogleChatChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<SecretRef>,
    #[serde(default)]
    pub webhook_url_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, GoogleChatAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MattermostChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_url_env: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretRef>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, MattermostAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NextcloudTalkChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_url_env: Option<String>,
    #[serde(default)]
    pub shared_secret: Option<SecretRef>,
    #[serde(default)]
    pub shared_secret_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, NextcloudTalkAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SynologyChatChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub token: Option<SecretRef>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub incoming_url: Option<SecretRef>,
    #[serde(default)]
    pub incoming_url_env: Option<String>,
    #[serde(default)]
    pub allowed_user_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, SynologyChatAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct IrcChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default = "default_irc_server_env")]
    pub server_env: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default = "default_irc_nickname_env")]
    pub nickname_env: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub realname: Option<String>,
    #[serde(default)]
    pub password: Option<SecretRef>,
    #[serde(default = "default_irc_password_env")]
    pub password_env: Option<String>,
    #[serde(default)]
    pub channel_names: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, IrcAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TeamsChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<SecretRef>,
    #[serde(default = "default_teams_webhook_url_env")]
    pub webhook_url_env: Option<String>,
    #[serde(default)]
    pub app_id: Option<SecretRef>,
    #[serde(default = "default_teams_app_id_env")]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub app_password: Option<SecretRef>,
    #[serde(default = "default_teams_app_password_env")]
    pub app_password_env: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default = "default_teams_tenant_id_env")]
    pub tenant_id_env: Option<String>,
    #[serde(default)]
    pub allowed_conversation_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, TeamsAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ImessageChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub bridge_url: Option<String>,
    #[serde(default = "default_imessage_bridge_url_env")]
    pub bridge_url_env: Option<String>,
    #[serde(default)]
    pub bridge_token: Option<SecretRef>,
    #[serde(default = "default_imessage_bridge_token_env")]
    pub bridge_token_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, ImessageAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SignalChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default, rename = "account")]
    pub signal_account: Option<String>,
    #[serde(default = "default_signal_account_env", rename = "account_env")]
    pub signal_account_env: Option<String>,
    #[serde(default)]
    pub service_url: Option<String>,
    #[serde(default = "default_signal_service_url_env")]
    pub service_url_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, SignalAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WhatsappChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default = "default_whatsapp_access_token_env")]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub phone_number_id: Option<String>,
    #[serde(default = "default_whatsapp_phone_number_id_env")]
    pub phone_number_id_env: Option<String>,
    #[serde(default)]
    pub verify_token: Option<SecretRef>,
    #[serde(default = "default_whatsapp_verify_token_env")]
    pub verify_token_env: Option<String>,
    #[serde(default)]
    pub app_secret: Option<SecretRef>,
    #[serde(default = "default_whatsapp_app_secret_env")]
    pub app_secret_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub webhook_bind: Option<String>,
    #[serde(default)]
    pub webhook_path: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, WhatsappAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TlonChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub ship: Option<String>,
    #[serde(default = "tlon_support::default_tlon_ship_env")]
    pub ship_env: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default = "tlon_support::default_tlon_url_env")]
    pub url_env: Option<String>,
    #[serde(default)]
    pub code: Option<SecretRef>,
    #[serde(default = "tlon_support::default_tlon_code_env")]
    pub code_env: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, TlonAccountConfig>,
}

impl Default for TelegramChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            bot_token: None,
            bot_token_env: Some(TELEGRAM_BOT_TOKEN_ENV.to_owned()),
            base_url: default_telegram_base_url(),
            polling_timeout_s: default_telegram_timeout_seconds(),
            allowed_chat_ids: Vec::new(),
            allowed_sender_ids: Vec::new(),
            require_mention: false,
            acp: ChannelAcpConfig::default(),
            streaming_mode: TelegramStreamingMode::default(),
            ack_reactions: true,
            accounts: BTreeMap::new(),
        }
    }
}

impl TelegramChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "telegram",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_telegram_env_pointer(
            &mut issues,
            "telegram.bot_token_env",
            self.bot_token_env.as_deref(),
            "telegram.bot_token",
        );
        validate_telegram_secret_ref_env_pointer(
            &mut issues,
            "telegram.bot_token",
            self.bot_token.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let field_path = format!("telegram.accounts.{account_id}.bot_token_env");
            let inline_field_path = format!("telegram.accounts.{account_id}.bot_token");
            validate_telegram_env_pointer(
                &mut issues,
                field_path.as_str(),
                account.bot_token_env.as_deref(),
                inline_field_path.as_str(),
            );
            validate_telegram_secret_ref_env_pointer(
                &mut issues,
                inline_field_path.as_str(),
                account.bot_token.as_ref(),
            );
        }
        issues
    }

    pub fn bot_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bot_token.as_ref(), self.bot_token_env.as_deref())
    }

    pub fn configured_account_ids(&self) -> Vec<String> {
        let ids = configured_account_ids(self.accounts.keys());
        if ids.is_empty() {
            return vec![self.default_configured_account_id()];
        }
        ids
    }

    pub fn default_configured_account_selection(&self) -> ChannelDefaultAccountSelection {
        resolve_default_configured_account_selection(
            self.accounts.keys(),
            self.default_account.as_deref(),
            self.resolved_account_identity().id.as_str(),
        )
    }

    pub fn default_configured_account_id(&self) -> String {
        self.default_configured_account_selection().id
    }

    pub fn resolved_account_route(
        &self,
        requested_account_id: Option<&str>,
        selected_configured_account_id: &str,
    ) -> ChannelResolvedAccountRoute {
        resolve_channel_account_route(
            self.accounts.keys(),
            self.default_account.as_deref(),
            self.resolved_account_identity().id.as_str(),
            requested_account_id,
            selected_configured_account_id,
        )
    }

    pub fn resolve_account(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedTelegramChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = TelegramChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            bot_token: account_override
                .and_then(|account| account.bot_token.clone())
                .or_else(|| self.bot_token.clone()),
            bot_token_env: account_override
                .and_then(|account| account.bot_token_env.clone())
                .or_else(|| self.bot_token_env.clone()),
            base_url: account_override
                .and_then(|account| account.base_url.clone())
                .unwrap_or_else(|| self.base_url.clone()),
            polling_timeout_s: account_override
                .and_then(|account| account.polling_timeout_s)
                .unwrap_or(self.polling_timeout_s),
            allowed_chat_ids: account_override
                .and_then(|account| account.allowed_chat_ids.clone())
                .unwrap_or_else(|| self.allowed_chat_ids.clone()),
            allowed_sender_ids: account_override
                .and_then(|account| account.allowed_sender_ids.clone())
                .unwrap_or_else(|| self.allowed_sender_ids.clone()),
            require_mention: account_override
                .and_then(|account| account.require_mention)
                .unwrap_or(self.require_mention),
            acp: resolve_channel_acp_config(
                &self.acp,
                account_override.and_then(|account| account.acp.as_ref()),
            ),
            streaming_mode: account_override
                .and_then(|account| account.streaming_mode)
                .unwrap_or(self.streaming_mode),
            ack_reactions: account_override
                .and_then(|account| account.ack_reactions)
                .unwrap_or(self.ack_reactions),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedTelegramChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            bot_token: merged.bot_token,
            bot_token_env: merged.bot_token_env,
            base_url: merged.base_url,
            polling_timeout_s: merged.polling_timeout_s,
            allowed_chat_ids: merged.allowed_chat_ids,
            allowed_sender_ids: merged.allowed_sender_ids,
            require_mention: merged.require_mention,
            acp: merged.acp,
            streaming_mode: merged.streaming_mode,
            ack_reactions: merged.ack_reactions,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedTelegramChannelConfig> {
        resolve_account_for_session_account_id(
            session_account_id,
            || self.resolve_account(session_account_id),
            || self.configured_account_ids(),
            |configured_id| self.resolve_account(Some(configured_id)),
            |resolved| resolved.account.id.as_str(),
        )
    }

    pub fn resolved_account_identity(&self) -> ChannelAccountIdentity {
        if let Some((id, label)) = resolve_configured_account_identity(self.account_id.as_deref()) {
            return ChannelAccountIdentity {
                id,
                label,
                source: ChannelAccountIdentitySource::Configured,
            };
        }

        if let Some(bot_id) = self
            .bot_token()
            .as_deref()
            .and_then(resolve_telegram_bot_id_from_token)
        {
            return ChannelAccountIdentity {
                id: format!("bot_{bot_id}"),
                label: format!("bot:{bot_id}"),
                source: ChannelAccountIdentitySource::DerivedCredential,
            };
        }

        default_channel_account_identity()
    }

    fn resolve_configured_account_selection(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedConfiguredAccount> {
        resolve_configured_account_selection(
            self.accounts.keys(),
            requested_account_id,
            self.default_account.as_deref(),
            self.resolved_account_identity().id.as_str(),
        )
    }
}

impl Default for FeishuChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            app_id: None,
            app_secret: None,
            app_id_env: Some(FEISHU_APP_ID_ENV.to_owned()),
            app_secret_env: Some(FEISHU_APP_SECRET_ENV.to_owned()),
            domain: FeishuDomain::Feishu,
            base_url: None,
            mode: Some(FeishuChannelServeMode::Websocket),
            receive_id_type: default_feishu_receive_id_type(),
            webhook_bind: default_feishu_webhook_bind(),
            webhook_path: default_feishu_webhook_path(),
            verification_token: None,
            verification_token_env: Some(FEISHU_VERIFICATION_TOKEN_ENV.to_owned()),
            encrypt_key: None,
            encrypt_key_env: Some(FEISHU_ENCRYPT_KEY_ENV.to_owned()),
            allowed_chat_ids: Vec::new(),
            allowed_sender_ids: Vec::new(),
            ack_reactions: true,
            ignore_bot_messages: true,
            require_mention: false,
            acp: ChannelAcpConfig::default(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for MatrixChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            user_id: None,
            access_token: None,
            access_token_env: Some(MATRIX_ACCESS_TOKEN_ENV.to_owned()),
            base_url: None,
            sync_timeout_s: default_matrix_sync_timeout_seconds(),
            allowed_room_ids: Vec::new(),
            allowed_sender_ids: Vec::new(),
            require_mention: false,
            ignore_self_messages: true,
            acp: ChannelAcpConfig::default(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for WecomChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            bot_id: None,
            secret: None,
            bot_id_env: Some(WECOM_BOT_ID_ENV.to_owned()),
            secret_env: Some(WECOM_SECRET_ENV.to_owned()),
            websocket_url: None,
            ping_interval_s: default_wecom_ping_interval_seconds(),
            reconnect_interval_s: default_wecom_reconnect_interval_seconds(),
            allowed_conversation_ids: Vec::new(),
            allowed_sender_ids: Vec::new(),
            acp: ChannelAcpConfig::default(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for WeixinChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            managed_bridge_plugin_id: None,
            bridge_url: None,
            bridge_url_env: Some(WEIXIN_BRIDGE_URL_ENV.to_owned()),
            bridge_access_token: None,
            bridge_access_token_env: Some(WEIXIN_BRIDGE_ACCESS_TOKEN_ENV.to_owned()),
            allowed_contact_ids: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for QqbotChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            managed_bridge_plugin_id: None,
            app_id: None,
            app_id_env: Some(QQBOT_APP_ID_ENV.to_owned()),
            client_secret: None,
            client_secret_env: Some(QQBOT_CLIENT_SECRET_ENV.to_owned()),
            allowed_peer_ids: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for OnebotChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            managed_bridge_plugin_id: None,
            websocket_url: None,
            websocket_url_env: Some(ONEBOT_WEBSOCKET_URL_ENV.to_owned()),
            access_token: None,
            access_token_env: Some(ONEBOT_ACCESS_TOKEN_ENV.to_owned()),
            allowed_group_ids: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for WhatsappPersonalChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            managed_bridge_plugin_id: None,
            bridge_url: None,
            bridge_url_env: Some(WHATSAPP_PERSONAL_BRIDGE_URL_ENV.to_owned()),
            auth_dir: None,
            auth_dir_env: Some(WHATSAPP_PERSONAL_AUTH_DIR_ENV.to_owned()),
            allowed_chat_ids: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for DiscordChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            bot_token: None,
            bot_token_env: Some(DISCORD_BOT_TOKEN_ENV.to_owned()),
            application_id: None,
            application_id_env: None,
            allowed_guild_ids: Vec::new(),
            api_base_url: None,
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for LineChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            channel_access_token: None,
            channel_access_token_env: Some(LINE_CHANNEL_ACCESS_TOKEN_ENV.to_owned()),
            channel_secret: None,
            channel_secret_env: Some(LINE_CHANNEL_SECRET_ENV.to_owned()),
            api_base_url: Some(default_line_api_base_url()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for DingtalkChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            webhook_url: None,
            webhook_url_env: Some(DINGTALK_WEBHOOK_URL_ENV.to_owned()),
            secret: None,
            secret_env: Some(DINGTALK_SECRET_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for WebhookChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            endpoint_url: None,
            endpoint_url_env: Some(WEBHOOK_ENDPOINT_URL_ENV.to_owned()),
            auth_token: None,
            auth_token_env: Some(WEBHOOK_AUTH_TOKEN_ENV.to_owned()),
            auth_header_name: default_webhook_auth_header_name(),
            auth_token_prefix: default_webhook_auth_token_prefix(),
            payload_format: WebhookPayloadFormat::default(),
            payload_text_field: default_webhook_payload_text_field(),
            public_base_url: None,
            signing_secret: None,
            signing_secret_env: Some(WEBHOOK_SIGNING_SECRET_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for EmailChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            smtp_host: None,
            smtp_username: None,
            smtp_username_env: Some(EMAIL_SMTP_USERNAME_ENV.to_owned()),
            smtp_password: None,
            smtp_password_env: Some(EMAIL_SMTP_PASSWORD_ENV.to_owned()),
            from_address: None,
            imap_host: None,
            imap_username: None,
            imap_username_env: Some(EMAIL_IMAP_USERNAME_ENV.to_owned()),
            imap_password: None,
            imap_password_env: Some(EMAIL_IMAP_PASSWORD_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for SlackChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            bot_token: None,
            bot_token_env: Some(SLACK_BOT_TOKEN_ENV.to_owned()),
            api_base_url: None,
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for GoogleChatChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            webhook_url: None,
            webhook_url_env: Some(GOOGLE_CHAT_WEBHOOK_URL_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for MattermostChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            server_url: None,
            server_url_env: Some(MATTERMOST_SERVER_URL_ENV.to_owned()),
            bot_token: None,
            bot_token_env: Some(MATTERMOST_BOT_TOKEN_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for NextcloudTalkChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            server_url: None,
            server_url_env: Some(NEXTCLOUD_TALK_SERVER_URL_ENV.to_owned()),
            shared_secret: None,
            shared_secret_env: Some(NEXTCLOUD_TALK_SHARED_SECRET_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for SynologyChatChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            token: None,
            token_env: Some(SYNOLOGY_CHAT_TOKEN_ENV.to_owned()),
            incoming_url: None,
            incoming_url_env: Some(SYNOLOGY_CHAT_INCOMING_URL_ENV.to_owned()),
            allowed_user_ids: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for TeamsChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            webhook_url: None,
            webhook_url_env: Some(TEAMS_WEBHOOK_URL_ENV.to_owned()),
            app_id: None,
            app_id_env: Some(TEAMS_APP_ID_ENV.to_owned()),
            app_password: None,
            app_password_env: Some(TEAMS_APP_PASSWORD_ENV.to_owned()),
            tenant_id: None,
            tenant_id_env: Some(TEAMS_TENANT_ID_ENV.to_owned()),
            allowed_conversation_ids: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for ImessageChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            bridge_url: None,
            bridge_url_env: Some(IMESSAGE_BRIDGE_URL_ENV.to_owned()),
            bridge_token: None,
            bridge_token_env: Some(IMESSAGE_BRIDGE_TOKEN_ENV.to_owned()),
            allowed_chat_ids: Vec::new(),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for SignalChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            signal_account: None,
            signal_account_env: Some(SIGNAL_ACCOUNT_ENV.to_owned()),
            service_url: None,
            service_url_env: Some(SIGNAL_SERVICE_URL_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for WhatsappChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            access_token: None,
            access_token_env: Some(WHATSAPP_ACCESS_TOKEN_ENV.to_owned()),
            phone_number_id: None,
            phone_number_id_env: Some(WHATSAPP_PHONE_NUMBER_ID_ENV.to_owned()),
            verify_token: None,
            verify_token_env: Some(WHATSAPP_VERIFY_TOKEN_ENV.to_owned()),
            app_secret: None,
            app_secret_env: Some(WHATSAPP_APP_SECRET_ENV.to_owned()),
            api_base_url: Some(default_whatsapp_api_base_url()),
            webhook_bind: None,
            webhook_path: None,
            accounts: BTreeMap::new(),
        }
    }
}

impl Default for TlonChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            ship: None,
            ship_env: Some(TLON_SHIP_ENV.to_owned()),
            url: None,
            url_env: Some(TLON_URL_ENV.to_owned()),
            code: None,
            code_env: Some(TLON_CODE_ENV.to_owned()),
            accounts: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod feishu_tests;

#[cfg(test)]
mod hotspot_tests;

#[cfg(test)]
mod partial_env_tests;

#[cfg(test)]
mod tests;
