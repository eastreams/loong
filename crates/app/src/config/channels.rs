use std::collections::BTreeMap;

use loongclaw_contracts::SecretRef;
use serde::{Deserialize, Serialize};

use crate::CliResult;
use crate::channel::sdk;
pub use crate::channel::sdk::{ChannelDescriptor, ChannelRuntimeKind};
use crate::prompt::{
    DEFAULT_PROMPT_PACK_ID, PromptPersonality, PromptRenderInput, render_default_system_prompt,
    render_system_prompt,
};

use super::runtime::LoongClawConfig;
use super::shared::{
    ConfigValidationCode, ConfigValidationIssue, ConfigValidationSeverity,
    EnvPointerValidationHint, validate_env_pointer_field, validate_secret_ref_env_pointer_field,
};
use crate::secrets::resolve_secret_with_legacy_env;

pub(crate) const TELEGRAM_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
pub(crate) const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
pub(crate) const DINGTALK_WEBHOOK_URL_ENV: &str = "DINGTALK_WEBHOOK_URL";
pub(crate) const DINGTALK_SECRET_ENV: &str = "DINGTALK_SECRET";
pub(crate) const EMAIL_SMTP_USERNAME_ENV: &str = "EMAIL_SMTP_USERNAME";
pub(crate) const EMAIL_SMTP_PASSWORD_ENV: &str = "EMAIL_SMTP_PASSWORD";
pub(crate) const EMAIL_IMAP_USERNAME_ENV: &str = "EMAIL_IMAP_USERNAME";
pub(crate) const EMAIL_IMAP_PASSWORD_ENV: &str = "EMAIL_IMAP_PASSWORD";
pub(crate) const FEISHU_APP_ID_ENV: &str = "FEISHU_APP_ID";
pub(crate) const FEISHU_APP_SECRET_ENV: &str = "FEISHU_APP_SECRET";
pub(crate) const FEISHU_VERIFICATION_TOKEN_ENV: &str = "FEISHU_VERIFICATION_TOKEN";
pub(crate) const FEISHU_ENCRYPT_KEY_ENV: &str = "FEISHU_ENCRYPT_KEY";
pub(crate) const GOOGLE_CHAT_WEBHOOK_URL_ENV: &str = "GOOGLE_CHAT_WEBHOOK_URL";
pub(crate) const LINE_CHANNEL_ACCESS_TOKEN_ENV: &str = "LINE_CHANNEL_ACCESS_TOKEN";
pub(crate) const LINE_CHANNEL_SECRET_ENV: &str = "LINE_CHANNEL_SECRET";
pub(crate) const MATRIX_ACCESS_TOKEN_ENV: &str = "MATRIX_ACCESS_TOKEN";
pub(crate) const MATTERMOST_SERVER_URL_ENV: &str = "MATTERMOST_SERVER_URL";
pub(crate) const MATTERMOST_BOT_TOKEN_ENV: &str = "MATTERMOST_BOT_TOKEN";
pub(crate) const NEXTCLOUD_TALK_SERVER_URL_ENV: &str = "NEXTCLOUD_TALK_SERVER_URL";
pub(crate) const NEXTCLOUD_TALK_SHARED_SECRET_ENV: &str = "NEXTCLOUD_TALK_SHARED_SECRET";
pub(crate) const SYNOLOGY_CHAT_TOKEN_ENV: &str = "SYNOLOGY_CHAT_TOKEN";
pub(crate) const SYNOLOGY_CHAT_INCOMING_URL_ENV: &str = "SYNOLOGY_CHAT_INCOMING_URL";
pub(crate) const SIGNAL_SERVICE_URL_ENV: &str = "SIGNAL_SERVICE_URL";
pub(crate) const SIGNAL_ACCOUNT_ENV: &str = "SIGNAL_ACCOUNT";
pub(crate) const SLACK_BOT_TOKEN_ENV: &str = "SLACK_BOT_TOKEN";
pub(crate) const TWITCH_ACCESS_TOKEN_ENV: &str = "TWITCH_ACCESS_TOKEN";
pub(crate) const TEAMS_APP_ID_ENV: &str = "TEAMS_APP_ID";
pub(crate) const TEAMS_APP_PASSWORD_ENV: &str = "TEAMS_APP_PASSWORD";
pub(crate) const TEAMS_TENANT_ID_ENV: &str = "TEAMS_TENANT_ID";
pub(crate) const TEAMS_WEBHOOK_URL_ENV: &str = "TEAMS_WEBHOOK_URL";
pub(crate) const IMESSAGE_BRIDGE_URL_ENV: &str = "IMESSAGE_BRIDGE_URL";
pub(crate) const IMESSAGE_BRIDGE_TOKEN_ENV: &str = "IMESSAGE_BRIDGE_TOKEN";
pub(crate) const IRC_SERVER_ENV: &str = "IRC_SERVER";
pub(crate) const IRC_NICKNAME_ENV: &str = "IRC_NICKNAME";
pub(crate) const IRC_PASSWORD_ENV: &str = "IRC_PASSWORD";
pub(crate) const WHATSAPP_ACCESS_TOKEN_ENV: &str = "WHATSAPP_ACCESS_TOKEN";
pub(crate) const WHATSAPP_PHONE_NUMBER_ID_ENV: &str = "WHATSAPP_PHONE_NUMBER_ID";
pub(crate) const WHATSAPP_VERIFY_TOKEN_ENV: &str = "WHATSAPP_VERIFY_TOKEN";
pub(crate) const WHATSAPP_APP_SECRET_ENV: &str = "WHATSAPP_APP_SECRET";
pub(crate) const WECOM_BOT_ID_ENV: &str = "WECOM_BOT_ID";
pub(crate) const WECOM_SECRET_ENV: &str = "WECOM_SECRET";
pub(crate) const WEBHOOK_ENDPOINT_URL_ENV: &str = "WEBHOOK_ENDPOINT_URL";
pub(crate) const WEBHOOK_AUTH_TOKEN_ENV: &str = "WEBHOOK_AUTH_TOKEN";
pub(crate) const WEBHOOK_SIGNING_SECRET_ENV: &str = "WEBHOOK_SIGNING_SECRET";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramStreamingMode {
    #[default]
    Off,
    Draft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookPayloadFormat {
    #[default]
    JsonText,
    PlainText,
}

impl WebhookPayloadFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JsonText => "json_text",
            Self::PlainText => "plain_text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EmailSmtpEndpoint {
    RelayHost(String),
    ConnectionUrl(String),
}

pub fn channel_descriptor(id: &str) -> Option<&'static ChannelDescriptor> {
    sdk::channel_descriptor(id)
}

pub fn service_channel_descriptors() -> Vec<&'static ChannelDescriptor> {
    sdk::service_channel_descriptors()
}

pub(super) fn enabled_channel_ids(config: &LoongClawConfig) -> Vec<String> {
    enabled_channel_ids_for_runtime_kind(config, None)
}

pub(super) fn enabled_service_channel_ids(config: &LoongClawConfig) -> Vec<String> {
    enabled_channel_ids_for_runtime_kind(config, Some(ChannelRuntimeKind::Service))
}

fn enabled_channel_ids_for_runtime_kind(
    config: &LoongClawConfig,
    runtime_kind: Option<ChannelRuntimeKind>,
) -> Vec<String> {
    sdk::enabled_channel_ids(config, runtime_kind)
}

pub(super) fn collect_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    sdk::collect_channel_validation_issues(config)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliChannelConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_prompt_pack_id")]
    pub prompt_pack_id: Option<String>,
    #[serde(default = "default_prompt_personality")]
    pub personality: Option<PromptPersonality>,
    #[serde(default)]
    pub system_prompt_addendum: Option<String>,
    #[serde(default = "default_exit_commands")]
    pub exit_commands: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelAcpConfig {
    #[serde(default)]
    pub bootstrap_mcp_servers: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
}

impl ChannelAcpConfig {
    pub fn resolved_working_directory(&self) -> Option<std::path::PathBuf> {
        self.working_directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
    }
}

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
    pub acp: ChannelAcpConfig,
    #[serde(default)]
    pub streaming_mode: TelegramStreamingMode,
    #[serde(default = "default_true")]
    pub ack_reactions: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, TelegramAccountConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeishuDomain {
    #[default]
    Feishu,
    Lark,
}

impl FeishuDomain {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feishu => "feishu",
            Self::Lark => "lark",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Feishu => "https://open.feishu.cn",
            Self::Lark => "https://open.larksuite.com",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAccountIdentitySource {
    Configured,
    DerivedCredential,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDefaultAccountSelectionSource {
    ExplicitDefault,
    MappedDefault,
    Fallback,
    RuntimeIdentity,
}

impl ChannelDefaultAccountSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitDefault => "explicit_default",
            Self::MappedDefault => "mapped_default",
            Self::Fallback => "fallback",
            Self::RuntimeIdentity => "runtime_identity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelAccountIdentity {
    pub id: String,
    pub label: String,
    pub source: ChannelAccountIdentitySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelDefaultAccountSelection {
    pub id: String,
    pub source: ChannelDefaultAccountSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelResolvedAccountRoute {
    pub requested_account_id: Option<String>,
    pub configured_account_count: usize,
    pub selected_configured_account_id: String,
    pub default_account_source: ChannelDefaultAccountSelectionSource,
}

impl ChannelResolvedAccountRoute {
    pub fn selected_by_default(&self) -> bool {
        self.requested_account_id.is_none()
    }

    pub fn uses_implicit_fallback_default(&self) -> bool {
        self.selected_by_default()
            && self.configured_account_count > 1
            && self.default_account_source == ChannelDefaultAccountSelectionSource::Fallback
    }
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
    pub ignore_bot_messages: Option<bool>,
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
    pub ignore_bot_messages: bool,
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
    #[serde(default = "default_true")]
    pub ignore_bot_messages: bool,
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

impl ResolvedIrcChannelConfig {
    pub fn server(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.server.as_deref(), self.server_env.as_deref())
    }

    pub fn nickname(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.nickname.as_deref(), self.nickname_env.as_deref())
    }

    pub fn username(&self) -> Option<&str> {
        self.username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn realname(&self) -> Option<&str> {
        self.realname
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn password(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.password.as_ref(), self.password_env.as_deref())
    }
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
pub struct TwitchAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default = "default_twitch_access_token_env")]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub oauth_base_url: Option<String>,
    #[serde(default)]
    pub channel_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTwitchChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub access_token: Option<SecretRef>,
    pub access_token_env: Option<String>,
    pub api_base_url: Option<String>,
    pub oauth_base_url: Option<String>,
    pub channel_names: Vec<String>,
}

impl ResolvedTwitchChannelConfig {
    pub fn access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.access_token.as_ref(), self.access_token_env.as_deref())
    }

    pub fn resolved_api_base_url(&self) -> String {
        self.api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_twitch_api_base_url)
    }

    pub fn resolved_oauth_base_url(&self) -> String {
        self.oauth_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_twitch_oauth_base_url)
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
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_whatsapp_api_base_url)
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
pub struct TwitchChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default = "default_twitch_access_token_env")]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub oauth_base_url: Option<String>,
    #[serde(default)]
    pub channel_names: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, TwitchAccountConfig>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, WhatsappAccountConfig>,
}

impl Default for CliChannelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            system_prompt: default_system_prompt(),
            prompt_pack_id: default_prompt_pack_id(),
            personality: default_prompt_personality(),
            system_prompt_addendum: None,
            exit_commands: default_exit_commands(),
        }
    }
}

impl CliChannelConfig {
    pub fn uses_native_prompt_pack(&self) -> bool {
        self.prompt_pack_id().is_some_and(|value| !value.is_empty())
    }

    pub fn prompt_pack_id(&self) -> Option<&str> {
        self.prompt_pack_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn resolved_personality(&self) -> PromptPersonality {
        self.personality.unwrap_or_default()
    }

    pub fn rendered_native_system_prompt(&self) -> String {
        render_system_prompt(PromptRenderInput {
            personality: self.resolved_personality(),
            addendum: self.system_prompt_addendum.clone(),
        })
    }

    pub fn resolved_system_prompt(&self) -> String {
        if self.uses_native_prompt_pack() {
            return self.rendered_native_system_prompt();
        }

        let inline = self.system_prompt.trim();
        if !inline.is_empty() {
            return inline.to_owned();
        }

        render_default_system_prompt()
    }

    pub fn refresh_native_system_prompt(&mut self) {
        if self.uses_native_prompt_pack() {
            self.system_prompt = self.rendered_native_system_prompt();
        }
    }
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
            mode: None,
            receive_id_type: default_feishu_receive_id_type(),
            webhook_bind: default_feishu_webhook_bind(),
            webhook_path: default_feishu_webhook_path(),
            verification_token: None,
            verification_token_env: Some(FEISHU_VERIFICATION_TOKEN_ENV.to_owned()),
            encrypt_key: None,
            encrypt_key_env: Some(FEISHU_ENCRYPT_KEY_ENV.to_owned()),
            allowed_chat_ids: Vec::new(),
            ignore_bot_messages: true,
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
            acp: ChannelAcpConfig::default(),
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

impl Default for IrcChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            server: None,
            server_env: Some(IRC_SERVER_ENV.to_owned()),
            nickname: None,
            nickname_env: Some(IRC_NICKNAME_ENV.to_owned()),
            username: None,
            realname: None,
            password: None,
            password_env: Some(IRC_PASSWORD_ENV.to_owned()),
            channel_names: Vec::new(),
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

impl Default for TwitchChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account_id: None,
            default_account: None,
            access_token: None,
            access_token_env: Some(TWITCH_ACCESS_TOKEN_ENV.to_owned()),
            api_base_url: Some(default_twitch_api_base_url()),
            oauth_base_url: Some(default_twitch_oauth_base_url()),
            channel_names: Vec::new(),
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
            accounts: BTreeMap::new(),
        }
    }
}

impl FeishuChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "feishu",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_feishu_env_pointer(
            &mut issues,
            "feishu.app_id_env",
            self.app_id_env.as_deref(),
            "feishu.app_id",
        );
        validate_feishu_secret_ref_env_pointer(&mut issues, "feishu.app_id", self.app_id.as_ref());
        validate_feishu_env_pointer(
            &mut issues,
            "feishu.app_secret_env",
            self.app_secret_env.as_deref(),
            "feishu.app_secret",
        );
        validate_feishu_secret_ref_env_pointer(
            &mut issues,
            "feishu.app_secret",
            self.app_secret.as_ref(),
        );
        validate_feishu_env_pointer(
            &mut issues,
            "feishu.verification_token_env",
            self.verification_token_env.as_deref(),
            "feishu.verification_token",
        );
        validate_feishu_secret_ref_env_pointer(
            &mut issues,
            "feishu.verification_token",
            self.verification_token.as_ref(),
        );
        validate_feishu_env_pointer(
            &mut issues,
            "feishu.encrypt_key_env",
            self.encrypt_key_env.as_deref(),
            "feishu.encrypt_key",
        );
        validate_feishu_secret_ref_env_pointer(
            &mut issues,
            "feishu.encrypt_key",
            self.encrypt_key.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let app_id_field_path = format!("feishu.accounts.{account_id}.app_id");
            validate_feishu_env_pointer(
                &mut issues,
                format!("{app_id_field_path}_env").as_str(),
                account.app_id_env.as_deref(),
                app_id_field_path.as_str(),
            );
            validate_feishu_secret_ref_env_pointer(
                &mut issues,
                app_id_field_path.as_str(),
                account.app_id.as_ref(),
            );
            let app_secret_field_path = format!("feishu.accounts.{account_id}.app_secret");
            validate_feishu_env_pointer(
                &mut issues,
                format!("{app_secret_field_path}_env").as_str(),
                account.app_secret_env.as_deref(),
                app_secret_field_path.as_str(),
            );
            validate_feishu_secret_ref_env_pointer(
                &mut issues,
                app_secret_field_path.as_str(),
                account.app_secret.as_ref(),
            );
            let verification_token_field_path =
                format!("feishu.accounts.{account_id}.verification_token");
            validate_feishu_env_pointer(
                &mut issues,
                format!("{verification_token_field_path}_env").as_str(),
                account.verification_token_env.as_deref(),
                verification_token_field_path.as_str(),
            );
            validate_feishu_secret_ref_env_pointer(
                &mut issues,
                verification_token_field_path.as_str(),
                account.verification_token.as_ref(),
            );
            let encrypt_key_field_path = format!("feishu.accounts.{account_id}.encrypt_key");
            validate_feishu_env_pointer(
                &mut issues,
                format!("{encrypt_key_field_path}_env").as_str(),
                account.encrypt_key_env.as_deref(),
                encrypt_key_field_path.as_str(),
            );
            validate_feishu_secret_ref_env_pointer(
                &mut issues,
                encrypt_key_field_path.as_str(),
                account.encrypt_key.as_ref(),
            );
        }
        issues
    }

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
    ) -> CliResult<ResolvedFeishuChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = FeishuChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            app_id: account_override
                .and_then(|account| account.app_id.clone())
                .or_else(|| self.app_id.clone()),
            app_secret: account_override
                .and_then(|account| account.app_secret.clone())
                .or_else(|| self.app_secret.clone()),
            app_id_env: account_override
                .and_then(|account| account.app_id_env.clone())
                .or_else(|| self.app_id_env.clone()),
            app_secret_env: account_override
                .and_then(|account| account.app_secret_env.clone())
                .or_else(|| self.app_secret_env.clone()),
            domain: account_override
                .and_then(|account| account.domain)
                .unwrap_or(self.domain),
            base_url: account_override
                .and_then(|account| account.base_url.clone())
                .or_else(|| self.base_url.clone()),
            mode: account_override
                .and_then(|account| account.mode)
                .or(self.mode),
            receive_id_type: account_override
                .and_then(|account| account.receive_id_type.clone())
                .unwrap_or_else(|| self.receive_id_type.clone()),
            webhook_bind: account_override
                .and_then(|account| account.webhook_bind.clone())
                .unwrap_or_else(|| self.webhook_bind.clone()),
            webhook_path: account_override
                .and_then(|account| account.webhook_path.clone())
                .unwrap_or_else(|| self.webhook_path.clone()),
            verification_token: account_override
                .and_then(|account| account.verification_token.clone())
                .or_else(|| self.verification_token.clone()),
            verification_token_env: account_override
                .and_then(|account| account.verification_token_env.clone())
                .or_else(|| self.verification_token_env.clone()),
            encrypt_key: account_override
                .and_then(|account| account.encrypt_key.clone())
                .or_else(|| self.encrypt_key.clone()),
            encrypt_key_env: account_override
                .and_then(|account| account.encrypt_key_env.clone())
                .or_else(|| self.encrypt_key_env.clone()),
            allowed_chat_ids: account_override
                .and_then(|account| account.allowed_chat_ids.clone())
                .unwrap_or_else(|| self.allowed_chat_ids.clone()),
            ignore_bot_messages: account_override
                .and_then(|account| account.ignore_bot_messages)
                .unwrap_or(self.ignore_bot_messages),
            acp: resolve_channel_acp_config(
                &self.acp,
                account_override.and_then(|account| account.acp.as_ref()),
            ),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedFeishuChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            app_id: merged.app_id,
            app_secret: merged.app_secret,
            app_id_env: merged.app_id_env,
            app_secret_env: merged.app_secret_env,
            domain: merged.domain,
            base_url: merged.base_url,
            mode: merged.mode.unwrap_or_default(),
            receive_id_type: merged.receive_id_type,
            webhook_bind: merged.webhook_bind,
            webhook_path: merged.webhook_path,
            verification_token: merged.verification_token,
            verification_token_env: merged.verification_token_env,
            encrypt_key: merged.encrypt_key,
            encrypt_key_env: merged.encrypt_key_env,
            allowed_chat_ids: merged.allowed_chat_ids,
            ignore_bot_messages: merged.ignore_bot_messages,
            acp: merged.acp,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedFeishuChannelConfig> {
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

        if let Some(app_id) = self
            .app_id()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return ChannelAccountIdentity {
                id: format!(
                    "{}_{}",
                    self.domain.as_str(),
                    normalize_channel_account_id(app_id)
                ),
                label: format!("{}:{app_id}", self.domain.as_str()),
                source: ChannelAccountIdentitySource::DerivedCredential,
            };
        }

        default_channel_account_identity()
    }

    pub fn resolved_base_url(&self) -> String {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.domain.default_base_url().to_owned())
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

impl MatrixChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "matrix",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_matrix_env_pointer(
            &mut issues,
            "matrix.access_token_env",
            self.access_token_env.as_deref(),
            "matrix.access_token",
        );
        validate_matrix_secret_ref_env_pointer(
            &mut issues,
            "matrix.access_token",
            self.access_token.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let access_token_field_path = format!("matrix.accounts.{account_id}.access_token");
            validate_matrix_env_pointer(
                &mut issues,
                format!("{access_token_field_path}_env").as_str(),
                account.access_token_env.as_deref(),
                access_token_field_path.as_str(),
            );
            validate_matrix_secret_ref_env_pointer(
                &mut issues,
                access_token_field_path.as_str(),
                account.access_token.as_ref(),
            );
        }
        issues
    }

    pub fn access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.access_token.as_ref(), self.access_token_env.as_deref())
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
    ) -> CliResult<ResolvedMatrixChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = MatrixChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            user_id: account_override
                .and_then(|account| account.user_id.clone())
                .or_else(|| self.user_id.clone()),
            access_token: account_override
                .and_then(|account| account.access_token.clone())
                .or_else(|| self.access_token.clone()),
            access_token_env: account_override
                .and_then(|account| account.access_token_env.clone())
                .or_else(|| self.access_token_env.clone()),
            base_url: account_override
                .and_then(|account| account.base_url.clone())
                .or_else(|| self.base_url.clone()),
            sync_timeout_s: account_override
                .and_then(|account| account.sync_timeout_s)
                .unwrap_or(self.sync_timeout_s),
            allowed_room_ids: account_override
                .and_then(|account| account.allowed_room_ids.clone())
                .unwrap_or_else(|| self.allowed_room_ids.clone()),
            ignore_self_messages: account_override
                .and_then(|account| account.ignore_self_messages)
                .unwrap_or(self.ignore_self_messages),
            acp: resolve_channel_acp_config(
                &self.acp,
                account_override.and_then(|account| account.acp.as_ref()),
            ),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedMatrixChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            user_id: merged.user_id,
            access_token: merged.access_token,
            access_token_env: merged.access_token_env,
            base_url: merged.base_url,
            sync_timeout_s: merged.sync_timeout_s,
            allowed_room_ids: merged.allowed_room_ids,
            ignore_self_messages: merged.ignore_self_messages,
            acp: merged.acp,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedMatrixChannelConfig> {
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

        if let Some((id, label)) = resolve_configured_account_identity(self.user_id.as_deref()) {
            return ChannelAccountIdentity {
                id,
                label,
                source: ChannelAccountIdentitySource::Configured,
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

impl WecomChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "wecom",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_wecom_env_pointer(
            &mut issues,
            "wecom.bot_id_env",
            self.bot_id_env.as_deref(),
            "wecom.bot_id",
        );
        validate_wecom_secret_ref_env_pointer(&mut issues, "wecom.bot_id", self.bot_id.as_ref());
        validate_wecom_env_pointer(
            &mut issues,
            "wecom.secret_env",
            self.secret_env.as_deref(),
            "wecom.secret",
        );
        validate_wecom_secret_ref_env_pointer(&mut issues, "wecom.secret", self.secret.as_ref());
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let bot_id_field_path = format!("wecom.accounts.{account_id}.bot_id");
            validate_wecom_env_pointer(
                &mut issues,
                format!("{bot_id_field_path}_env").as_str(),
                account.bot_id_env.as_deref(),
                bot_id_field_path.as_str(),
            );
            validate_wecom_secret_ref_env_pointer(
                &mut issues,
                bot_id_field_path.as_str(),
                account.bot_id.as_ref(),
            );
            let secret_field_path = format!("wecom.accounts.{account_id}.secret");
            validate_wecom_env_pointer(
                &mut issues,
                format!("{secret_field_path}_env").as_str(),
                account.secret_env.as_deref(),
                secret_field_path.as_str(),
            );
            validate_wecom_secret_ref_env_pointer(
                &mut issues,
                secret_field_path.as_str(),
                account.secret.as_ref(),
            );
        }
        issues
    }

    pub fn bot_id(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bot_id.as_ref(), self.bot_id_env.as_deref())
    }

    pub fn secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.secret.as_ref(), self.secret_env.as_deref())
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
    ) -> CliResult<ResolvedWecomChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = WecomChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            bot_id: account_override
                .and_then(|account| account.bot_id.clone())
                .or_else(|| self.bot_id.clone()),
            secret: account_override
                .and_then(|account| account.secret.clone())
                .or_else(|| self.secret.clone()),
            bot_id_env: account_override
                .and_then(|account| account.bot_id_env.clone())
                .or_else(|| self.bot_id_env.clone()),
            secret_env: account_override
                .and_then(|account| account.secret_env.clone())
                .or_else(|| self.secret_env.clone()),
            websocket_url: account_override
                .and_then(|account| account.websocket_url.clone())
                .or_else(|| self.websocket_url.clone()),
            ping_interval_s: account_override
                .and_then(|account| account.ping_interval_s)
                .unwrap_or(self.ping_interval_s),
            reconnect_interval_s: account_override
                .and_then(|account| account.reconnect_interval_s)
                .unwrap_or(self.reconnect_interval_s),
            allowed_conversation_ids: account_override
                .and_then(|account| account.allowed_conversation_ids.clone())
                .unwrap_or_else(|| self.allowed_conversation_ids.clone()),
            acp: resolve_channel_acp_config(
                &self.acp,
                account_override.and_then(|account| account.acp.as_ref()),
            ),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedWecomChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            bot_id: merged.bot_id,
            secret: merged.secret,
            bot_id_env: merged.bot_id_env,
            secret_env: merged.secret_env,
            websocket_url: merged.websocket_url,
            ping_interval_s: merged.ping_interval_s.clamp(1, 300),
            reconnect_interval_s: merged.reconnect_interval_s.clamp(1, 300),
            allowed_conversation_ids: merged.allowed_conversation_ids,
            acp: merged.acp,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedWecomChannelConfig> {
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
            .bot_id()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let normalized_bot_id = normalize_channel_account_id(bot_id);
            return ChannelAccountIdentity {
                id: format!("wecom_{normalized_bot_id}"),
                label: format!("wecom:{bot_id}"),
                source: ChannelAccountIdentitySource::DerivedCredential,
            };
        }

        default_channel_account_identity()
    }

    pub fn resolved_websocket_url(&self) -> String {
        self.websocket_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_wecom_websocket_url)
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

impl LineChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "line",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_line_env_pointer(
            &mut issues,
            "line.channel_access_token_env",
            self.channel_access_token_env.as_deref(),
            "line.channel_access_token",
        );
        validate_line_secret_ref_env_pointer(
            &mut issues,
            "line.channel_access_token",
            self.channel_access_token.as_ref(),
        );
        validate_line_env_pointer(
            &mut issues,
            "line.channel_secret_env",
            self.channel_secret_env.as_deref(),
            "line.channel_secret",
        );
        validate_line_secret_ref_env_pointer(
            &mut issues,
            "line.channel_secret",
            self.channel_secret.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let access_token_field_path =
                format!("line.accounts.{account_id}.channel_access_token");
            let access_token_env_field_path = format!("{access_token_field_path}_env");
            validate_line_env_pointer(
                &mut issues,
                access_token_env_field_path.as_str(),
                account.channel_access_token_env.as_deref(),
                access_token_field_path.as_str(),
            );
            validate_line_secret_ref_env_pointer(
                &mut issues,
                access_token_field_path.as_str(),
                account.channel_access_token.as_ref(),
            );
            let channel_secret_field_path = format!("line.accounts.{account_id}.channel_secret");
            let channel_secret_env_field_path = format!("{channel_secret_field_path}_env");
            validate_line_env_pointer(
                &mut issues,
                channel_secret_env_field_path.as_str(),
                account.channel_secret_env.as_deref(),
                channel_secret_field_path.as_str(),
            );
            validate_line_secret_ref_env_pointer(
                &mut issues,
                channel_secret_field_path.as_str(),
                account.channel_secret.as_ref(),
            );
        }
        issues
    }

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
    ) -> CliResult<ResolvedLineChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = LineChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            channel_access_token: account_override
                .and_then(|account| account.channel_access_token.clone())
                .or_else(|| self.channel_access_token.clone()),
            channel_access_token_env: account_override
                .and_then(|account| account.channel_access_token_env.clone())
                .or_else(|| self.channel_access_token_env.clone()),
            channel_secret: account_override
                .and_then(|account| account.channel_secret.clone())
                .or_else(|| self.channel_secret.clone()),
            channel_secret_env: account_override
                .and_then(|account| account.channel_secret_env.clone())
                .or_else(|| self.channel_secret_env.clone()),
            api_base_url: account_override
                .and_then(|account| account.api_base_url.clone())
                .or_else(|| self.api_base_url.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedLineChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            channel_access_token: merged.channel_access_token,
            channel_access_token_env: merged.channel_access_token_env,
            channel_secret: merged.channel_secret,
            channel_secret_env: merged.channel_secret_env,
            api_base_url: merged.api_base_url,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedLineChannelConfig> {
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

impl DingtalkChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "dingtalk",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_dingtalk_env_pointer(
            &mut issues,
            "dingtalk.webhook_url_env",
            self.webhook_url_env.as_deref(),
            "dingtalk.webhook_url",
        );
        validate_dingtalk_secret_ref_env_pointer(
            &mut issues,
            "dingtalk.webhook_url",
            self.webhook_url.as_ref(),
        );
        validate_dingtalk_env_pointer(
            &mut issues,
            "dingtalk.secret_env",
            self.secret_env.as_deref(),
            "dingtalk.secret",
        );
        validate_dingtalk_secret_ref_env_pointer(
            &mut issues,
            "dingtalk.secret",
            self.secret.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let webhook_url_field_path = format!("dingtalk.accounts.{account_id}.webhook_url");
            let webhook_url_env_field_path = format!("{webhook_url_field_path}_env");
            validate_dingtalk_env_pointer(
                &mut issues,
                webhook_url_env_field_path.as_str(),
                account.webhook_url_env.as_deref(),
                webhook_url_field_path.as_str(),
            );
            validate_dingtalk_secret_ref_env_pointer(
                &mut issues,
                webhook_url_field_path.as_str(),
                account.webhook_url.as_ref(),
            );
            let secret_field_path = format!("dingtalk.accounts.{account_id}.secret");
            let secret_env_field_path = format!("{secret_field_path}_env");
            validate_dingtalk_env_pointer(
                &mut issues,
                secret_env_field_path.as_str(),
                account.secret_env.as_deref(),
                secret_field_path.as_str(),
            );
            validate_dingtalk_secret_ref_env_pointer(
                &mut issues,
                secret_field_path.as_str(),
                account.secret.as_ref(),
            );
        }
        issues
    }

    pub fn webhook_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.webhook_url.as_ref(), self.webhook_url_env.as_deref())
    }

    pub fn secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.secret.as_ref(), self.secret_env.as_deref())
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
    ) -> CliResult<ResolvedDingtalkChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = DingtalkChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            webhook_url: account_override
                .and_then(|account| account.webhook_url.clone())
                .or_else(|| self.webhook_url.clone()),
            webhook_url_env: account_override
                .and_then(|account| account.webhook_url_env.clone())
                .or_else(|| self.webhook_url_env.clone()),
            secret: account_override
                .and_then(|account| account.secret.clone())
                .or_else(|| self.secret.clone()),
            secret_env: account_override
                .and_then(|account| account.secret_env.clone())
                .or_else(|| self.secret_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedDingtalkChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            webhook_url: merged.webhook_url,
            webhook_url_env: merged.webhook_url_env,
            secret: merged.secret,
            secret_env: merged.secret_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedDingtalkChannelConfig> {
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

impl WebhookChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "webhook",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_webhook_env_pointer(
            &mut issues,
            "webhook.endpoint_url_env",
            self.endpoint_url_env.as_deref(),
            "webhook.endpoint_url",
        );
        validate_webhook_secret_ref_env_pointer(
            &mut issues,
            "webhook.endpoint_url",
            self.endpoint_url.as_ref(),
        );
        validate_webhook_env_pointer(
            &mut issues,
            "webhook.auth_token_env",
            self.auth_token_env.as_deref(),
            "webhook.auth_token",
        );
        validate_webhook_secret_ref_env_pointer(
            &mut issues,
            "webhook.auth_token",
            self.auth_token.as_ref(),
        );
        validate_webhook_env_pointer(
            &mut issues,
            "webhook.signing_secret_env",
            self.signing_secret_env.as_deref(),
            "webhook.signing_secret",
        );
        validate_webhook_secret_ref_env_pointer(
            &mut issues,
            "webhook.signing_secret",
            self.signing_secret.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let endpoint_url_field_path = format!("webhook.accounts.{account_id}.endpoint_url");
            let endpoint_url_env_field_path = format!("{endpoint_url_field_path}_env");
            validate_webhook_env_pointer(
                &mut issues,
                endpoint_url_env_field_path.as_str(),
                account.endpoint_url_env.as_deref(),
                endpoint_url_field_path.as_str(),
            );
            validate_webhook_secret_ref_env_pointer(
                &mut issues,
                endpoint_url_field_path.as_str(),
                account.endpoint_url.as_ref(),
            );

            let auth_token_field_path = format!("webhook.accounts.{account_id}.auth_token");
            let auth_token_env_field_path = format!("{auth_token_field_path}_env");
            validate_webhook_env_pointer(
                &mut issues,
                auth_token_env_field_path.as_str(),
                account.auth_token_env.as_deref(),
                auth_token_field_path.as_str(),
            );
            validate_webhook_secret_ref_env_pointer(
                &mut issues,
                auth_token_field_path.as_str(),
                account.auth_token.as_ref(),
            );

            let signing_secret_field_path = format!("webhook.accounts.{account_id}.signing_secret");
            let signing_secret_env_field_path = format!("{signing_secret_field_path}_env");
            validate_webhook_env_pointer(
                &mut issues,
                signing_secret_env_field_path.as_str(),
                account.signing_secret_env.as_deref(),
                signing_secret_field_path.as_str(),
            );
            validate_webhook_secret_ref_env_pointer(
                &mut issues,
                signing_secret_field_path.as_str(),
                account.signing_secret.as_ref(),
            );
        }
        issues
    }

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
    ) -> CliResult<ResolvedWebhookChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = WebhookChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            endpoint_url: account_override
                .and_then(|account| account.endpoint_url.clone())
                .or_else(|| self.endpoint_url.clone()),
            endpoint_url_env: account_override
                .and_then(|account| account.endpoint_url_env.clone())
                .or_else(|| self.endpoint_url_env.clone()),
            auth_token: account_override
                .and_then(|account| account.auth_token.clone())
                .or_else(|| self.auth_token.clone()),
            auth_token_env: account_override
                .and_then(|account| account.auth_token_env.clone())
                .or_else(|| self.auth_token_env.clone()),
            auth_header_name: account_override
                .and_then(|account| account.auth_header_name.clone())
                .unwrap_or_else(|| self.auth_header_name.clone()),
            auth_token_prefix: account_override
                .and_then(|account| account.auth_token_prefix.clone())
                .unwrap_or_else(|| self.auth_token_prefix.clone()),
            payload_format: account_override
                .and_then(|account| account.payload_format)
                .unwrap_or(self.payload_format),
            payload_text_field: account_override
                .and_then(|account| account.payload_text_field.clone())
                .unwrap_or_else(|| self.payload_text_field.clone()),
            public_base_url: account_override
                .and_then(|account| account.public_base_url.clone())
                .or_else(|| self.public_base_url.clone()),
            signing_secret: account_override
                .and_then(|account| account.signing_secret.clone())
                .or_else(|| self.signing_secret.clone()),
            signing_secret_env: account_override
                .and_then(|account| account.signing_secret_env.clone())
                .or_else(|| self.signing_secret_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedWebhookChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            endpoint_url: merged.endpoint_url,
            endpoint_url_env: merged.endpoint_url_env,
            auth_token: merged.auth_token,
            auth_token_env: merged.auth_token_env,
            auth_header_name: merged.auth_header_name,
            auth_token_prefix: merged.auth_token_prefix,
            payload_format: merged.payload_format,
            payload_text_field: merged.payload_text_field,
            public_base_url: merged.public_base_url,
            signing_secret: merged.signing_secret,
            signing_secret_env: merged.signing_secret_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedWebhookChannelConfig> {
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

impl EmailChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "email",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_email_env_pointer(
            &mut issues,
            "email.smtp_username_env",
            self.smtp_username_env.as_deref(),
            "email.smtp_username",
        );
        validate_email_secret_ref_env_pointer(
            &mut issues,
            "email.smtp_username",
            self.smtp_username.as_ref(),
        );
        validate_email_env_pointer(
            &mut issues,
            "email.smtp_password_env",
            self.smtp_password_env.as_deref(),
            "email.smtp_password",
        );
        validate_email_secret_ref_env_pointer(
            &mut issues,
            "email.smtp_password",
            self.smtp_password.as_ref(),
        );
        validate_email_env_pointer(
            &mut issues,
            "email.imap_username_env",
            self.imap_username_env.as_deref(),
            "email.imap_username",
        );
        validate_email_secret_ref_env_pointer(
            &mut issues,
            "email.imap_username",
            self.imap_username.as_ref(),
        );
        validate_email_env_pointer(
            &mut issues,
            "email.imap_password_env",
            self.imap_password_env.as_deref(),
            "email.imap_password",
        );
        validate_email_secret_ref_env_pointer(
            &mut issues,
            "email.imap_password",
            self.imap_password.as_ref(),
        );

        if let Some(smtp_host) = self.smtp_host() {
            let parse_result = parse_email_smtp_endpoint(smtp_host.as_str());
            if let Err(error) = parse_result {
                let issue = build_email_invalid_value_issue(
                    "email.smtp_host",
                    error.as_str(),
                    "Configure a bare relay host like `smtp.example.com` or a full `smtp://` or `smtps://` URL.",
                );
                issues.push(issue);
            }
        }

        if let Some(from_address) = self.from_address() {
            let parse_result = from_address.parse::<lettre::message::Mailbox>();
            if parse_result.is_err() {
                let issue = build_email_invalid_value_issue(
                    "email.from_address",
                    "mailbox parse failed",
                    "Use a valid RFC 5322 mailbox like `ops@example.com` or `LoongClaw <ops@example.com>`.",
                );
                issues.push(issue);
            }
        }

        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let smtp_username_field_path = format!("email.accounts.{account_id}.smtp_username");
            let smtp_username_env_field_path = format!("{smtp_username_field_path}_env");
            validate_email_env_pointer(
                &mut issues,
                smtp_username_env_field_path.as_str(),
                account.smtp_username_env.as_deref(),
                smtp_username_field_path.as_str(),
            );
            validate_email_secret_ref_env_pointer(
                &mut issues,
                smtp_username_field_path.as_str(),
                account.smtp_username.as_ref(),
            );

            let smtp_password_field_path = format!("email.accounts.{account_id}.smtp_password");
            let smtp_password_env_field_path = format!("{smtp_password_field_path}_env");
            validate_email_env_pointer(
                &mut issues,
                smtp_password_env_field_path.as_str(),
                account.smtp_password_env.as_deref(),
                smtp_password_field_path.as_str(),
            );
            validate_email_secret_ref_env_pointer(
                &mut issues,
                smtp_password_field_path.as_str(),
                account.smtp_password.as_ref(),
            );

            let imap_username_field_path = format!("email.accounts.{account_id}.imap_username");
            let imap_username_env_field_path = format!("{imap_username_field_path}_env");
            validate_email_env_pointer(
                &mut issues,
                imap_username_env_field_path.as_str(),
                account.imap_username_env.as_deref(),
                imap_username_field_path.as_str(),
            );
            validate_email_secret_ref_env_pointer(
                &mut issues,
                imap_username_field_path.as_str(),
                account.imap_username.as_ref(),
            );

            let imap_password_field_path = format!("email.accounts.{account_id}.imap_password");
            let imap_password_env_field_path = format!("{imap_password_field_path}_env");
            validate_email_env_pointer(
                &mut issues,
                imap_password_env_field_path.as_str(),
                account.imap_password_env.as_deref(),
                imap_password_field_path.as_str(),
            );
            validate_email_secret_ref_env_pointer(
                &mut issues,
                imap_password_field_path.as_str(),
                account.imap_password.as_ref(),
            );

            let smtp_host = account
                .smtp_host
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if let Some(smtp_host) = smtp_host {
                let parse_result = parse_email_smtp_endpoint(smtp_host.as_str());
                if let Err(error) = parse_result {
                    let field_path = format!("email.accounts.{account_id}.smtp_host");
                    let issue = build_email_invalid_value_issue(
                        field_path.as_str(),
                        error.as_str(),
                        "Configure a bare relay host like `smtp.example.com` or a full `smtp://` or `smtps://` URL.",
                    );
                    issues.push(issue);
                }
            }

            let from_address = account
                .from_address
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if let Some(from_address) = from_address {
                let parse_result = from_address.parse::<lettre::message::Mailbox>();
                if parse_result.is_err() {
                    let field_path = format!("email.accounts.{account_id}.from_address");
                    let issue = build_email_invalid_value_issue(
                        field_path.as_str(),
                        "mailbox parse failed",
                        "Use a valid RFC 5322 mailbox like `ops@example.com` or `LoongClaw <ops@example.com>`.",
                    );
                    issues.push(issue);
                }
            }
        }

        issues
    }

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
    ) -> CliResult<ResolvedEmailChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = EmailChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            smtp_host: account_override
                .and_then(|account| account.smtp_host.clone())
                .or_else(|| self.smtp_host.clone()),
            smtp_username: account_override
                .and_then(|account| account.smtp_username.clone())
                .or_else(|| self.smtp_username.clone()),
            smtp_username_env: account_override
                .and_then(|account| account.smtp_username_env.clone())
                .or_else(|| self.smtp_username_env.clone()),
            smtp_password: account_override
                .and_then(|account| account.smtp_password.clone())
                .or_else(|| self.smtp_password.clone()),
            smtp_password_env: account_override
                .and_then(|account| account.smtp_password_env.clone())
                .or_else(|| self.smtp_password_env.clone()),
            from_address: account_override
                .and_then(|account| account.from_address.clone())
                .or_else(|| self.from_address.clone()),
            imap_host: account_override
                .and_then(|account| account.imap_host.clone())
                .or_else(|| self.imap_host.clone()),
            imap_username: account_override
                .and_then(|account| account.imap_username.clone())
                .or_else(|| self.imap_username.clone()),
            imap_username_env: account_override
                .and_then(|account| account.imap_username_env.clone())
                .or_else(|| self.imap_username_env.clone()),
            imap_password: account_override
                .and_then(|account| account.imap_password.clone())
                .or_else(|| self.imap_password.clone()),
            imap_password_env: account_override
                .and_then(|account| account.imap_password_env.clone())
                .or_else(|| self.imap_password_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedEmailChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            smtp_host: merged.smtp_host,
            smtp_username: merged.smtp_username,
            smtp_username_env: merged.smtp_username_env,
            smtp_password: merged.smtp_password,
            smtp_password_env: merged.smtp_password_env,
            from_address: merged.from_address,
            imap_host: merged.imap_host,
            imap_username: merged.imap_username,
            imap_username_env: merged.imap_username_env,
            imap_password: merged.imap_password,
            imap_password_env: merged.imap_password_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedEmailChannelConfig> {
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

impl DiscordChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "discord",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_discord_env_pointer(
            &mut issues,
            "discord.bot_token_env",
            self.bot_token_env.as_deref(),
            "discord.bot_token",
        );
        validate_discord_secret_ref_env_pointer(
            &mut issues,
            "discord.bot_token",
            self.bot_token.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let bot_token_field_path = format!("discord.accounts.{account_id}.bot_token");
            let bot_token_env_field_path = format!("{bot_token_field_path}_env");
            validate_discord_env_pointer(
                &mut issues,
                bot_token_env_field_path.as_str(),
                account.bot_token_env.as_deref(),
                bot_token_field_path.as_str(),
            );
            validate_discord_secret_ref_env_pointer(
                &mut issues,
                bot_token_field_path.as_str(),
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
    ) -> CliResult<ResolvedDiscordChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = DiscordChannelConfig {
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
            api_base_url: account_override
                .and_then(|account| account.api_base_url.clone())
                .or_else(|| self.api_base_url.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedDiscordChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            bot_token: merged.bot_token,
            bot_token_env: merged.bot_token_env,
            api_base_url: merged.api_base_url,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedDiscordChannelConfig> {
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

impl SlackChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "slack",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_slack_env_pointer(
            &mut issues,
            "slack.bot_token_env",
            self.bot_token_env.as_deref(),
            "slack.bot_token",
        );
        validate_slack_secret_ref_env_pointer(
            &mut issues,
            "slack.bot_token",
            self.bot_token.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let bot_token_field_path = format!("slack.accounts.{account_id}.bot_token");
            let bot_token_env_field_path = format!("{bot_token_field_path}_env");
            validate_slack_env_pointer(
                &mut issues,
                bot_token_env_field_path.as_str(),
                account.bot_token_env.as_deref(),
                bot_token_field_path.as_str(),
            );
            validate_slack_secret_ref_env_pointer(
                &mut issues,
                bot_token_field_path.as_str(),
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
    ) -> CliResult<ResolvedSlackChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = SlackChannelConfig {
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
            api_base_url: account_override
                .and_then(|account| account.api_base_url.clone())
                .or_else(|| self.api_base_url.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedSlackChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            bot_token: merged.bot_token,
            bot_token_env: merged.bot_token_env,
            api_base_url: merged.api_base_url,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedSlackChannelConfig> {
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

impl GoogleChatChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "google_chat",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_google_chat_env_pointer(
            &mut issues,
            "google_chat.webhook_url_env",
            self.webhook_url_env.as_deref(),
            "google_chat.webhook_url",
        );
        validate_google_chat_secret_ref_env_pointer(
            &mut issues,
            "google_chat.webhook_url",
            self.webhook_url.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let webhook_url_field_path = format!("google_chat.accounts.{account_id}.webhook_url");
            let webhook_url_env_field_path = format!("{webhook_url_field_path}_env");
            validate_google_chat_env_pointer(
                &mut issues,
                webhook_url_env_field_path.as_str(),
                account.webhook_url_env.as_deref(),
                webhook_url_field_path.as_str(),
            );
            validate_google_chat_secret_ref_env_pointer(
                &mut issues,
                webhook_url_field_path.as_str(),
                account.webhook_url.as_ref(),
            );
        }
        issues
    }

    pub fn webhook_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.webhook_url.as_ref(), self.webhook_url_env.as_deref())
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
    ) -> CliResult<ResolvedGoogleChatChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = GoogleChatChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            webhook_url: account_override
                .and_then(|account| account.webhook_url.clone())
                .or_else(|| self.webhook_url.clone()),
            webhook_url_env: account_override
                .and_then(|account| account.webhook_url_env.clone())
                .or_else(|| self.webhook_url_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedGoogleChatChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            webhook_url: merged.webhook_url,
            webhook_url_env: merged.webhook_url_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedGoogleChatChannelConfig> {
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

impl MattermostChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "mattermost",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_mattermost_env_pointer(
            &mut issues,
            "mattermost.server_url_env",
            self.server_url_env.as_deref(),
            "mattermost.server_url",
        );
        validate_mattermost_env_pointer(
            &mut issues,
            "mattermost.bot_token_env",
            self.bot_token_env.as_deref(),
            "mattermost.bot_token",
        );
        validate_mattermost_secret_ref_env_pointer(
            &mut issues,
            "mattermost.bot_token",
            self.bot_token.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let server_url_field_path = format!("mattermost.accounts.{account_id}.server_url");
            let server_url_env_field_path = format!("{server_url_field_path}_env");
            validate_mattermost_env_pointer(
                &mut issues,
                server_url_env_field_path.as_str(),
                account.server_url_env.as_deref(),
                server_url_field_path.as_str(),
            );
            let bot_token_field_path = format!("mattermost.accounts.{account_id}.bot_token");
            let bot_token_env_field_path = format!("{bot_token_field_path}_env");
            validate_mattermost_env_pointer(
                &mut issues,
                bot_token_env_field_path.as_str(),
                account.bot_token_env.as_deref(),
                bot_token_field_path.as_str(),
            );
            validate_mattermost_secret_ref_env_pointer(
                &mut issues,
                bot_token_field_path.as_str(),
                account.bot_token.as_ref(),
            );
        }
        issues
    }

    pub fn server_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.server_url.as_deref(), self.server_url_env.as_deref())
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
    ) -> CliResult<ResolvedMattermostChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = MattermostChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            server_url: account_override
                .and_then(|account| account.server_url.clone())
                .or_else(|| self.server_url.clone()),
            server_url_env: account_override
                .and_then(|account| account.server_url_env.clone())
                .or_else(|| self.server_url_env.clone()),
            bot_token: account_override
                .and_then(|account| account.bot_token.clone())
                .or_else(|| self.bot_token.clone()),
            bot_token_env: account_override
                .and_then(|account| account.bot_token_env.clone())
                .or_else(|| self.bot_token_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedMattermostChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            server_url: merged.server_url,
            server_url_env: merged.server_url_env,
            bot_token: merged.bot_token,
            bot_token_env: merged.bot_token_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedMattermostChannelConfig> {
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

impl NextcloudTalkChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "nextcloud_talk",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_nextcloud_talk_env_pointer(
            &mut issues,
            "nextcloud_talk.server_url_env",
            self.server_url_env.as_deref(),
            "nextcloud_talk.server_url",
        );
        validate_nextcloud_talk_env_pointer(
            &mut issues,
            "nextcloud_talk.shared_secret_env",
            self.shared_secret_env.as_deref(),
            "nextcloud_talk.shared_secret",
        );
        validate_nextcloud_talk_secret_ref_env_pointer(
            &mut issues,
            "nextcloud_talk.shared_secret",
            self.shared_secret.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let server_url_field_path = format!("nextcloud_talk.accounts.{account_id}.server_url");
            let server_url_env_field_path = format!("{server_url_field_path}_env");
            validate_nextcloud_talk_env_pointer(
                &mut issues,
                server_url_env_field_path.as_str(),
                account.server_url_env.as_deref(),
                server_url_field_path.as_str(),
            );

            let shared_secret_field_path =
                format!("nextcloud_talk.accounts.{account_id}.shared_secret");
            let shared_secret_env_field_path = format!("{shared_secret_field_path}_env");
            validate_nextcloud_talk_env_pointer(
                &mut issues,
                shared_secret_env_field_path.as_str(),
                account.shared_secret_env.as_deref(),
                shared_secret_field_path.as_str(),
            );
            validate_nextcloud_talk_secret_ref_env_pointer(
                &mut issues,
                shared_secret_field_path.as_str(),
                account.shared_secret.as_ref(),
            );
        }
        issues
    }

    pub fn server_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.server_url.as_deref(), self.server_url_env.as_deref())
    }

    pub fn shared_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.shared_secret.as_ref(),
            self.shared_secret_env.as_deref(),
        )
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
    ) -> CliResult<ResolvedNextcloudTalkChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = NextcloudTalkChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            server_url: account_override
                .and_then(|account| account.server_url.clone())
                .or_else(|| self.server_url.clone()),
            server_url_env: account_override
                .and_then(|account| account.server_url_env.clone())
                .or_else(|| self.server_url_env.clone()),
            shared_secret: account_override
                .and_then(|account| account.shared_secret.clone())
                .or_else(|| self.shared_secret.clone()),
            shared_secret_env: account_override
                .and_then(|account| account.shared_secret_env.clone())
                .or_else(|| self.shared_secret_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedNextcloudTalkChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            server_url: merged.server_url,
            server_url_env: merged.server_url_env,
            shared_secret: merged.shared_secret,
            shared_secret_env: merged.shared_secret_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedNextcloudTalkChannelConfig> {
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

impl SynologyChatChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "synology_chat",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_synology_chat_env_pointer(
            &mut issues,
            "synology_chat.token_env",
            self.token_env.as_deref(),
            "synology_chat.token",
        );
        validate_synology_chat_secret_ref_env_pointer(
            &mut issues,
            "synology_chat.token",
            self.token.as_ref(),
        );
        validate_synology_chat_env_pointer(
            &mut issues,
            "synology_chat.incoming_url_env",
            self.incoming_url_env.as_deref(),
            "synology_chat.incoming_url",
        );
        validate_synology_chat_secret_ref_env_pointer(
            &mut issues,
            "synology_chat.incoming_url",
            self.incoming_url.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let token_field_path = format!("synology_chat.accounts.{account_id}.token");
            let token_env_field_path = format!("{token_field_path}_env");
            validate_synology_chat_env_pointer(
                &mut issues,
                token_env_field_path.as_str(),
                account.token_env.as_deref(),
                token_field_path.as_str(),
            );
            validate_synology_chat_secret_ref_env_pointer(
                &mut issues,
                token_field_path.as_str(),
                account.token.as_ref(),
            );

            let incoming_url_field_path =
                format!("synology_chat.accounts.{account_id}.incoming_url");
            let incoming_url_env_field_path = format!("{incoming_url_field_path}_env");
            validate_synology_chat_env_pointer(
                &mut issues,
                incoming_url_env_field_path.as_str(),
                account.incoming_url_env.as_deref(),
                incoming_url_field_path.as_str(),
            );
            validate_synology_chat_secret_ref_env_pointer(
                &mut issues,
                incoming_url_field_path.as_str(),
                account.incoming_url.as_ref(),
            );
        }
        issues
    }

    pub fn token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.token.as_ref(), self.token_env.as_deref())
    }

    pub fn incoming_url(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.incoming_url.as_ref(), self.incoming_url_env.as_deref())
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
    ) -> CliResult<ResolvedSynologyChatChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = SynologyChatChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            token: account_override
                .and_then(|account| account.token.clone())
                .or_else(|| self.token.clone()),
            token_env: account_override
                .and_then(|account| account.token_env.clone())
                .or_else(|| self.token_env.clone()),
            incoming_url: account_override
                .and_then(|account| account.incoming_url.clone())
                .or_else(|| self.incoming_url.clone()),
            incoming_url_env: account_override
                .and_then(|account| account.incoming_url_env.clone())
                .or_else(|| self.incoming_url_env.clone()),
            allowed_user_ids: account_override
                .and_then(|account| account.allowed_user_ids.clone())
                .unwrap_or_else(|| self.allowed_user_ids.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedSynologyChatChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            token: merged.token,
            token_env: merged.token_env,
            incoming_url: merged.incoming_url,
            incoming_url_env: merged.incoming_url_env,
            allowed_user_ids: merged.allowed_user_ids,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedSynologyChatChannelConfig> {
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

impl IrcChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "irc",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_irc_env_pointer(
            &mut issues,
            "irc.server_env",
            self.server_env.as_deref(),
            "irc.server",
        );
        validate_irc_env_pointer(
            &mut issues,
            "irc.nickname_env",
            self.nickname_env.as_deref(),
            "irc.nickname",
        );
        validate_irc_env_pointer(
            &mut issues,
            "irc.password_env",
            self.password_env.as_deref(),
            "irc.password",
        );
        validate_irc_secret_ref_env_pointer(&mut issues, "irc.password", self.password.as_ref());
        validate_irc_server_field(&mut issues, "irc.server", self.server());

        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let server_field_path = format!("irc.accounts.{account_id}.server");
            let server_env_field_path = format!("{server_field_path}_env");
            validate_irc_env_pointer(
                &mut issues,
                server_env_field_path.as_str(),
                account.server_env.as_deref(),
                server_field_path.as_str(),
            );
            let server = resolve_string_with_legacy_env(
                account.server.as_deref(),
                account.server_env.as_deref(),
            );
            validate_irc_server_field(&mut issues, server_field_path.as_str(), server);

            let nickname_field_path = format!("irc.accounts.{account_id}.nickname");
            let nickname_env_field_path = format!("{nickname_field_path}_env");
            validate_irc_env_pointer(
                &mut issues,
                nickname_env_field_path.as_str(),
                account.nickname_env.as_deref(),
                nickname_field_path.as_str(),
            );

            let password_field_path = format!("irc.accounts.{account_id}.password");
            let password_env_field_path = format!("{password_field_path}_env");
            validate_irc_env_pointer(
                &mut issues,
                password_env_field_path.as_str(),
                account.password_env.as_deref(),
                password_field_path.as_str(),
            );
            validate_irc_secret_ref_env_pointer(
                &mut issues,
                password_field_path.as_str(),
                account.password.as_ref(),
            );
        }

        issues
    }

    pub fn server(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.server.as_deref(), self.server_env.as_deref())
    }

    pub fn nickname(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.nickname.as_deref(), self.nickname_env.as_deref())
    }

    pub fn password(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.password.as_ref(), self.password_env.as_deref())
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
    ) -> CliResult<ResolvedIrcChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = IrcChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            server: account_override
                .and_then(|account| account.server.clone())
                .or_else(|| self.server.clone()),
            server_env: account_override
                .and_then(|account| account.server_env.clone())
                .or_else(|| self.server_env.clone()),
            nickname: account_override
                .and_then(|account| account.nickname.clone())
                .or_else(|| self.nickname.clone()),
            nickname_env: account_override
                .and_then(|account| account.nickname_env.clone())
                .or_else(|| self.nickname_env.clone()),
            username: account_override
                .and_then(|account| account.username.clone())
                .or_else(|| self.username.clone()),
            realname: account_override
                .and_then(|account| account.realname.clone())
                .or_else(|| self.realname.clone()),
            password: account_override
                .and_then(|account| account.password.clone())
                .or_else(|| self.password.clone()),
            password_env: account_override
                .and_then(|account| account.password_env.clone())
                .or_else(|| self.password_env.clone()),
            channel_names: account_override
                .and_then(|account| account.channel_names.clone())
                .unwrap_or_else(|| self.channel_names.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedIrcChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            server: merged.server,
            server_env: merged.server_env,
            nickname: merged.nickname,
            nickname_env: merged.nickname_env,
            username: merged.username,
            realname: merged.realname,
            password: merged.password,
            password_env: merged.password_env,
            channel_names: merged.channel_names,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedIrcChannelConfig> {
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

        let nickname = self.nickname();
        let nickname = nickname
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(nickname) = nickname {
            let normalized_nickname = normalize_channel_account_id(nickname);
            let account_id = format!("irc_{normalized_nickname}");
            let account_label = format!("irc:{nickname}");
            return ChannelAccountIdentity {
                id: account_id,
                label: account_label,
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

impl TeamsChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "teams",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_teams_env_pointer(
            &mut issues,
            "teams.webhook_url_env",
            self.webhook_url_env.as_deref(),
            "teams.webhook_url",
        );
        validate_teams_secret_ref_env_pointer(
            &mut issues,
            "teams.webhook_url",
            self.webhook_url.as_ref(),
        );
        validate_teams_env_pointer(
            &mut issues,
            "teams.app_id_env",
            self.app_id_env.as_deref(),
            "teams.app_id",
        );
        validate_teams_secret_ref_env_pointer(&mut issues, "teams.app_id", self.app_id.as_ref());
        validate_teams_env_pointer(
            &mut issues,
            "teams.app_password_env",
            self.app_password_env.as_deref(),
            "teams.app_password",
        );
        validate_teams_secret_ref_env_pointer(
            &mut issues,
            "teams.app_password",
            self.app_password.as_ref(),
        );
        validate_teams_env_pointer(
            &mut issues,
            "teams.tenant_id_env",
            self.tenant_id_env.as_deref(),
            "teams.tenant_id",
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let webhook_url_field_path = format!("teams.accounts.{account_id}.webhook_url");
            let webhook_url_env_field_path = format!("{webhook_url_field_path}_env");
            validate_teams_env_pointer(
                &mut issues,
                webhook_url_env_field_path.as_str(),
                account.webhook_url_env.as_deref(),
                webhook_url_field_path.as_str(),
            );
            validate_teams_secret_ref_env_pointer(
                &mut issues,
                webhook_url_field_path.as_str(),
                account.webhook_url.as_ref(),
            );

            let app_id_field_path = format!("teams.accounts.{account_id}.app_id");
            let app_id_env_field_path = format!("{app_id_field_path}_env");
            validate_teams_env_pointer(
                &mut issues,
                app_id_env_field_path.as_str(),
                account.app_id_env.as_deref(),
                app_id_field_path.as_str(),
            );
            validate_teams_secret_ref_env_pointer(
                &mut issues,
                app_id_field_path.as_str(),
                account.app_id.as_ref(),
            );

            let app_password_field_path = format!("teams.accounts.{account_id}.app_password");
            let app_password_env_field_path = format!("{app_password_field_path}_env");
            validate_teams_env_pointer(
                &mut issues,
                app_password_env_field_path.as_str(),
                account.app_password_env.as_deref(),
                app_password_field_path.as_str(),
            );
            validate_teams_secret_ref_env_pointer(
                &mut issues,
                app_password_field_path.as_str(),
                account.app_password.as_ref(),
            );

            let tenant_id_field_path = format!("teams.accounts.{account_id}.tenant_id");
            let tenant_id_env_field_path = format!("{tenant_id_field_path}_env");
            validate_teams_env_pointer(
                &mut issues,
                tenant_id_env_field_path.as_str(),
                account.tenant_id_env.as_deref(),
                tenant_id_field_path.as_str(),
            );
        }
        issues
    }

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
    ) -> CliResult<ResolvedTeamsChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = TeamsChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            webhook_url: account_override
                .and_then(|account| account.webhook_url.clone())
                .or_else(|| self.webhook_url.clone()),
            webhook_url_env: account_override
                .and_then(|account| account.webhook_url_env.clone())
                .or_else(|| self.webhook_url_env.clone()),
            app_id: account_override
                .and_then(|account| account.app_id.clone())
                .or_else(|| self.app_id.clone()),
            app_id_env: account_override
                .and_then(|account| account.app_id_env.clone())
                .or_else(|| self.app_id_env.clone()),
            app_password: account_override
                .and_then(|account| account.app_password.clone())
                .or_else(|| self.app_password.clone()),
            app_password_env: account_override
                .and_then(|account| account.app_password_env.clone())
                .or_else(|| self.app_password_env.clone()),
            tenant_id: account_override
                .and_then(|account| account.tenant_id.clone())
                .or_else(|| self.tenant_id.clone()),
            tenant_id_env: account_override
                .and_then(|account| account.tenant_id_env.clone())
                .or_else(|| self.tenant_id_env.clone()),
            allowed_conversation_ids: account_override
                .and_then(|account| account.allowed_conversation_ids.clone())
                .unwrap_or_else(|| self.allowed_conversation_ids.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedTeamsChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            webhook_url: merged.webhook_url,
            webhook_url_env: merged.webhook_url_env,
            app_id: merged.app_id,
            app_id_env: merged.app_id_env,
            app_password: merged.app_password,
            app_password_env: merged.app_password_env,
            tenant_id: merged.tenant_id,
            tenant_id_env: merged.tenant_id_env,
            allowed_conversation_ids: merged.allowed_conversation_ids,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedTeamsChannelConfig> {
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

impl ImessageChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "imessage",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_imessage_env_pointer(
            &mut issues,
            "imessage.bridge_url_env",
            self.bridge_url_env.as_deref(),
            "imessage.bridge_url",
        );
        validate_imessage_env_pointer(
            &mut issues,
            "imessage.bridge_token_env",
            self.bridge_token_env.as_deref(),
            "imessage.bridge_token",
        );
        validate_imessage_secret_ref_env_pointer(
            &mut issues,
            "imessage.bridge_token",
            self.bridge_token.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let bridge_url_field_path = format!("imessage.accounts.{account_id}.bridge_url");
            let bridge_url_env_field_path = format!("{bridge_url_field_path}_env");
            validate_imessage_env_pointer(
                &mut issues,
                bridge_url_env_field_path.as_str(),
                account.bridge_url_env.as_deref(),
                bridge_url_field_path.as_str(),
            );

            let bridge_token_field_path = format!("imessage.accounts.{account_id}.bridge_token");
            let bridge_token_env_field_path = format!("{bridge_token_field_path}_env");
            validate_imessage_env_pointer(
                &mut issues,
                bridge_token_env_field_path.as_str(),
                account.bridge_token_env.as_deref(),
                bridge_token_field_path.as_str(),
            );
            validate_imessage_secret_ref_env_pointer(
                &mut issues,
                bridge_token_field_path.as_str(),
                account.bridge_token.as_ref(),
            );
        }
        issues
    }

    pub fn bridge_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.bridge_url.as_deref(), self.bridge_url_env.as_deref())
    }

    pub fn bridge_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.bridge_token.as_ref(), self.bridge_token_env.as_deref())
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
    ) -> CliResult<ResolvedImessageChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = ImessageChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            bridge_url: account_override
                .and_then(|account| account.bridge_url.clone())
                .or_else(|| self.bridge_url.clone()),
            bridge_url_env: account_override
                .and_then(|account| account.bridge_url_env.clone())
                .or_else(|| self.bridge_url_env.clone()),
            bridge_token: account_override
                .and_then(|account| account.bridge_token.clone())
                .or_else(|| self.bridge_token.clone()),
            bridge_token_env: account_override
                .and_then(|account| account.bridge_token_env.clone())
                .or_else(|| self.bridge_token_env.clone()),
            allowed_chat_ids: account_override
                .and_then(|account| account.allowed_chat_ids.clone())
                .unwrap_or_else(|| self.allowed_chat_ids.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedImessageChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            bridge_url: merged.bridge_url,
            bridge_url_env: merged.bridge_url_env,
            bridge_token: merged.bridge_token,
            bridge_token_env: merged.bridge_token_env,
            allowed_chat_ids: merged.allowed_chat_ids,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedImessageChannelConfig> {
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

impl SignalChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "signal",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_signal_env_pointer(
            &mut issues,
            "signal.account_env",
            self.signal_account_env.as_deref(),
            "signal.account",
        );
        validate_signal_env_pointer(
            &mut issues,
            "signal.service_url_env",
            self.service_url_env.as_deref(),
            "signal.service_url",
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let signal_account_field_path = format!("signal.accounts.{account_id}.account");
            let signal_account_env_field_path = format!("{signal_account_field_path}_env");
            validate_signal_env_pointer(
                &mut issues,
                signal_account_env_field_path.as_str(),
                account.signal_account_env.as_deref(),
                signal_account_field_path.as_str(),
            );
            let service_url_field_path = format!("signal.accounts.{account_id}.service_url");
            let service_url_env_field_path = format!("{service_url_field_path}_env");
            validate_signal_env_pointer(
                &mut issues,
                service_url_env_field_path.as_str(),
                account.service_url_env.as_deref(),
                service_url_field_path.as_str(),
            );
        }
        issues
    }

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
    ) -> CliResult<ResolvedSignalChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = SignalChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            signal_account: account_override
                .and_then(|account| account.signal_account.clone())
                .or_else(|| self.signal_account.clone()),
            signal_account_env: account_override
                .and_then(|account| account.signal_account_env.clone())
                .or_else(|| self.signal_account_env.clone()),
            service_url: account_override
                .and_then(|account| account.service_url.clone())
                .or_else(|| self.service_url.clone()),
            service_url_env: account_override
                .and_then(|account| account.service_url_env.clone())
                .or_else(|| self.service_url_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedSignalChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            signal_account: merged.signal_account,
            signal_account_env: merged.signal_account_env,
            service_url: merged.service_url,
            service_url_env: merged.service_url_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedSignalChannelConfig> {
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

        let signal_account = self.signal_account();
        let signal_account = signal_account
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(signal_account) = signal_account {
            let normalized_account_id = normalize_channel_account_id(signal_account);
            let account_id = format!("signal_{normalized_account_id}");
            let account_label = format!("signal:{signal_account}");
            return ChannelAccountIdentity {
                id: account_id,
                label: account_label,
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

impl TwitchChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "twitch",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_twitch_env_pointer(
            &mut issues,
            "twitch.access_token_env",
            self.access_token_env.as_deref(),
            "twitch.access_token",
        );
        validate_twitch_secret_ref_env_pointer(
            &mut issues,
            "twitch.access_token",
            self.access_token.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let access_token_field_path = format!("twitch.accounts.{account_id}.access_token");
            let access_token_env_field_path = format!("{access_token_field_path}_env");
            validate_twitch_env_pointer(
                &mut issues,
                access_token_env_field_path.as_str(),
                account.access_token_env.as_deref(),
                access_token_field_path.as_str(),
            );
            validate_twitch_secret_ref_env_pointer(
                &mut issues,
                access_token_field_path.as_str(),
                account.access_token.as_ref(),
            );
        }
        issues
    }

    pub fn access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.access_token.as_ref(), self.access_token_env.as_deref())
    }

    pub fn resolved_api_base_url(&self) -> String {
        self.api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_twitch_api_base_url)
    }

    pub fn resolved_oauth_base_url(&self) -> String {
        self.oauth_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_twitch_oauth_base_url)
    }

    pub fn configured_account_ids(&self) -> Vec<String> {
        let configured_ids = configured_account_ids(self.accounts.keys());
        if configured_ids.is_empty() {
            return vec![self.default_configured_account_id()];
        }
        configured_ids
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
    ) -> CliResult<ResolvedTwitchChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = TwitchChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            access_token: account_override
                .and_then(|account| account.access_token.clone())
                .or_else(|| self.access_token.clone()),
            access_token_env: account_override
                .and_then(|account| account.access_token_env.clone())
                .or_else(|| self.access_token_env.clone()),
            api_base_url: account_override
                .and_then(|account| account.api_base_url.clone())
                .or_else(|| self.api_base_url.clone()),
            oauth_base_url: account_override
                .and_then(|account| account.oauth_base_url.clone())
                .or_else(|| self.oauth_base_url.clone()),
            channel_names: account_override
                .and_then(|account| account.channel_names.clone())
                .unwrap_or_else(|| self.channel_names.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedTwitchChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            access_token: merged.access_token,
            access_token_env: merged.access_token_env,
            api_base_url: merged.api_base_url,
            oauth_base_url: merged.oauth_base_url,
            channel_names: merged.channel_names,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedTwitchChannelConfig> {
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

impl WhatsappChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "whatsapp",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_whatsapp_env_pointer(
            &mut issues,
            "whatsapp.access_token_env",
            self.access_token_env.as_deref(),
            "whatsapp.access_token",
        );
        validate_whatsapp_secret_ref_env_pointer(
            &mut issues,
            "whatsapp.access_token",
            self.access_token.as_ref(),
        );
        validate_whatsapp_env_pointer(
            &mut issues,
            "whatsapp.phone_number_id_env",
            self.phone_number_id_env.as_deref(),
            "whatsapp.phone_number_id",
        );
        validate_whatsapp_env_pointer(
            &mut issues,
            "whatsapp.verify_token_env",
            self.verify_token_env.as_deref(),
            "whatsapp.verify_token",
        );
        validate_whatsapp_secret_ref_env_pointer(
            &mut issues,
            "whatsapp.verify_token",
            self.verify_token.as_ref(),
        );
        validate_whatsapp_env_pointer(
            &mut issues,
            "whatsapp.app_secret_env",
            self.app_secret_env.as_deref(),
            "whatsapp.app_secret",
        );
        validate_whatsapp_secret_ref_env_pointer(
            &mut issues,
            "whatsapp.app_secret",
            self.app_secret.as_ref(),
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);
            let access_token_field_path = format!("whatsapp.accounts.{account_id}.access_token");
            let access_token_env_field_path = format!("{access_token_field_path}_env");
            validate_whatsapp_env_pointer(
                &mut issues,
                access_token_env_field_path.as_str(),
                account.access_token_env.as_deref(),
                access_token_field_path.as_str(),
            );
            validate_whatsapp_secret_ref_env_pointer(
                &mut issues,
                access_token_field_path.as_str(),
                account.access_token.as_ref(),
            );
            let phone_number_id_field_path =
                format!("whatsapp.accounts.{account_id}.phone_number_id");
            let phone_number_id_env_field_path = format!("{phone_number_id_field_path}_env");
            validate_whatsapp_env_pointer(
                &mut issues,
                phone_number_id_env_field_path.as_str(),
                account.phone_number_id_env.as_deref(),
                phone_number_id_field_path.as_str(),
            );
            let verify_token_field_path = format!("whatsapp.accounts.{account_id}.verify_token");
            let verify_token_env_field_path = format!("{verify_token_field_path}_env");
            validate_whatsapp_env_pointer(
                &mut issues,
                verify_token_env_field_path.as_str(),
                account.verify_token_env.as_deref(),
                verify_token_field_path.as_str(),
            );
            validate_whatsapp_secret_ref_env_pointer(
                &mut issues,
                verify_token_field_path.as_str(),
                account.verify_token.as_ref(),
            );
            let app_secret_field_path = format!("whatsapp.accounts.{account_id}.app_secret");
            let app_secret_env_field_path = format!("{app_secret_field_path}_env");
            validate_whatsapp_env_pointer(
                &mut issues,
                app_secret_env_field_path.as_str(),
                account.app_secret_env.as_deref(),
                app_secret_field_path.as_str(),
            );
            validate_whatsapp_secret_ref_env_pointer(
                &mut issues,
                app_secret_field_path.as_str(),
                account.app_secret.as_ref(),
            );
        }
        issues
    }

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
    ) -> CliResult<ResolvedWhatsappChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = WhatsappChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            access_token: account_override
                .and_then(|account| account.access_token.clone())
                .or_else(|| self.access_token.clone()),
            access_token_env: account_override
                .and_then(|account| account.access_token_env.clone())
                .or_else(|| self.access_token_env.clone()),
            phone_number_id: account_override
                .and_then(|account| account.phone_number_id.clone())
                .or_else(|| self.phone_number_id.clone()),
            phone_number_id_env: account_override
                .and_then(|account| account.phone_number_id_env.clone())
                .or_else(|| self.phone_number_id_env.clone()),
            verify_token: account_override
                .and_then(|account| account.verify_token.clone())
                .or_else(|| self.verify_token.clone()),
            verify_token_env: account_override
                .and_then(|account| account.verify_token_env.clone())
                .or_else(|| self.verify_token_env.clone()),
            app_secret: account_override
                .and_then(|account| account.app_secret.clone())
                .or_else(|| self.app_secret.clone()),
            app_secret_env: account_override
                .and_then(|account| account.app_secret_env.clone())
                .or_else(|| self.app_secret_env.clone()),
            api_base_url: account_override
                .and_then(|account| account.api_base_url.clone())
                .or_else(|| self.api_base_url.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedWhatsappChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            access_token: merged.access_token,
            access_token_env: merged.access_token_env,
            phone_number_id: merged.phone_number_id,
            phone_number_id_env: merged.phone_number_id_env,
            verify_token: merged.verify_token,
            verify_token_env: merged.verify_token_env,
            app_secret: merged.app_secret,
            app_secret_env: merged.app_secret_env,
            api_base_url: merged.api_base_url,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedWhatsappChannelConfig> {
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

        let phone_number_id = self.phone_number_id();
        let phone_number_id = phone_number_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(phone_number_id) = phone_number_id {
            let normalized_account_id = normalize_channel_account_id(phone_number_id);
            let account_id = format!("whatsapp_{normalized_account_id}");
            let account_label = format!("whatsapp:{phone_number_id}");
            return ChannelAccountIdentity {
                id: account_id,
                label: account_label,
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

fn default_telegram_base_url() -> String {
    "https://api.telegram.org".to_owned()
}

const fn default_telegram_timeout_seconds() -> u64 {
    15
}

const fn default_true() -> bool {
    true
}

fn default_feishu_receive_id_type() -> String {
    "chat_id".to_owned()
}

fn default_feishu_webhook_bind() -> String {
    "127.0.0.1:8080".to_owned()
}

fn default_feishu_webhook_path() -> String {
    "/feishu/events".to_owned()
}

const fn default_matrix_sync_timeout_seconds() -> u64 {
    30
}

fn default_wecom_websocket_url() -> String {
    "wss://openws.work.weixin.qq.com".to_owned()
}

const fn default_wecom_ping_interval_seconds() -> u64 {
    30
}

const fn default_wecom_reconnect_interval_seconds() -> u64 {
    5
}

fn default_discord_api_base_url() -> String {
    "https://discord.com/api/v10".to_owned()
}

fn default_discord_bot_token_env() -> Option<String> {
    Some(DISCORD_BOT_TOKEN_ENV.to_owned())
}

fn default_line_api_base_url() -> String {
    "https://api.line.me/v2/bot".to_owned()
}

fn default_email_smtp_username_env() -> Option<String> {
    Some(EMAIL_SMTP_USERNAME_ENV.to_owned())
}

fn default_email_smtp_password_env() -> Option<String> {
    Some(EMAIL_SMTP_PASSWORD_ENV.to_owned())
}

fn default_email_imap_username_env() -> Option<String> {
    Some(EMAIL_IMAP_USERNAME_ENV.to_owned())
}

fn default_email_imap_password_env() -> Option<String> {
    Some(EMAIL_IMAP_PASSWORD_ENV.to_owned())
}

fn default_webhook_endpoint_url_env() -> Option<String> {
    Some(WEBHOOK_ENDPOINT_URL_ENV.to_owned())
}

fn default_webhook_auth_token_env() -> Option<String> {
    Some(WEBHOOK_AUTH_TOKEN_ENV.to_owned())
}

fn default_webhook_signing_secret_env() -> Option<String> {
    Some(WEBHOOK_SIGNING_SECRET_ENV.to_owned())
}

fn default_webhook_auth_header_name() -> String {
    "Authorization".to_owned()
}

fn default_webhook_auth_token_prefix() -> String {
    "Bearer ".to_owned()
}

fn default_webhook_payload_text_field() -> String {
    "text".to_owned()
}

fn default_teams_webhook_url_env() -> Option<String> {
    Some(TEAMS_WEBHOOK_URL_ENV.to_owned())
}

fn default_teams_app_id_env() -> Option<String> {
    Some(TEAMS_APP_ID_ENV.to_owned())
}

fn default_teams_app_password_env() -> Option<String> {
    Some(TEAMS_APP_PASSWORD_ENV.to_owned())
}

fn default_teams_tenant_id_env() -> Option<String> {
    Some(TEAMS_TENANT_ID_ENV.to_owned())
}

fn default_irc_server_env() -> Option<String> {
    Some(IRC_SERVER_ENV.to_owned())
}

fn default_irc_nickname_env() -> Option<String> {
    Some(IRC_NICKNAME_ENV.to_owned())
}

fn default_irc_password_env() -> Option<String> {
    Some(IRC_PASSWORD_ENV.to_owned())
}

fn default_imessage_bridge_url_env() -> Option<String> {
    Some(IMESSAGE_BRIDGE_URL_ENV.to_owned())
}

fn default_imessage_bridge_token_env() -> Option<String> {
    Some(IMESSAGE_BRIDGE_TOKEN_ENV.to_owned())
}

fn default_signal_service_url() -> String {
    "http://127.0.0.1:8080".to_owned()
}

fn default_signal_account_env() -> Option<String> {
    Some(SIGNAL_ACCOUNT_ENV.to_owned())
}

fn default_signal_service_url_env() -> Option<String> {
    Some(SIGNAL_SERVICE_URL_ENV.to_owned())
}

fn default_twitch_api_base_url() -> String {
    "https://api.twitch.tv/helix".to_owned()
}

fn default_twitch_oauth_base_url() -> String {
    "https://id.twitch.tv/oauth2".to_owned()
}

fn default_twitch_access_token_env() -> Option<String> {
    Some(TWITCH_ACCESS_TOKEN_ENV.to_owned())
}

fn default_slack_api_base_url() -> String {
    "https://slack.com/api".to_owned()
}

fn default_slack_bot_token_env() -> Option<String> {
    Some(SLACK_BOT_TOKEN_ENV.to_owned())
}

fn default_whatsapp_api_base_url() -> String {
    "https://graph.facebook.com/v25.0".to_owned()
}

fn default_whatsapp_access_token_env() -> Option<String> {
    Some(WHATSAPP_ACCESS_TOKEN_ENV.to_owned())
}

fn default_whatsapp_phone_number_id_env() -> Option<String> {
    Some(WHATSAPP_PHONE_NUMBER_ID_ENV.to_owned())
}

fn default_whatsapp_verify_token_env() -> Option<String> {
    Some(WHATSAPP_VERIFY_TOKEN_ENV.to_owned())
}

fn default_whatsapp_app_secret_env() -> Option<String> {
    Some(WHATSAPP_APP_SECRET_ENV.to_owned())
}

fn default_system_prompt() -> String {
    render_default_system_prompt()
}

fn default_prompt_pack_id() -> Option<String> {
    Some(DEFAULT_PROMPT_PACK_ID.to_owned())
}

fn default_prompt_personality() -> Option<PromptPersonality> {
    Some(PromptPersonality::default())
}

fn default_exit_commands() -> Vec<String> {
    vec!["/exit".to_owned(), "/quit".to_owned()]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedConfiguredAccount {
    id: String,
    label: String,
    account_key: Option<String>,
}

fn default_channel_account_identity() -> ChannelAccountIdentity {
    ChannelAccountIdentity {
        id: "default".to_owned(),
        label: "default".to_owned(),
        source: ChannelAccountIdentitySource::Default,
    }
}

fn resolve_configured_account_identity(raw: Option<&str>) -> Option<(String, String)> {
    let label = raw.map(str::trim).filter(|value| !value.is_empty())?;
    if !label.chars().any(|value| value.is_ascii_alphanumeric()) {
        return None;
    }
    Some((normalize_channel_account_id(label), label.to_owned()))
}

fn resolve_telegram_bot_id_from_token(token: &str) -> Option<&str> {
    let bot_id = token.split(':').next()?.trim();
    if bot_id.is_empty() || !bot_id.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    Some(bot_id)
}

fn resolve_string_with_legacy_env(raw: Option<&str>, env_key: Option<&str>) -> Option<String> {
    let inline = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if inline.is_some() {
        return inline;
    }

    let env_name = env_key.map(str::trim).filter(|value| !value.is_empty())?;
    let env_value = std::env::var(env_name).ok()?;
    let trimmed_value = env_value.trim();
    if trimmed_value.is_empty() {
        return None;
    }
    Some(trimmed_value.to_owned())
}

pub(crate) fn parse_email_smtp_endpoint(raw: &str) -> CliResult<EmailSmtpEndpoint> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("email smtp_host is empty".to_owned());
    }

    if trimmed.contains("://") {
        let parsed_url = reqwest::Url::parse(trimmed)
            .map_err(|error| format!("email smtp_host url is invalid: {error}"))?;
        let scheme = parsed_url.scheme();
        if scheme != "smtp" && scheme != "smtps" {
            return Err(format!(
                "email smtp_host url must use smtp:// or smtps://, got {scheme}://"
            ));
        }

        let host = parsed_url
            .host_str()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if host.is_none() {
            return Err("email smtp_host url is missing a host".to_owned());
        }

        return Ok(EmailSmtpEndpoint::ConnectionUrl(trimmed.to_owned()));
    }

    if trimmed.chars().any(char::is_whitespace) {
        return Err("email smtp_host must not contain whitespace".to_owned());
    }
    if trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#') {
        return Err(
            "email smtp_host must be a bare host or a full smtp:// or smtps:// URL".to_owned(),
        );
    }
    if trimmed.contains(':') {
        return Err(
            "email smtp_host with an explicit port must use a full smtp:// or smtps:// URL"
                .to_owned(),
        );
    }

    Ok(EmailSmtpEndpoint::RelayHost(trimmed.to_owned()))
}

pub(crate) fn normalize_channel_account_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "default".to_owned();
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut last_was_separator = false;
    for value in trimmed.chars() {
        if value.is_ascii_alphanumeric() {
            normalized.push(value.to_ascii_lowercase());
            last_was_separator = false;
            continue;
        }
        if matches!(value, '_' | '-') {
            if !normalized.is_empty() && !last_was_separator {
                normalized.push(value);
                last_was_separator = true;
            }
            continue;
        }
        if !normalized.is_empty() && !last_was_separator {
            normalized.push('-');
            last_was_separator = true;
        }
    }

    while matches!(normalized.chars().last(), Some('-' | '_')) {
        normalized.pop();
    }

    if normalized.is_empty() {
        "default".to_owned()
    } else {
        normalized
    }
}

fn validate_email_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("imap_username_env") {
        EMAIL_IMAP_USERNAME_ENV
    } else if field_path.ends_with("imap_password_env") {
        EMAIL_IMAP_PASSWORD_ENV
    } else if field_path.ends_with("smtp_password_env") {
        EMAIL_SMTP_PASSWORD_ENV
    } else {
        EMAIL_SMTP_USERNAME_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_email_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("imap_username") {
        EMAIL_IMAP_USERNAME_ENV
    } else if field_path.ends_with("imap_password") {
        EMAIL_IMAP_PASSWORD_ENV
    } else if field_path.ends_with("smtp_password") {
        EMAIL_SMTP_PASSWORD_ENV
    } else {
        EMAIL_SMTP_USERNAME_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn build_email_invalid_value_issue(
    field_path: &str,
    invalid_reason: &str,
    suggested_fix: &str,
) -> ConfigValidationIssue {
    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert("invalid_reason".to_owned(), invalid_reason.to_owned());
    extra_message_variables.insert("suggested_fix".to_owned(), suggested_fix.to_owned());

    ConfigValidationIssue {
        severity: ConfigValidationSeverity::Error,
        code: ConfigValidationCode::InvalidValue,
        field_path: field_path.to_owned(),
        inline_field_path: field_path.to_owned(),
        example_env_name: String::new(),
        suggested_env_name: None,
        extra_message_variables,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IrcServerTransport {
    Plain,
    Tls,
}

impl IrcServerTransport {
    pub(crate) const fn default_port(self) -> u16 {
        match self {
            Self::Plain => 6667,
            Self::Tls => 6697,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IrcServerEndpoint {
    pub(crate) transport: IrcServerTransport,
    pub(crate) host: String,
    pub(crate) port: u16,
}

pub(crate) fn parse_irc_server_endpoint(raw: &str) -> CliResult<IrcServerEndpoint> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("irc server is empty".to_owned());
    }

    if trimmed.contains("://") {
        let url = reqwest::Url::parse(trimmed)
            .map_err(|error| format!("irc server url is invalid: {error}"))?;
        let transport = match url.scheme() {
            "irc" => IrcServerTransport::Plain,
            "ircs" => IrcServerTransport::Tls,
            scheme => {
                return Err(format!(
                    "unsupported irc server scheme `{scheme}`; expected `irc` or `ircs`"
                ));
            }
        };

        if !url.username().is_empty() || url.password().is_some() {
            return Err("irc server url must not include credentials".to_owned());
        }
        if url.query().is_some() {
            return Err("irc server url must not include a query".to_owned());
        }
        if url.fragment().is_some() {
            return Err("irc server url must not include a fragment".to_owned());
        }

        let path = url.path();
        if !(path.is_empty() || path == "/") {
            return Err("irc server url must not include a path".to_owned());
        }

        let host = url
            .host_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "irc server url is missing a host".to_owned())?;
        let port = url.port().unwrap_or_else(|| transport.default_port());
        return Ok(IrcServerEndpoint {
            transport,
            host: host.to_owned(),
            port,
        });
    }

    if trimmed.contains(':') {
        return Err(
            "bare `host:port` is not supported for irc server; use `irc://host:port` or `ircs://host:port`"
                .to_owned(),
        );
    }
    if trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#') {
        return Err(
            "bare irc server must be a hostname only; use `irc://` or `ircs://` for url-style values"
                .to_owned(),
        );
    }

    Ok(IrcServerEndpoint {
        transport: IrcServerTransport::Plain,
        host: trimmed.to_owned(),
        port: IrcServerTransport::Plain.default_port(),
    })
}

fn validate_irc_server_field(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    server: Option<String>,
) {
    let Some(server) = server else {
        return;
    };
    let parse_result = parse_irc_server_endpoint(server.as_str());
    let Err(invalid_reason) = parse_result else {
        return;
    };

    let suggested_fix =
        "use a bare host or an explicit `irc://host[:port]` / `ircs://host[:port]` endpoint";
    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert("invalid_reason".to_owned(), invalid_reason);
    extra_message_variables.insert("suggested_fix".to_owned(), suggested_fix.to_owned());
    issues.push(ConfigValidationIssue {
        severity: super::shared::ConfigValidationSeverity::Error,
        code: ConfigValidationCode::InvalidValue,
        field_path: field_path.to_owned(),
        inline_field_path: field_path.to_owned(),
        example_env_name: IRC_SERVER_ENV.to_owned(),
        suggested_env_name: None,
        extra_message_variables,
    });
}

fn validate_telegram_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: TELEGRAM_BOT_TOKEN_ENV,
            detect_telegram_token_shape: true,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_telegram_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: TELEGRAM_BOT_TOKEN_ENV,
            detect_telegram_token_shape: true,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_feishu_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("app_id_env") {
        FEISHU_APP_ID_ENV
    } else if field_path.ends_with("app_secret_env") {
        FEISHU_APP_SECRET_ENV
    } else if field_path.ends_with("verification_token_env") {
        FEISHU_VERIFICATION_TOKEN_ENV
    } else {
        FEISHU_ENCRYPT_KEY_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_feishu_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("app_id") {
        FEISHU_APP_ID_ENV
    } else if field_path.ends_with("app_secret") {
        FEISHU_APP_SECRET_ENV
    } else if field_path.ends_with("verification_token") {
        FEISHU_VERIFICATION_TOKEN_ENV
    } else {
        FEISHU_ENCRYPT_KEY_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_matrix_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: MATRIX_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_matrix_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: MATRIX_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_wecom_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("bot_id_env") {
        WECOM_BOT_ID_ENV
    } else {
        WECOM_SECRET_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_wecom_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("bot_id") {
        WECOM_BOT_ID_ENV
    } else {
        WECOM_SECRET_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_discord_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: DISCORD_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_line_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("channel_secret_env") {
        LINE_CHANNEL_SECRET_ENV
    } else {
        LINE_CHANNEL_ACCESS_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_line_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("channel_secret") {
        LINE_CHANNEL_SECRET_ENV
    } else {
        LINE_CHANNEL_ACCESS_TOKEN_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_dingtalk_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("secret_env") {
        DINGTALK_SECRET_ENV
    } else {
        DINGTALK_WEBHOOK_URL_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_dingtalk_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("secret") {
        DINGTALK_SECRET_ENV
    } else {
        DINGTALK_WEBHOOK_URL_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_webhook_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("endpoint_url_env") {
        WEBHOOK_ENDPOINT_URL_ENV
    } else if field_path.ends_with("signing_secret_env") {
        WEBHOOK_SIGNING_SECRET_ENV
    } else {
        WEBHOOK_AUTH_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_webhook_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("endpoint_url") {
        WEBHOOK_ENDPOINT_URL_ENV
    } else if field_path.ends_with("signing_secret") {
        WEBHOOK_SIGNING_SECRET_ENV
    } else {
        WEBHOOK_AUTH_TOKEN_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_discord_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: DISCORD_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_google_chat_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: GOOGLE_CHAT_WEBHOOK_URL_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_google_chat_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: GOOGLE_CHAT_WEBHOOK_URL_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_nextcloud_talk_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("shared_secret_env") {
        NEXTCLOUD_TALK_SHARED_SECRET_ENV
    } else {
        NEXTCLOUD_TALK_SERVER_URL_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_nextcloud_talk_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: NEXTCLOUD_TALK_SHARED_SECRET_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_synology_chat_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("incoming_url_env") {
        SYNOLOGY_CHAT_INCOMING_URL_ENV
    } else {
        SYNOLOGY_CHAT_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_synology_chat_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("incoming_url") {
        SYNOLOGY_CHAT_INCOMING_URL_ENV
    } else {
        SYNOLOGY_CHAT_TOKEN_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_teams_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("webhook_url_env") {
        TEAMS_WEBHOOK_URL_ENV
    } else if field_path.ends_with("app_password_env") {
        TEAMS_APP_PASSWORD_ENV
    } else if field_path.ends_with("tenant_id_env") {
        TEAMS_TENANT_ID_ENV
    } else {
        TEAMS_APP_ID_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_teams_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("webhook_url") {
        TEAMS_WEBHOOK_URL_ENV
    } else if field_path.ends_with("app_password") {
        TEAMS_APP_PASSWORD_ENV
    } else {
        TEAMS_APP_ID_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_imessage_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("bridge_url_env") {
        IMESSAGE_BRIDGE_URL_ENV
    } else {
        IMESSAGE_BRIDGE_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_imessage_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: IMESSAGE_BRIDGE_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_irc_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("server_env") {
        IRC_SERVER_ENV
    } else if field_path.ends_with("nickname_env") {
        IRC_NICKNAME_ENV
    } else {
        IRC_PASSWORD_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_irc_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: IRC_PASSWORD_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_signal_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("service_url_env") {
        SIGNAL_SERVICE_URL_ENV
    } else {
        SIGNAL_ACCOUNT_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_twitch_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: TWITCH_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_twitch_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: TWITCH_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_mattermost_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("server_url_env") {
        MATTERMOST_SERVER_URL_ENV
    } else {
        MATTERMOST_BOT_TOKEN_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_mattermost_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: MATTERMOST_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_slack_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: SLACK_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_slack_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: SLACK_BOT_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_whatsapp_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("access_token_env") {
        WHATSAPP_ACCESS_TOKEN_ENV
    } else if field_path.ends_with("phone_number_id_env") {
        WHATSAPP_PHONE_NUMBER_ID_ENV
    } else if field_path.ends_with("verify_token_env") {
        WHATSAPP_VERIFY_TOKEN_ENV
    } else {
        WHATSAPP_APP_SECRET_ENV
    };
    if let Err(issue) = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_whatsapp_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let example_env_name = if field_path.ends_with("access_token") {
        WHATSAPP_ACCESS_TOKEN_ENV
    } else if field_path.ends_with("verify_token") {
        WHATSAPP_VERIFY_TOKEN_ENV
    } else {
        WHATSAPP_APP_SECRET_ENV
    };
    if let Err(issue) = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    ) {
        issues.push(*issue);
    }
}

fn validate_channel_account_integrity<'a, I>(
    issues: &mut Vec<ConfigValidationIssue>,
    channel_key: &str,
    default_account: Option<&str>,
    keys: I,
) where
    I: IntoIterator<Item = &'a String>,
{
    let mut normalized_to_labels = BTreeMap::<String, Vec<String>>::new();
    for raw_key in keys {
        let label = raw_key.trim();
        if label.is_empty() {
            continue;
        }
        normalized_to_labels
            .entry(normalize_channel_account_id(label))
            .or_default()
            .push(label.to_owned());
    }

    for (normalized_account_id, labels) in &normalized_to_labels {
        if labels.len() < 2 {
            continue;
        }
        let mut extra_message_variables = BTreeMap::new();
        extra_message_variables.insert(
            "normalized_account_id".to_owned(),
            normalized_account_id.clone(),
        );
        extra_message_variables.insert("raw_account_labels".to_owned(), labels.join(", "));
        issues.push(ConfigValidationIssue {
            severity: super::shared::ConfigValidationSeverity::Error,
            code: ConfigValidationCode::DuplicateChannelAccountId,
            field_path: format!("{channel_key}.accounts"),
            inline_field_path: format!("{channel_key}.accounts.{normalized_account_id}"),
            example_env_name: String::new(),
            suggested_env_name: None,
            extra_message_variables,
        });
    }

    if normalized_to_labels.is_empty() {
        return;
    }

    let Some(requested_default_account) = default_account
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let normalized_default_account = normalize_channel_account_id(requested_default_account);
    if normalized_to_labels.contains_key(&normalized_default_account) {
        return;
    }

    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert(
        "requested_account_id".to_owned(),
        normalized_default_account,
    );
    extra_message_variables.insert(
        "configured_account_ids".to_owned(),
        normalized_to_labels
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    );
    issues.push(ConfigValidationIssue {
        severity: super::shared::ConfigValidationSeverity::Error,
        code: ConfigValidationCode::UnknownChannelDefaultAccount,
        field_path: format!("{channel_key}.default_account"),
        inline_field_path: format!("{channel_key}.accounts"),
        example_env_name: String::new(),
        suggested_env_name: None,
        extra_message_variables,
    });
}

fn configured_account_ids<'a, I>(keys: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut ids = keys
        .into_iter()
        .map(|value| normalize_channel_account_id(value))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn normalize_optional_account_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_channel_account_id)
}

fn resolve_default_configured_account_selection_from_ids(
    ids: &[String],
    preferred: Option<&str>,
    fallback: &str,
) -> ChannelDefaultAccountSelection {
    if let Some(preferred) = normalize_optional_account_id(preferred)
        && !ids.is_empty()
        && ids.iter().any(|value| value == &preferred)
    {
        return ChannelDefaultAccountSelection {
            id: preferred,
            source: ChannelDefaultAccountSelectionSource::ExplicitDefault,
        };
    }
    if ids.is_empty() {
        return ChannelDefaultAccountSelection {
            id: normalize_channel_account_id(fallback),
            source: ChannelDefaultAccountSelectionSource::RuntimeIdentity,
        };
    }
    if ids.iter().any(|value| value == "default") {
        return ChannelDefaultAccountSelection {
            id: "default".to_owned(),
            source: ChannelDefaultAccountSelectionSource::MappedDefault,
        };
    }
    ChannelDefaultAccountSelection {
        id: ids
            .first()
            .cloned()
            .unwrap_or_else(|| normalize_channel_account_id(fallback)),
        source: ChannelDefaultAccountSelectionSource::Fallback,
    }
}

fn resolve_default_configured_account_selection<'a, I>(
    keys: I,
    preferred: Option<&str>,
    fallback: &str,
) -> ChannelDefaultAccountSelection
where
    I: IntoIterator<Item = &'a String>,
{
    let ids = configured_account_ids(keys);
    resolve_default_configured_account_selection_from_ids(ids.as_slice(), preferred, fallback)
}

fn resolve_channel_account_route<'a, I>(
    keys: I,
    preferred: Option<&str>,
    fallback: &str,
    requested_account_id: Option<&str>,
    selected_configured_account_id: &str,
) -> ChannelResolvedAccountRoute
where
    I: IntoIterator<Item = &'a String>,
{
    let ids = configured_account_ids(keys);
    let default_selection =
        resolve_default_configured_account_selection_from_ids(ids.as_slice(), preferred, fallback);
    ChannelResolvedAccountRoute {
        requested_account_id: normalize_optional_account_id(requested_account_id),
        configured_account_count: ids.len(),
        selected_configured_account_id: normalize_channel_account_id(
            selected_configured_account_id,
        ),
        default_account_source: default_selection.source,
    }
}

fn resolve_channel_acp_config(
    base: &ChannelAcpConfig,
    account_override: Option<&ChannelAcpConfig>,
) -> ChannelAcpConfig {
    account_override.cloned().unwrap_or_else(|| base.clone())
}

fn resolve_account_for_session_account_id<R>(
    session_account_id: Option<&str>,
    resolve_direct: impl FnOnce() -> CliResult<R>,
    configured_ids: impl FnOnce() -> Vec<String>,
    resolve_configured: impl Fn(&str) -> CliResult<R>,
    runtime_account_id: impl Fn(&R) -> &str,
) -> CliResult<R> {
    let Some(requested) = normalize_optional_account_id(session_account_id) else {
        return resolve_direct();
    };

    match resolve_direct() {
        Ok(resolved) => Ok(resolved),
        Err(original_error) => {
            for configured_id in configured_ids() {
                let resolved = resolve_configured(configured_id.as_str())?;
                if normalize_channel_account_id(runtime_account_id(&resolved)) == requested {
                    return Ok(resolved);
                }
            }
            Err(original_error)
        }
    }
}

fn resolve_configured_account_selection<'a, I>(
    keys: I,
    requested_account_id: Option<&str>,
    preferred_default_account_id: Option<&str>,
    fallback_id: &str,
) -> CliResult<ResolvedConfiguredAccount>
where
    I: IntoIterator<Item = &'a String>,
{
    let entries = keys
        .into_iter()
        .filter_map(|value| {
            let raw_key = value.to_owned();
            let label = value.trim();
            if label.is_empty() {
                return None;
            }
            Some((
                normalize_channel_account_id(label),
                label.to_owned(),
                raw_key,
            ))
        })
        .collect::<Vec<_>>();
    let configured_ids = entries
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect::<Vec<_>>();

    if let Some(requested) = requested_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_channel_account_id)
    {
        if entries.is_empty() {
            return Ok(ResolvedConfiguredAccount {
                label: requested.clone(),
                id: requested,
                account_key: None,
            });
        }
        let Some((id, label, raw_key)) = entries.iter().find(|(id, _, _)| *id == requested) else {
            return Err(format!(
                "requested account `{requested}` is not configured (configured accounts: {})",
                configured_ids.join(", ")
            ));
        };
        return Ok(ResolvedConfiguredAccount {
            id: id.clone(),
            label: label.clone(),
            account_key: Some(raw_key.clone()),
        });
    }

    let default_id = resolve_default_configured_account_selection(
        entries.iter().map(|(_, _, raw_key)| raw_key),
        preferred_default_account_id,
        fallback_id,
    )
    .id;
    if let Some((id, label, raw_key)) = entries.iter().find(|(id, _, _)| *id == default_id) {
        return Ok(ResolvedConfiguredAccount {
            id: id.clone(),
            label: label.clone(),
            account_key: Some(raw_key.clone()),
        });
    }

    Ok(ResolvedConfiguredAccount {
        id: default_id.clone(),
        label: default_id,
        account_key: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn telegram_account_identity_prefers_explicit_account_id() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "account_id": "Ops-Bot",
            "bot_token": "123456:token-value"
        }))
        .expect("deserialize telegram config");

        let identity = config.resolved_account_identity();
        assert_eq!(identity.id, "ops-bot");
        assert_eq!(identity.label, "Ops-Bot");
    }

    #[test]
    fn telegram_account_identity_derives_from_bot_token_prefix() {
        let config = TelegramChannelConfig {
            bot_token: Some(loongclaw_contracts::SecretRef::Inline(
                "987654:token-value".to_owned(),
            )),
            bot_token_env: None,
            ..TelegramChannelConfig::default()
        };

        let identity = config.resolved_account_identity();
        assert_eq!(identity.id, "bot_987654");
        assert_eq!(identity.label, "bot:987654");
    }

    #[test]
    fn feishu_account_identity_prefers_explicit_account_id() {
        let config: FeishuChannelConfig = serde_json::from_value(json!({
            "account_id": "Customer-Support",
            "app_id": "cli_a1b2c3",
            "domain": "lark"
        }))
        .expect("deserialize feishu config");

        let identity = config.resolved_account_identity();
        assert_eq!(identity.id, "customer-support");
        assert_eq!(identity.label, "Customer-Support");
    }

    #[test]
    fn feishu_account_identity_derives_from_domain_and_app_id() {
        let config = FeishuChannelConfig {
            app_id: Some(loongclaw_contracts::SecretRef::Inline(
                "cli_a1b2c3".to_owned(),
            )),
            app_id_env: None,
            domain: FeishuDomain::Lark,
            ..FeishuChannelConfig::default()
        };

        let identity = config.resolved_account_identity();
        assert_eq!(identity.id, "lark_cli_a1b2c3");
        assert_eq!(identity.label, "lark:cli_a1b2c3");
    }

    #[test]
    fn configured_account_identity_rejects_non_alphanumeric_labels() {
        assert_eq!(resolve_configured_account_identity(Some(" !!! ")), None);
    }

    #[test]
    fn telegram_multi_account_resolution_merges_base_and_account_overrides() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bot_token_env": "BASE_TELEGRAM_TOKEN",
            "polling_timeout_s": 25,
            "allowed_chat_ids": [1001],
            "acp": {
                "bootstrap_mcp_servers": ["filesystem"],
                "working_directory": " /workspace/base "
            },
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token_env": "WORK_TELEGRAM_TOKEN",
                    "allowed_chat_ids": [2002],
                    "acp": {
                        "bootstrap_mcp_servers": ["search"],
                        "working_directory": "/workspace/work-bot"
                    }
                },
                "Personal": {
                    "enabled": false,
                    "bot_token_env": "PERSONAL_TELEGRAM_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram multi-account config");

        assert_eq!(
            config.configured_account_ids(),
            vec!["personal", "work-bot"]
        );
        assert_eq!(config.default_configured_account_id(), "work-bot");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default telegram account");
        assert_eq!(resolved.configured_account_id, "work-bot");
        assert_eq!(resolved.account.id, "ops-bot");
        assert_eq!(resolved.account.label, "Ops-Bot");
        assert_eq!(
            resolved.bot_token_env.as_deref(),
            Some("WORK_TELEGRAM_TOKEN")
        );
        assert_eq!(resolved.allowed_chat_ids, vec![2002]);
        assert_eq!(
            resolved.acp.bootstrap_mcp_servers,
            vec!["search".to_owned()]
        );
        assert_eq!(
            resolved.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/work-bot"))
        );
        assert_eq!(resolved.polling_timeout_s, 25);

        let disabled = config
            .resolve_account(Some("Personal"))
            .expect("resolve explicit telegram account");
        assert_eq!(disabled.configured_account_id, "personal");
        assert!(!disabled.enabled);
        assert_eq!(disabled.allowed_chat_ids, vec![1001]);
        assert_eq!(
            disabled.acp.bootstrap_mcp_servers,
            vec!["filesystem".to_owned()]
        );
        assert_eq!(
            disabled.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/base"))
        );
    }

    #[test]
    fn telegram_resolve_account_for_session_account_id_matches_runtime_identity() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token_env": "WORK_TELEGRAM_TOKEN",
                    "acp": {
                        "bootstrap_mcp_servers": ["search"],
                        "working_directory": "/workspace/work-bot"
                    }
                }
            }
        }))
        .expect("deserialize telegram config");

        let resolved = config
            .resolve_account_for_session_account_id(Some("ops-bot"))
            .expect("resolve telegram runtime account identity");
        assert_eq!(resolved.configured_account_id, "work-bot");
        assert_eq!(resolved.account.id, "ops-bot");
        assert_eq!(
            resolved.acp.bootstrap_mcp_servers,
            vec!["search".to_owned()]
        );
        assert_eq!(
            resolved.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/work-bot"))
        );
    }

    #[test]
    fn telegram_default_account_selection_source_tracks_explicit_default() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "bot_token_env": "WORK_TELEGRAM_TOKEN"
                },
                "Personal": {
                    "bot_token_env": "PERSONAL_TELEGRAM_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram config");

        let selection = config.default_configured_account_selection();
        assert_eq!(selection.id, "work-bot");
        assert_eq!(
            selection.source,
            ChannelDefaultAccountSelectionSource::ExplicitDefault
        );
    }

    #[test]
    fn telegram_default_account_selection_source_tracks_mapped_default() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "accounts": {
                "default": {
                    "bot_token_env": "DEFAULT_TELEGRAM_TOKEN"
                },
                "Work": {
                    "bot_token_env": "WORK_TELEGRAM_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram config");

        let selection = config.default_configured_account_selection();
        assert_eq!(selection.id, "default");
        assert_eq!(
            selection.source,
            ChannelDefaultAccountSelectionSource::MappedDefault
        );
    }

    #[test]
    fn telegram_default_account_selection_source_tracks_sorted_fallback() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "accounts": {
                "Work": {
                    "bot_token_env": "WORK_TELEGRAM_TOKEN"
                },
                "Alerts": {
                    "bot_token_env": "ALERTS_TELEGRAM_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram config");

        let selection = config.default_configured_account_selection();
        assert_eq!(selection.id, "alerts");
        assert_eq!(
            selection.source,
            ChannelDefaultAccountSelectionSource::Fallback
        );
    }

    #[test]
    fn telegram_default_account_does_not_override_single_account_fallback_identity() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "default_account": "Work Bot",
            "bot_token": "123456:token-value",
            "allowed_chat_ids": [1001]
        }))
        .expect("deserialize single-account telegram config");

        let selection = config.default_configured_account_selection();
        assert_eq!(selection.id, "bot_123456");
        assert_eq!(
            selection.source,
            ChannelDefaultAccountSelectionSource::RuntimeIdentity
        );

        let resolved = config
            .resolve_account(None)
            .expect("resolve single-account telegram config");
        assert_eq!(resolved.configured_account_id, "bot_123456");
        assert_eq!(resolved.account.id, "bot_123456");
    }

    #[test]
    fn telegram_resolved_account_route_flags_implicit_multi_account_fallback() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "accounts": {
                "Work": {
                    "bot_token_env": "WORK_TELEGRAM_TOKEN"
                },
                "Alerts": {
                    "bot_token_env": "ALERTS_TELEGRAM_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default telegram account");
        let route = config.resolved_account_route(None, resolved.configured_account_id.as_str());

        assert!(route.selected_by_default());
        assert_eq!(route.selected_configured_account_id, "alerts");
        assert_eq!(route.configured_account_count, 2);
        assert_eq!(
            route.default_account_source,
            ChannelDefaultAccountSelectionSource::Fallback
        );
        assert!(route.uses_implicit_fallback_default());
    }

    #[test]
    fn telegram_resolved_account_route_does_not_flag_explicit_account_request() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "accounts": {
                "Work": {
                    "bot_token_env": "WORK_TELEGRAM_TOKEN"
                },
                "Alerts": {
                    "bot_token_env": "ALERTS_TELEGRAM_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram config");

        let resolved = config
            .resolve_account(Some("Work"))
            .expect("resolve explicit telegram account");
        let route =
            config.resolved_account_route(Some("Work"), resolved.configured_account_id.as_str());

        assert!(!route.selected_by_default());
        assert_eq!(route.requested_account_id.as_deref(), Some("work"));
        assert_eq!(route.selected_configured_account_id, "work");
        assert!(!route.uses_implicit_fallback_default());
    }

    #[test]
    fn feishu_multi_account_resolution_merges_base_and_account_overrides() {
        let config: FeishuChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "mode": "webhook",
            "app_id_env": "BASE_FEISHU_APP_ID",
            "app_secret_env": "BASE_FEISHU_APP_SECRET",
            "verification_token_env": "BASE_FEISHU_VERIFY",
            "encrypt_key_env": "BASE_FEISHU_ENCRYPT",
            "receive_id_type": "chat_id",
            "webhook_bind": "127.0.0.1:8080",
            "webhook_path": "/feishu/events",
            "allowed_chat_ids": ["oc_base"],
            "acp": {
                "bootstrap_mcp_servers": ["filesystem"],
                "working_directory": " /workspace/base "
            },
            "default_account": "Lark Prod",
            "accounts": {
                "Lark Prod": {
                    "domain": "lark",
                    "app_id": "cli_lark_123",
                    "app_secret": "secret",
                    "verification_token": "verify",
                    "encrypt_key": "encrypt",
                    "allowed_chat_ids": ["oc_lark"],
                    "acp": {
                        "bootstrap_mcp_servers": ["search"],
                        "working_directory": "/workspace/lark-prod"
                    }
                },
                "Feishu Backup": {
                    "enabled": false,
                    "app_id": "cli_backup_456",
                    "app_secret": "secret"
                }
            }
        }))
        .expect("deserialize feishu multi-account config");

        assert_eq!(
            config.configured_account_ids(),
            vec!["feishu-backup", "lark-prod"]
        );
        assert_eq!(config.default_configured_account_id(), "lark-prod");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default feishu account");
        assert_eq!(resolved.configured_account_id, "lark-prod");
        assert_eq!(resolved.domain, FeishuDomain::Lark);
        assert_eq!(resolved.account.id, "lark_cli_lark_123");
        assert_eq!(resolved.account.label, "lark:cli_lark_123");
        assert_eq!(resolved.allowed_chat_ids, vec!["oc_lark".to_owned()]);
        assert_eq!(
            resolved.acp.bootstrap_mcp_servers,
            vec!["search".to_owned()]
        );
        assert_eq!(
            resolved.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/lark-prod"))
        );
        assert_eq!(resolved.receive_id_type, "chat_id");
        assert_eq!(resolved.mode, FeishuChannelServeMode::Webhook);
        assert_eq!(resolved.resolved_base_url(), "https://open.larksuite.com");

        let disabled = config
            .resolve_account(Some("Feishu Backup"))
            .expect("resolve explicit feishu account");
        assert_eq!(disabled.configured_account_id, "feishu-backup");
        assert!(!disabled.enabled);
        assert_eq!(disabled.allowed_chat_ids, vec!["oc_base".to_owned()]);
        assert_eq!(
            disabled.acp.bootstrap_mcp_servers,
            vec!["filesystem".to_owned()]
        );
        assert_eq!(
            disabled.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/base"))
        );
    }

    #[test]
    fn feishu_mode_defaults_to_webhook_when_not_configured() {
        let config: FeishuChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "app_id": "cli_a1b2c3",
            "app_secret": "secret"
        }))
        .expect("deserialize feishu config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default feishu account");

        assert_eq!(resolved.mode, FeishuChannelServeMode::Webhook);
    }

    #[test]
    fn feishu_multi_account_resolution_allows_websocket_mode_override() {
        let config: FeishuChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "mode": "webhook",
            "app_id": "cli_base",
            "app_secret": "base-secret",
            "allowed_chat_ids": ["oc_base"],
            "accounts": {
                "Long Connection": {
                    "mode": "websocket",
                    "app_id": "cli_ws",
                    "app_secret": "ws-secret"
                }
            }
        }))
        .expect("deserialize feishu config");

        let resolved = config
            .resolve_account(Some("Long Connection"))
            .expect("resolve websocket feishu account");

        assert_eq!(resolved.mode, FeishuChannelServeMode::Websocket);
        assert_eq!(resolved.allowed_chat_ids, vec!["oc_base".to_owned()]);
    }

    #[test]
    fn feishu_resolve_account_for_session_account_id_matches_runtime_identity() {
        let config: FeishuChannelConfig = serde_json::from_value(json!({
            "default_account": "Lark Prod",
            "accounts": {
                "Lark Prod": {
                    "domain": "lark",
                    "app_id": "cli_lark_123",
                    "app_secret": "secret",
                    "acp": {
                        "bootstrap_mcp_servers": ["search"],
                        "working_directory": "/workspace/lark-prod"
                    }
                }
            }
        }))
        .expect("deserialize feishu config");

        let resolved = config
            .resolve_account_for_session_account_id(Some("lark_cli_lark_123"))
            .expect("resolve feishu runtime account identity");
        assert_eq!(resolved.configured_account_id, "lark-prod");
        assert_eq!(resolved.account.id, "lark_cli_lark_123");
        assert_eq!(
            resolved.acp.bootstrap_mcp_servers,
            vec!["search".to_owned()]
        );
        assert_eq!(
            resolved.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/lark-prod"))
        );
    }

    #[test]
    fn feishu_resolved_account_route_tracks_explicit_default_without_fallback_warning() {
        let config: FeishuChannelConfig = serde_json::from_value(json!({
            "default_account": "Lark Prod",
            "accounts": {
                "Lark Prod": {
                    "domain": "lark",
                    "app_id": "cli_lark_123",
                    "app_secret": "secret"
                },
                "Feishu Backup": {
                    "app_id": "cli_backup_456",
                    "app_secret": "secret"
                }
            }
        }))
        .expect("deserialize feishu config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default feishu account");
        let route = config.resolved_account_route(None, resolved.configured_account_id.as_str());

        assert!(route.selected_by_default());
        assert_eq!(route.selected_configured_account_id, "lark-prod");
        assert_eq!(
            route.default_account_source,
            ChannelDefaultAccountSelectionSource::ExplicitDefault
        );
        assert!(!route.uses_implicit_fallback_default());
    }

    #[test]
    fn wecom_account_identity_prefers_explicit_account_id() {
        let config: WecomChannelConfig = serde_json::from_value(json!({
            "account_id": "Ops-Bot",
            "bot_id": "bot_123"
        }))
        .expect("deserialize wecom config");

        let identity = config.resolved_account_identity();
        assert_eq!(identity.id, "ops-bot");
        assert_eq!(identity.label, "Ops-Bot");
    }

    #[test]
    fn wecom_account_identity_derives_from_bot_id() {
        let config = WecomChannelConfig {
            bot_id: Some(loongclaw_contracts::SecretRef::Inline("bot_123".to_owned())),
            bot_id_env: None,
            ..WecomChannelConfig::default()
        };

        let identity = config.resolved_account_identity();
        assert_eq!(identity.id, "wecom_bot_123");
        assert_eq!(identity.label, "wecom:bot_123");
    }

    #[test]
    fn wecom_multi_account_resolution_merges_base_and_account_overrides() {
        let config: WecomChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bot_id_env": "BASE_WECOM_BOT_ID",
            "secret_env": "BASE_WECOM_SECRET",
            "ping_interval_s": 45,
            "reconnect_interval_s": 12,
            "allowed_conversation_ids": ["group_base"],
            "acp": {
                "bootstrap_mcp_servers": ["filesystem"],
                "working_directory": " /workspace/base "
            },
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "account_id": "WeCom-Work",
                    "bot_id": "bot_work",
                    "secret": "secret-work",
                    "allowed_conversation_ids": ["group_work"],
                    "acp": {
                        "bootstrap_mcp_servers": ["search"],
                        "working_directory": "/workspace/work-bot"
                    }
                },
                "Alerts": {
                    "enabled": false,
                    "bot_id": "bot_alerts",
                    "secret": "secret-alerts"
                }
            }
        }))
        .expect("deserialize wecom multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["alerts", "work-bot"]);
        assert_eq!(config.default_configured_account_id(), "work-bot");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default wecom account");
        assert_eq!(resolved.configured_account_id, "work-bot");
        assert_eq!(resolved.account.id, "wecom-work");
        assert_eq!(resolved.account.label, "WeCom-Work");
        assert_eq!(
            resolved.allowed_conversation_ids,
            vec!["group_work".to_owned()]
        );
        assert_eq!(resolved.ping_interval_s, 45);
        assert_eq!(resolved.reconnect_interval_s, 12);
        assert_eq!(
            resolved.acp.bootstrap_mcp_servers,
            vec!["search".to_owned()]
        );
        assert_eq!(
            resolved.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/work-bot"))
        );
        assert_eq!(
            resolved.resolved_websocket_url(),
            "wss://openws.work.weixin.qq.com"
        );

        let disabled = config
            .resolve_account(Some("Alerts"))
            .expect("resolve explicit wecom account");
        assert_eq!(disabled.configured_account_id, "alerts");
        assert!(!disabled.enabled);
        assert_eq!(
            disabled.allowed_conversation_ids,
            vec!["group_base".to_owned()]
        );
        assert_eq!(
            disabled.acp.bootstrap_mcp_servers,
            vec!["filesystem".to_owned()]
        );
        assert_eq!(
            disabled.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/base"))
        );
    }

    #[test]
    fn wecom_resolve_account_for_session_account_id_matches_runtime_identity() {
        let config: WecomChannelConfig = serde_json::from_value(json!({
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "account_id": "wecom-shared",
                    "bot_id": "bot_work",
                    "secret": "secret-work",
                    "acp": {
                        "bootstrap_mcp_servers": ["search"],
                        "working_directory": "/workspace/work-bot"
                    }
                }
            }
        }))
        .expect("deserialize wecom config");

        let resolved = config
            .resolve_account_for_session_account_id(Some("wecom-shared"))
            .expect("resolve wecom runtime account identity");
        assert_eq!(resolved.configured_account_id, "work-bot");
        assert_eq!(resolved.account.id, "wecom-shared");
        assert_eq!(
            resolved.acp.bootstrap_mcp_servers,
            vec!["search".to_owned()]
        );
        assert_eq!(
            resolved.acp.resolved_working_directory(),
            Some(std::path::PathBuf::from("/workspace/work-bot"))
        );
    }

    #[test]
    fn line_resolves_account_credentials_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_LINE_CHANNEL_ACCESS_TOKEN", "line-access-token");
        env.set("TEST_LINE_CHANNEL_SECRET", "line-channel-secret");

        let config_value = json!({
            "enabled": true,
            "account_id": "Line-Primary",
            "channel_access_token_env": "TEST_LINE_CHANNEL_ACCESS_TOKEN",
            "channel_secret_env": "TEST_LINE_CHANNEL_SECRET"
        });
        let config: LineChannelConfig =
            serde_json::from_value(config_value).expect("deserialize line config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default line account");
        let channel_access_token = resolved.channel_access_token();
        let channel_secret = resolved.channel_secret();

        assert_eq!(resolved.configured_account_id, "line-primary");
        assert_eq!(resolved.account.id, "line-primary");
        assert_eq!(resolved.account.label, "Line-Primary");
        assert_eq!(channel_access_token.as_deref(), Some("line-access-token"));
        assert_eq!(channel_secret.as_deref(), Some("line-channel-secret"));
        assert_eq!(
            resolved.resolved_api_base_url(),
            "https://api.line.me/v2/bot"
        );
    }

    #[test]
    fn line_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Line-Shared",
            "channel_access_token": "base-line-token",
            "default_account": "Marketing",
            "accounts": {
                "Marketing": {
                    "account_id": "Line-Marketing",
                    "channel_access_token": "marketing-line-token"
                },
                "Backup": {
                    "enabled": false,
                    "channel_secret": "backup-secret",
                    "api_base_url": "https://line.example.test/v2/bot"
                }
            }
        });
        let config: LineChannelConfig =
            serde_json::from_value(config_value).expect("deserialize line multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "marketing"]);
        assert_eq!(config.default_configured_account_id(), "marketing");

        let marketing = config
            .resolve_account(None)
            .expect("resolve default line account");
        let marketing_channel_access_token = marketing.channel_access_token();

        assert_eq!(marketing.configured_account_id, "marketing");
        assert_eq!(marketing.account.id, "line-marketing");
        assert_eq!(marketing.account.label, "Line-Marketing");
        assert_eq!(
            marketing_channel_access_token.as_deref(),
            Some("marketing-line-token")
        );
        assert_eq!(marketing.channel_secret(), None);
        assert_eq!(
            marketing.resolved_api_base_url(),
            "https://api.line.me/v2/bot"
        );

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit line account");
        let backup_channel_access_token = backup.channel_access_token();
        let backup_channel_secret = backup.channel_secret();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "line-shared");
        assert_eq!(backup.account.label, "Line-Shared");
        assert_eq!(
            backup_channel_access_token.as_deref(),
            Some("base-line-token")
        );
        assert_eq!(backup_channel_secret.as_deref(), Some("backup-secret"));
        assert_eq!(
            backup.resolved_api_base_url(),
            "https://line.example.test/v2/bot"
        );
    }

    #[test]
    fn dingtalk_resolves_webhook_url_and_secret_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_DINGTALK_WEBHOOK_URL",
            "https://oapi.dingtalk.com/robot/send?access_token=test-token",
        );
        env.set("TEST_DINGTALK_SECRET", "ding-secret");

        let config_value = json!({
            "enabled": true,
            "account_id": "DingTalk-Primary",
            "webhook_url_env": "TEST_DINGTALK_WEBHOOK_URL",
            "secret_env": "TEST_DINGTALK_SECRET"
        });
        let config: DingtalkChannelConfig =
            serde_json::from_value(config_value).expect("deserialize dingtalk config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default dingtalk account");
        let webhook_url = resolved.webhook_url();
        let secret = resolved.secret();

        assert_eq!(resolved.configured_account_id, "dingtalk-primary");
        assert_eq!(resolved.account.id, "dingtalk-primary");
        assert_eq!(resolved.account.label, "DingTalk-Primary");
        assert_eq!(
            webhook_url.as_deref(),
            Some("https://oapi.dingtalk.com/robot/send?access_token=test-token")
        );
        assert_eq!(secret.as_deref(), Some("ding-secret"));
    }

    #[test]
    fn dingtalk_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "DingTalk-Shared",
            "webhook_url": "https://oapi.dingtalk.com/robot/send?access_token=base-token",
            "secret": "base-secret",
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "DingTalk-Ops",
                    "webhook_url": "https://oapi.dingtalk.com/robot/send?access_token=ops-token"
                },
                "Backup": {
                    "enabled": false,
                    "secret": "backup-secret"
                }
            }
        });
        let config: DingtalkChannelConfig = serde_json::from_value(config_value)
            .expect("deserialize dingtalk multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default dingtalk account");
        let ops_webhook_url = ops.webhook_url();
        let ops_secret = ops.secret();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "dingtalk-ops");
        assert_eq!(ops.account.label, "DingTalk-Ops");
        assert_eq!(
            ops_webhook_url.as_deref(),
            Some("https://oapi.dingtalk.com/robot/send?access_token=ops-token")
        );
        assert_eq!(ops_secret.as_deref(), Some("base-secret"));

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit dingtalk account");
        let backup_webhook_url = backup.webhook_url();
        let backup_secret = backup.secret();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "dingtalk-shared");
        assert_eq!(backup.account.label, "DingTalk-Shared");
        assert_eq!(
            backup_webhook_url.as_deref(),
            Some("https://oapi.dingtalk.com/robot/send?access_token=base-token")
        );
        assert_eq!(backup_secret.as_deref(), Some("backup-secret"));
    }

    #[test]
    fn webhook_resolves_endpoint_and_secrets_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_WEBHOOK_ENDPOINT_URL",
            "https://hooks.example.test/ingest?token=secret",
        );
        env.set("TEST_WEBHOOK_AUTH_TOKEN", "token-123");
        env.set("TEST_WEBHOOK_SIGNING_SECRET", "signing-secret-123");

        let config_value = json!({
            "enabled": true,
            "account_id": "Webhook-Ops",
            "endpoint_url_env": "TEST_WEBHOOK_ENDPOINT_URL",
            "auth_token_env": "TEST_WEBHOOK_AUTH_TOKEN",
            "signing_secret_env": "TEST_WEBHOOK_SIGNING_SECRET"
        });
        let config: WebhookChannelConfig =
            serde_json::from_value(config_value).expect("deserialize webhook config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default webhook account");
        let endpoint_url = resolved.endpoint_url();
        let auth_token = resolved.auth_token();
        let signing_secret = resolved.signing_secret();

        assert_eq!(resolved.configured_account_id, "webhook-ops");
        assert_eq!(resolved.account.id, "webhook-ops");
        assert_eq!(resolved.account.label, "Webhook-Ops");
        assert_eq!(
            endpoint_url.as_deref(),
            Some("https://hooks.example.test/ingest?token=secret")
        );
        assert_eq!(auth_token.as_deref(), Some("token-123"));
        assert_eq!(signing_secret.as_deref(), Some("signing-secret-123"));
        assert_eq!(resolved.auth_header_name, "Authorization");
        assert_eq!(resolved.auth_token_prefix, "Bearer ");
        assert_eq!(resolved.payload_format, WebhookPayloadFormat::JsonText);
        assert_eq!(resolved.payload_text_field, "text");
    }

    #[test]
    fn webhook_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Webhook-Shared",
            "endpoint_url": "https://hooks.example.test/base",
            "auth_token": "base-token",
            "auth_header_name": "X-LoongClaw-Token",
            "auth_token_prefix": "Token ",
            "payload_format": "json_text",
            "payload_text_field": "message",
            "public_base_url": "https://public.example.test/webhook",
            "signing_secret": "base-signing-secret",
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "Webhook-Ops",
                    "endpoint_url": "https://hooks.example.test/ops",
                    "payload_format": "plain_text"
                },
                "Backup": {
                    "enabled": false,
                    "auth_token": "backup-token",
                    "auth_header_name": "X-Backup-Token",
                    "payload_text_field": "backup_message"
                }
            }
        });
        let config: WebhookChannelConfig =
            serde_json::from_value(config_value).expect("deserialize webhook multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default webhook account");
        let ops_endpoint_url = ops.endpoint_url();
        let ops_auth_token = ops.auth_token();
        let ops_signing_secret = ops.signing_secret();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "webhook-ops");
        assert_eq!(ops.account.label, "Webhook-Ops");
        assert_eq!(
            ops_endpoint_url.as_deref(),
            Some("https://hooks.example.test/ops")
        );
        assert_eq!(ops_auth_token.as_deref(), Some("base-token"));
        assert_eq!(ops_signing_secret.as_deref(), Some("base-signing-secret"));
        assert_eq!(ops.auth_header_name, "X-LoongClaw-Token");
        assert_eq!(ops.auth_token_prefix, "Token ");
        assert_eq!(ops.payload_format, WebhookPayloadFormat::PlainText);
        assert_eq!(ops.payload_text_field, "message");
        assert_eq!(
            ops.public_base_url.as_deref(),
            Some("https://public.example.test/webhook")
        );

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit webhook account");
        let backup_endpoint_url = backup.endpoint_url();
        let backup_auth_token = backup.auth_token();
        let backup_signing_secret = backup.signing_secret();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "webhook-shared");
        assert_eq!(backup.account.label, "Webhook-Shared");
        assert_eq!(
            backup_endpoint_url.as_deref(),
            Some("https://hooks.example.test/base")
        );
        assert_eq!(backup_auth_token.as_deref(), Some("backup-token"));
        assert_eq!(
            backup_signing_secret.as_deref(),
            Some("base-signing-secret")
        );
        assert_eq!(backup.auth_header_name, "X-Backup-Token");
        assert_eq!(backup.auth_token_prefix, "Token ");
        assert_eq!(backup.payload_format, WebhookPayloadFormat::JsonText);
        assert_eq!(backup.payload_text_field, "backup_message");
    }

    #[test]
    fn webhook_account_without_env_overrides_inherits_top_level_env_names() {
        let config_value = json!({
            "enabled": true,
            "endpoint_url_env": "ACME_WEBHOOK_ENDPOINT",
            "auth_token_env": "ACME_WEBHOOK_AUTH_TOKEN",
            "signing_secret_env": "ACME_WEBHOOK_SIGNING_SECRET",
            "default_account": "Ops",
            "accounts": {
                "Ops": {}
            }
        });
        let config: WebhookChannelConfig =
            serde_json::from_value(config_value).expect("deserialize webhook multi-account config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default webhook account");

        assert_eq!(
            resolved.endpoint_url_env.as_deref(),
            Some("ACME_WEBHOOK_ENDPOINT")
        );
        assert_eq!(
            resolved.auth_token_env.as_deref(),
            Some("ACME_WEBHOOK_AUTH_TOKEN")
        );
        assert_eq!(
            resolved.signing_secret_env.as_deref(),
            Some("ACME_WEBHOOK_SIGNING_SECRET")
        );
    }

    #[test]
    fn google_chat_resolves_webhook_url_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_GOOGLE_CHAT_WEBHOOK_URL",
            "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=test-key&token=test-token",
        );

        let config_value = json!({
            "enabled": true,
            "account_id": "Google-Chat-Primary",
            "webhook_url_env": "TEST_GOOGLE_CHAT_WEBHOOK_URL"
        });
        let config: GoogleChatChannelConfig =
            serde_json::from_value(config_value).expect("deserialize google chat config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default google chat account");
        let webhook_url = resolved.webhook_url();

        assert_eq!(resolved.configured_account_id, "google-chat-primary");
        assert_eq!(resolved.account.id, "google-chat-primary");
        assert_eq!(resolved.account.label, "Google-Chat-Primary");
        assert_eq!(
            webhook_url.as_deref(),
            Some(
                "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=test-key&token=test-token"
            )
        );
    }

    #[test]
    fn google_chat_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Google-Chat-Shared",
            "webhook_url": "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=base-key&token=base-token",
            "default_account": "Announcements",
            "accounts": {
                "Announcements": {
                    "account_id": "Google-Chat-Announcements",
                    "webhook_url": "https://chat.googleapis.com/v1/spaces/BBBB/messages?key=ann-key&token=ann-token"
                },
                "Backup": {
                    "enabled": false
                }
            }
        });
        let config: GoogleChatChannelConfig = serde_json::from_value(config_value)
            .expect("deserialize google chat multi-account config");

        assert_eq!(
            config.configured_account_ids(),
            vec!["announcements", "backup"]
        );
        assert_eq!(config.default_configured_account_id(), "announcements");

        let announcements = config
            .resolve_account(None)
            .expect("resolve default google chat account");
        let announcements_webhook_url = announcements.webhook_url();

        assert_eq!(announcements.configured_account_id, "announcements");
        assert_eq!(announcements.account.id, "google-chat-announcements");
        assert_eq!(announcements.account.label, "Google-Chat-Announcements");
        assert_eq!(
            announcements_webhook_url.as_deref(),
            Some("https://chat.googleapis.com/v1/spaces/BBBB/messages?key=ann-key&token=ann-token")
        );

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit google chat account");
        let backup_webhook_url = backup.webhook_url();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "google-chat-shared");
        assert_eq!(backup.account.label, "Google-Chat-Shared");
        assert_eq!(
            backup_webhook_url.as_deref(),
            Some(
                "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=base-key&token=base-token"
            )
        );
    }

    #[test]
    fn nextcloud_talk_resolves_server_url_and_shared_secret_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_NEXTCLOUD_TALK_SERVER_URL",
            "https://cloud.example.test",
        );
        env.set(
            "TEST_NEXTCLOUD_TALK_SHARED_SECRET",
            "nextcloud-shared-secret",
        );

        let config_value = json!({
            "enabled": true,
            "account_id": "Nextcloud-Primary",
            "server_url_env": "TEST_NEXTCLOUD_TALK_SERVER_URL",
            "shared_secret_env": "TEST_NEXTCLOUD_TALK_SHARED_SECRET"
        });
        let config: NextcloudTalkChannelConfig =
            serde_json::from_value(config_value).expect("deserialize nextcloud talk config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default nextcloud talk account");
        let server_url = resolved.server_url();
        let shared_secret = resolved.shared_secret();

        assert_eq!(resolved.configured_account_id, "nextcloud-primary");
        assert_eq!(resolved.account.id, "nextcloud-primary");
        assert_eq!(resolved.account.label, "Nextcloud-Primary");
        assert_eq!(server_url.as_deref(), Some("https://cloud.example.test"));
        assert_eq!(shared_secret.as_deref(), Some("nextcloud-shared-secret"));
    }

    #[test]
    fn nextcloud_talk_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Nextcloud-Shared",
            "server_url": "https://cloud.example.test",
            "shared_secret": "base-shared-secret",
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "Nextcloud-Ops",
                    "server_url": "https://ops.example.test"
                },
                "Backup": {
                    "enabled": false,
                    "shared_secret": "backup-shared-secret"
                }
            }
        });
        let config: NextcloudTalkChannelConfig = serde_json::from_value(config_value)
            .expect("deserialize nextcloud talk multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default nextcloud talk account");
        let ops_server_url = ops.server_url();
        let ops_shared_secret = ops.shared_secret();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "nextcloud-ops");
        assert_eq!(ops.account.label, "Nextcloud-Ops");
        assert_eq!(ops_server_url.as_deref(), Some("https://ops.example.test"));
        assert_eq!(ops_shared_secret.as_deref(), Some("base-shared-secret"));

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit nextcloud talk account");
        let backup_server_url = backup.server_url();
        let backup_shared_secret = backup.shared_secret();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "nextcloud-shared");
        assert_eq!(backup.account.label, "Nextcloud-Shared");
        assert_eq!(
            backup_server_url.as_deref(),
            Some("https://cloud.example.test")
        );
        assert_eq!(
            backup_shared_secret.as_deref(),
            Some("backup-shared-secret")
        );
    }

    #[test]
    fn synology_chat_resolves_token_and_incoming_url_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_SYNOLOGY_CHAT_TOKEN", "synology-outgoing-token");
        env.set(
            "TEST_SYNOLOGY_CHAT_INCOMING_URL",
            "https://chat.example.test/webapi/entry.cgi?api=SYNO.Chat.External&method=incoming&version=2&token=secret-token",
        );

        let config_value = json!({
            "enabled": true,
            "account_id": "Synology-Ops",
            "token_env": "TEST_SYNOLOGY_CHAT_TOKEN",
            "incoming_url_env": "TEST_SYNOLOGY_CHAT_INCOMING_URL",
            "allowed_user_ids": [42]
        });
        let config: SynologyChatChannelConfig =
            serde_json::from_value(config_value).expect("deserialize synology chat config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default synology chat account");
        let token = resolved.token();
        let incoming_url = resolved.incoming_url();

        assert_eq!(resolved.configured_account_id, "synology-ops");
        assert_eq!(resolved.account.id, "synology-ops");
        assert_eq!(resolved.account.label, "Synology-Ops");
        assert_eq!(token.as_deref(), Some("synology-outgoing-token"));
        assert_eq!(
            incoming_url.as_deref(),
            Some(
                "https://chat.example.test/webapi/entry.cgi?api=SYNO.Chat.External&method=incoming&version=2&token=secret-token"
            )
        );
        assert_eq!(resolved.allowed_user_ids, vec![42]);
    }

    #[test]
    fn synology_chat_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Synology-Shared",
            "token": "base-synology-token",
            "incoming_url": "https://chat.example.test/webapi/entry.cgi?token=base-token",
            "allowed_user_ids": [1, 2],
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "Synology-Ops",
                    "incoming_url": "https://ops.example.test/webapi/entry.cgi?token=ops-token"
                },
                "Backup": {
                    "enabled": false,
                    "token": "backup-synology-token",
                    "allowed_user_ids": [9]
                }
            }
        });
        let config: SynologyChatChannelConfig = serde_json::from_value(config_value)
            .expect("deserialize synology chat multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default synology chat account");
        let ops_token = ops.token();
        let ops_incoming_url = ops.incoming_url();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "synology-ops");
        assert_eq!(ops.account.label, "Synology-Ops");
        assert_eq!(ops_token.as_deref(), Some("base-synology-token"));
        assert_eq!(
            ops_incoming_url.as_deref(),
            Some("https://ops.example.test/webapi/entry.cgi?token=ops-token")
        );
        assert_eq!(ops.allowed_user_ids, vec![1, 2]);

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit synology chat account");
        let backup_token = backup.token();
        let backup_incoming_url = backup.incoming_url();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "synology-shared");
        assert_eq!(backup.account.label, "Synology-Shared");
        assert_eq!(backup_token.as_deref(), Some("backup-synology-token"));
        assert_eq!(
            backup_incoming_url.as_deref(),
            Some("https://chat.example.test/webapi/entry.cgi?token=base-token")
        );
        assert_eq!(backup.allowed_user_ids, vec![9]);
    }

    #[test]
    fn teams_resolves_webhook_and_future_serve_credentials_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_TEAMS_WEBHOOK_URL",
            "https://teams.example.test/webhook/connector",
        );
        env.set("TEST_TEAMS_APP_ID", "teams-app-id");
        env.set("TEST_TEAMS_APP_PASSWORD", "teams-app-password");
        env.set("TEST_TEAMS_TENANT_ID", "teams-tenant-id");

        let config_value = json!({
            "enabled": true,
            "account_id": "Teams-Ops",
            "webhook_url_env": "TEST_TEAMS_WEBHOOK_URL",
            "app_id_env": "TEST_TEAMS_APP_ID",
            "app_password_env": "TEST_TEAMS_APP_PASSWORD",
            "tenant_id_env": "TEST_TEAMS_TENANT_ID",
            "allowed_conversation_ids": ["19:ops-thread"]
        });
        let config: TeamsChannelConfig =
            serde_json::from_value(config_value).expect("deserialize teams config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default teams account");
        let webhook_url = resolved.webhook_url();
        let app_id = resolved.app_id();
        let app_password = resolved.app_password();
        let tenant_id = resolved.tenant_id();

        assert_eq!(resolved.configured_account_id, "teams-ops");
        assert_eq!(resolved.account.id, "teams-ops");
        assert_eq!(resolved.account.label, "Teams-Ops");
        assert_eq!(
            webhook_url.as_deref(),
            Some("https://teams.example.test/webhook/connector")
        );
        assert_eq!(app_id.as_deref(), Some("teams-app-id"));
        assert_eq!(app_password.as_deref(), Some("teams-app-password"));
        assert_eq!(tenant_id.as_deref(), Some("teams-tenant-id"));
        assert_eq!(
            resolved.allowed_conversation_ids,
            vec!["19:ops-thread".to_owned()]
        );
    }

    #[test]
    fn teams_multi_account_resolution_merges_send_and_future_serve_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Teams-Shared",
            "webhook_url": "https://teams.example.test/webhook/base",
            "app_id": "base-app-id",
            "app_password": "base-app-password",
            "tenant_id": "base-tenant-id",
            "allowed_conversation_ids": ["19:base-thread"],
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "Teams-Ops",
                    "webhook_url": "https://teams.example.test/webhook/ops"
                },
                "Backup": {
                    "enabled": false,
                    "app_password": "backup-app-password",
                    "allowed_conversation_ids": ["19:backup-thread"]
                }
            }
        });
        let config: TeamsChannelConfig =
            serde_json::from_value(config_value).expect("deserialize teams multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default teams account");
        let ops_webhook_url = ops.webhook_url();
        let ops_app_id = ops.app_id();
        let ops_app_password = ops.app_password();
        let ops_tenant_id = ops.tenant_id();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "teams-ops");
        assert_eq!(ops.account.label, "Teams-Ops");
        assert_eq!(
            ops_webhook_url.as_deref(),
            Some("https://teams.example.test/webhook/ops")
        );
        assert_eq!(ops_app_id.as_deref(), Some("base-app-id"));
        assert_eq!(ops_app_password.as_deref(), Some("base-app-password"));
        assert_eq!(ops_tenant_id.as_deref(), Some("base-tenant-id"));
        assert_eq!(
            ops.allowed_conversation_ids,
            vec!["19:base-thread".to_owned()]
        );

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit teams account");
        let backup_webhook_url = backup.webhook_url();
        let backup_app_id = backup.app_id();
        let backup_app_password = backup.app_password();
        let backup_tenant_id = backup.tenant_id();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "teams-shared");
        assert_eq!(backup.account.label, "Teams-Shared");
        assert_eq!(
            backup_webhook_url.as_deref(),
            Some("https://teams.example.test/webhook/base")
        );
        assert_eq!(backup_app_id.as_deref(), Some("base-app-id"));
        assert_eq!(backup_app_password.as_deref(), Some("backup-app-password"));
        assert_eq!(backup_tenant_id.as_deref(), Some("base-tenant-id"));
        assert_eq!(
            backup.allowed_conversation_ids,
            vec!["19:backup-thread".to_owned()]
        );
    }

    #[test]
    fn imessage_resolves_bridge_url_and_token_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_IMESSAGE_BRIDGE_URL",
            "https://bluebubbles.example.test/base",
        );
        env.set("TEST_IMESSAGE_BRIDGE_TOKEN", "bluebubbles-password");

        let config_value = json!({
            "enabled": true,
            "account_id": "BlueBubbles-Ops",
            "bridge_url_env": "TEST_IMESSAGE_BRIDGE_URL",
            "bridge_token_env": "TEST_IMESSAGE_BRIDGE_TOKEN",
            "allowed_chat_ids": ["iMessage;-;+15550001111"]
        });
        let config: ImessageChannelConfig =
            serde_json::from_value(config_value).expect("deserialize imessage config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default imessage account");
        let bridge_url = resolved.bridge_url();
        let bridge_token = resolved.bridge_token();

        assert_eq!(resolved.configured_account_id, "bluebubbles-ops");
        assert_eq!(resolved.account.id, "bluebubbles-ops");
        assert_eq!(resolved.account.label, "BlueBubbles-Ops");
        assert_eq!(
            bridge_url.as_deref(),
            Some("https://bluebubbles.example.test/base")
        );
        assert_eq!(bridge_token.as_deref(), Some("bluebubbles-password"));
        assert_eq!(
            resolved.allowed_chat_ids,
            vec!["iMessage;-;+15550001111".to_owned()]
        );
    }

    #[test]
    fn imessage_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "BlueBubbles-Shared",
            "bridge_url": "https://bluebubbles.example.test/base",
            "bridge_token": "base-bridge-token",
            "allowed_chat_ids": ["iMessage;-;+15550001111"],
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "BlueBubbles-Ops",
                    "bridge_url": "https://bluebubbles.example.test/ops"
                },
                "Backup": {
                    "enabled": false,
                    "bridge_token": "backup-bridge-token",
                    "allowed_chat_ids": ["iMessage;-;+15550002222"]
                }
            }
        });
        let config: ImessageChannelConfig = serde_json::from_value(config_value)
            .expect("deserialize imessage multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default imessage account");
        let ops_bridge_url = ops.bridge_url();
        let ops_bridge_token = ops.bridge_token();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "bluebubbles-ops");
        assert_eq!(ops.account.label, "BlueBubbles-Ops");
        assert_eq!(
            ops_bridge_url.as_deref(),
            Some("https://bluebubbles.example.test/ops")
        );
        assert_eq!(ops_bridge_token.as_deref(), Some("base-bridge-token"));
        assert_eq!(
            ops.allowed_chat_ids,
            vec!["iMessage;-;+15550001111".to_owned()]
        );

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit imessage account");
        let backup_bridge_url = backup.bridge_url();
        let backup_bridge_token = backup.bridge_token();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "bluebubbles-shared");
        assert_eq!(backup.account.label, "BlueBubbles-Shared");
        assert_eq!(
            backup_bridge_url.as_deref(),
            Some("https://bluebubbles.example.test/base")
        );
        assert_eq!(backup_bridge_token.as_deref(), Some("backup-bridge-token"));
        assert_eq!(
            backup.allowed_chat_ids,
            vec!["iMessage;-;+15550002222".to_owned()]
        );
    }

    #[test]
    fn mattermost_resolves_server_url_and_bot_token_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_MATTERMOST_SERVER_URL",
            "https://mattermost.example.test",
        );
        env.set("TEST_MATTERMOST_BOT_TOKEN", "mattermost-token");

        let config_value = json!({
            "enabled": true,
            "account_id": "Mattermost-Ops",
            "server_url_env": "TEST_MATTERMOST_SERVER_URL",
            "bot_token_env": "TEST_MATTERMOST_BOT_TOKEN"
        });
        let config: MattermostChannelConfig =
            serde_json::from_value(config_value).expect("deserialize mattermost config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default mattermost account");
        let server_url = resolved.server_url();
        let bot_token = resolved.bot_token();

        assert_eq!(resolved.configured_account_id, "mattermost-ops");
        assert_eq!(resolved.account.id, "mattermost-ops");
        assert_eq!(resolved.account.label, "Mattermost-Ops");
        assert_eq!(
            server_url.as_deref(),
            Some("https://mattermost.example.test")
        );
        assert_eq!(bot_token.as_deref(), Some("mattermost-token"));
    }

    #[test]
    fn mattermost_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Mattermost-Shared",
            "server_url": "https://mattermost.example.test",
            "bot_token": "base-mattermost-token",
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "Mattermost-Ops",
                    "bot_token": "ops-mattermost-token"
                },
                "Backup": {
                    "enabled": false,
                    "server_url": "https://backup-mattermost.example.test"
                }
            }
        });
        let config: MattermostChannelConfig = serde_json::from_value(config_value)
            .expect("deserialize mattermost multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default mattermost account");
        let ops_server_url = ops.server_url();
        let ops_bot_token = ops.bot_token();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "mattermost-ops");
        assert_eq!(ops.account.label, "Mattermost-Ops");
        assert_eq!(
            ops_server_url.as_deref(),
            Some("https://mattermost.example.test")
        );
        assert_eq!(ops_bot_token.as_deref(), Some("ops-mattermost-token"));

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit mattermost account");
        let backup_server_url = backup.server_url();
        let backup_bot_token = backup.bot_token();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "mattermost-shared");
        assert_eq!(backup.account.label, "Mattermost-Shared");
        assert_eq!(
            backup_server_url.as_deref(),
            Some("https://backup-mattermost.example.test")
        );
        assert_eq!(backup_bot_token.as_deref(), Some("base-mattermost-token"));
    }

    #[test]
    fn signal_resolves_account_and_service_url_from_env_pointers() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_SIGNAL_ACCOUNT", "+15550001111");
        env.set("TEST_SIGNAL_SERVICE_URL", "http://signal.example.test:8080");

        let config_value = json!({
            "enabled": true,
            "account_env": "TEST_SIGNAL_ACCOUNT",
            "service_url_env": "TEST_SIGNAL_SERVICE_URL"
        });
        let config: SignalChannelConfig =
            serde_json::from_value(config_value).expect("deserialize signal config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default signal account");
        let signal_account = resolved.signal_account();
        let service_url = resolved.service_url();

        assert_eq!(resolved.configured_account_id, "signal_15550001111");
        assert_eq!(resolved.account.id, "signal_15550001111");
        assert_eq!(resolved.account.label, "signal:+15550001111");
        assert_eq!(signal_account.as_deref(), Some("+15550001111"));
        assert_eq!(
            service_url.as_deref(),
            Some("http://signal.example.test:8080")
        );
    }

    #[test]
    fn discord_partial_deserialization_keeps_default_env_pointer() {
        let config: DiscordChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize discord config");

        assert_eq!(config.bot_token_env.as_deref(), Some(DISCORD_BOT_TOKEN_ENV));
    }

    #[test]
    fn slack_partial_deserialization_keeps_default_env_pointer() {
        let config: SlackChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize slack config");

        assert_eq!(config.bot_token_env.as_deref(), Some(SLACK_BOT_TOKEN_ENV));
    }

    #[test]
    fn webhook_partial_deserialization_keeps_default_env_pointers() {
        let config: WebhookChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize webhook config");

        assert_eq!(
            config.endpoint_url_env.as_deref(),
            Some(WEBHOOK_ENDPOINT_URL_ENV)
        );
        assert_eq!(
            config.auth_token_env.as_deref(),
            Some(WEBHOOK_AUTH_TOKEN_ENV)
        );
        assert_eq!(
            config.signing_secret_env.as_deref(),
            Some(WEBHOOK_SIGNING_SECRET_ENV)
        );
        assert_eq!(config.auth_header_name, "Authorization");
        assert_eq!(config.auth_token_prefix, "Bearer ");
        assert_eq!(config.payload_format, WebhookPayloadFormat::JsonText);
        assert_eq!(config.payload_text_field, "text");
    }

    #[test]
    fn teams_partial_deserialization_keeps_default_env_pointers() {
        let config: TeamsChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize teams config");

        assert_eq!(
            config.webhook_url_env.as_deref(),
            Some(TEAMS_WEBHOOK_URL_ENV)
        );
        assert_eq!(config.app_id_env.as_deref(), Some(TEAMS_APP_ID_ENV));
        assert_eq!(
            config.app_password_env.as_deref(),
            Some(TEAMS_APP_PASSWORD_ENV)
        );
        assert_eq!(config.tenant_id_env.as_deref(), Some(TEAMS_TENANT_ID_ENV));
    }

    #[test]
    fn imessage_partial_deserialization_keeps_default_env_pointers() {
        let config: ImessageChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize imessage config");

        assert_eq!(
            config.bridge_url_env.as_deref(),
            Some(IMESSAGE_BRIDGE_URL_ENV)
        );
        assert_eq!(
            config.bridge_token_env.as_deref(),
            Some(IMESSAGE_BRIDGE_TOKEN_ENV)
        );
    }

    #[test]
    fn email_partial_deserialization_keeps_default_env_pointers() {
        let config: EmailChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize email config");

        assert_eq!(
            config.smtp_username_env.as_deref(),
            Some(EMAIL_SMTP_USERNAME_ENV)
        );
        assert_eq!(
            config.smtp_password_env.as_deref(),
            Some(EMAIL_SMTP_PASSWORD_ENV)
        );
        assert_eq!(
            config.imap_username_env.as_deref(),
            Some(EMAIL_IMAP_USERNAME_ENV)
        );
        assert_eq!(
            config.imap_password_env.as_deref(),
            Some(EMAIL_IMAP_PASSWORD_ENV)
        );
    }

    #[test]
    fn parse_email_smtp_endpoint_accepts_relay_host() {
        let endpoint =
            parse_email_smtp_endpoint("smtp.example.test").expect("relay host should parse");

        assert_eq!(
            endpoint,
            EmailSmtpEndpoint::RelayHost("smtp.example.test".to_owned())
        );
    }

    #[test]
    fn parse_email_smtp_endpoint_accepts_connection_url() {
        let endpoint = parse_email_smtp_endpoint("smtps://smtp.example.test:465")
            .expect("smtp url should parse");

        assert_eq!(
            endpoint,
            EmailSmtpEndpoint::ConnectionUrl("smtps://smtp.example.test:465".to_owned())
        );
    }

    #[test]
    fn parse_email_smtp_endpoint_rejects_host_port_without_scheme() {
        let error = parse_email_smtp_endpoint("smtp.example.test:587")
            .expect_err("bare host:port should be rejected");

        assert_eq!(
            error,
            "email smtp_host with an explicit port must use a full smtp:// or smtps:// URL"
        );
    }

    #[test]
    fn signal_partial_deserialization_keeps_default_env_pointers() {
        let config: SignalChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize signal config");

        assert_eq!(
            config.signal_account_env.as_deref(),
            Some(SIGNAL_ACCOUNT_ENV)
        );
        assert_eq!(
            config.service_url_env.as_deref(),
            Some(SIGNAL_SERVICE_URL_ENV)
        );
    }

    #[test]
    fn whatsapp_partial_deserialization_keeps_default_env_pointers() {
        let config: WhatsappChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize whatsapp config");

        assert_eq!(
            config.access_token_env.as_deref(),
            Some(WHATSAPP_ACCESS_TOKEN_ENV)
        );
        assert_eq!(
            config.phone_number_id_env.as_deref(),
            Some(WHATSAPP_PHONE_NUMBER_ID_ENV)
        );
        assert_eq!(
            config.verify_token_env.as_deref(),
            Some(WHATSAPP_VERIFY_TOKEN_ENV)
        );
        assert_eq!(
            config.app_secret_env.as_deref(),
            Some(WHATSAPP_APP_SECRET_ENV)
        );
    }

    #[test]
    fn signal_default_service_url_env_override_wins_over_fallback() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("SIGNAL_SERVICE_URL", "http://signal.override.test:8080");

        let config = SignalChannelConfig::default();
        let service_url = config.service_url();

        assert_eq!(
            service_url.as_deref(),
            Some("http://signal.override.test:8080")
        );
    }

    #[test]
    fn signal_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account": "+15550001111",
            "service_url": "http://127.0.0.1:8080",
            "default_account": "Alerts",
            "accounts": {
                "Alerts": {
                    "account_id": "Signal-Alerts",
                    "account": "+15550002222"
                },
                "Backup": {
                    "enabled": false,
                    "service_url": "http://backup.example.test:8080"
                }
            }
        });
        let config: SignalChannelConfig =
            serde_json::from_value(config_value).expect("deserialize signal multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["alerts", "backup"]);
        assert_eq!(config.default_configured_account_id(), "alerts");

        let alerts = config
            .resolve_account(None)
            .expect("resolve default signal account");
        let alerts_signal_account = alerts.signal_account();
        let alerts_service_url = alerts.service_url();

        assert_eq!(alerts.configured_account_id, "alerts");
        assert_eq!(alerts.account.id, "signal-alerts");
        assert_eq!(alerts.account.label, "Signal-Alerts");
        assert_eq!(alerts_signal_account.as_deref(), Some("+15550002222"));
        assert_eq!(alerts_service_url.as_deref(), Some("http://127.0.0.1:8080"));

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit signal account");
        let backup_signal_account = backup.signal_account();
        let backup_service_url = backup.service_url();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup_signal_account.as_deref(), Some("+15550001111"));
        assert_eq!(
            backup_service_url.as_deref(),
            Some("http://backup.example.test:8080")
        );
    }

    #[test]
    fn whatsapp_resolves_phone_number_id_from_env_pointer() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_WHATSAPP_PHONE_NUMBER_ID", "1234567890");

        let config_value = json!({
            "enabled": true,
            "access_token": "whatsapp-token",
            "phone_number_id_env": "TEST_WHATSAPP_PHONE_NUMBER_ID"
        });
        let config: WhatsappChannelConfig =
            serde_json::from_value(config_value).expect("deserialize whatsapp config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default whatsapp account");
        let phone_number_id = resolved.phone_number_id();

        assert_eq!(resolved.configured_account_id, "whatsapp_1234567890");
        assert_eq!(resolved.account.id, "whatsapp_1234567890");
        assert_eq!(resolved.account.label, "whatsapp:1234567890");
        assert_eq!(phone_number_id.as_deref(), Some("1234567890"));
    }

    #[test]
    fn whatsapp_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "access_token": "base-access-token",
            "api_base_url": "https://graph.facebook.com/v25.0",
            "default_account": "Business",
            "accounts": {
                "Business": {
                    "account_id": "WhatsApp-Biz",
                    "phone_number_id": "1111111111"
                },
                "Backup": {
                    "enabled": false,
                    "phone_number_id": "2222222222",
                    "api_base_url": "https://graph.facebook.com/v26.0"
                }
            }
        });
        let config: WhatsappChannelConfig = serde_json::from_value(config_value)
            .expect("deserialize whatsapp multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "business"]);
        assert_eq!(config.default_configured_account_id(), "business");

        let business = config
            .resolve_account(None)
            .expect("resolve default whatsapp account");
        let business_access_token = business.access_token();
        let business_phone_number_id = business.phone_number_id();

        assert_eq!(business.configured_account_id, "business");
        assert_eq!(business.account.id, "whatsapp-biz");
        assert_eq!(business.account.label, "WhatsApp-Biz");
        assert_eq!(business_access_token.as_deref(), Some("base-access-token"));
        assert_eq!(business_phone_number_id.as_deref(), Some("1111111111"));
        assert_eq!(
            business.resolved_api_base_url(),
            "https://graph.facebook.com/v25.0"
        );

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit whatsapp account");
        let backup_access_token = backup.access_token();
        let backup_phone_number_id = backup.phone_number_id();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup_access_token.as_deref(), Some("base-access-token"));
        assert_eq!(backup_phone_number_id.as_deref(), Some("2222222222"));
        assert_eq!(
            backup.resolved_api_base_url(),
            "https://graph.facebook.com/v26.0"
        );
    }

    #[test]
    fn telegram_streaming_mode_deserializes_from_json() {
        let off: TelegramStreamingMode = serde_json::from_str("\"off\"").expect("deserialize off");
        assert_eq!(off, TelegramStreamingMode::Off);

        let draft: TelegramStreamingMode =
            serde_json::from_str("\"draft\"").expect("deserialize draft");
        assert_eq!(draft, TelegramStreamingMode::Draft);
    }

    #[test]
    fn telegram_streaming_mode_default_is_off() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bot_token_env": "TEST_TOKEN"
        }))
        .expect("deserialize telegram config");
        assert_eq!(config.streaming_mode, TelegramStreamingMode::Off);
    }

    #[test]
    fn telegram_streaming_mode_inherited_from_base_in_multi_account() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bot_token_env": "BASE_TOKEN",
            "streaming_mode": "draft",
            "accounts": {
                "Account1": {
                    "bot_token_env": "ACCOUNT1_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram config");

        let resolved = config
            .resolve_account(Some("Account1"))
            .expect("resolve account1");
        assert_eq!(resolved.streaming_mode, TelegramStreamingMode::Draft);
    }

    #[test]
    fn telegram_streaming_mode_overridden_per_account() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bot_token_env": "BASE_TOKEN",
            "streaming_mode": "draft",
            "accounts": {
                "Account1": {
                    "streaming_mode": "off",
                    "bot_token_env": "ACCOUNT1_TOKEN"
                }
            }
        }))
        .expect("deserialize telegram config");

        let resolved = config
            .resolve_account(Some("Account1"))
            .expect("resolve account1");
        assert_eq!(resolved.streaming_mode, TelegramStreamingMode::Off);
    }
}
