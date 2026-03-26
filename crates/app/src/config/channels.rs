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
    ConfigValidationCode, ConfigValidationIssue, EnvPointerValidationHint,
    validate_env_pointer_field, validate_secret_ref_env_pointer_field,
};
use crate::secrets::resolve_secret_with_legacy_env;

pub(crate) const TELEGRAM_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
pub(crate) const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
pub(crate) const FEISHU_APP_ID_ENV: &str = "FEISHU_APP_ID";
pub(crate) const FEISHU_APP_SECRET_ENV: &str = "FEISHU_APP_SECRET";
pub(crate) const FEISHU_VERIFICATION_TOKEN_ENV: &str = "FEISHU_VERIFICATION_TOKEN";
pub(crate) const FEISHU_ENCRYPT_KEY_ENV: &str = "FEISHU_ENCRYPT_KEY";
pub(crate) const MATRIX_ACCESS_TOKEN_ENV: &str = "MATRIX_ACCESS_TOKEN";
pub(crate) const SIGNAL_SERVICE_URL_ENV: &str = "SIGNAL_SERVICE_URL";
pub(crate) const SIGNAL_ACCOUNT_ENV: &str = "SIGNAL_ACCOUNT";
pub(crate) const SLACK_BOT_TOKEN_ENV: &str = "SLACK_BOT_TOKEN";
pub(crate) const WHATSAPP_ACCESS_TOKEN_ENV: &str = "WHATSAPP_ACCESS_TOKEN";
pub(crate) const WHATSAPP_PHONE_NUMBER_ID_ENV: &str = "WHATSAPP_PHONE_NUMBER_ID";
pub(crate) const WHATSAPP_VERIFY_TOKEN_ENV: &str = "WHATSAPP_VERIFY_TOKEN";
pub(crate) const WHATSAPP_APP_SECRET_ENV: &str = "WHATSAPP_APP_SECRET";
pub(crate) const WECOM_BOT_ID_ENV: &str = "WECOM_BOT_ID";
pub(crate) const WECOM_SECRET_ENV: &str = "WECOM_SECRET";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramStreamingMode {
    #[default]
    Off,
    Draft,
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

fn default_signal_service_url() -> String {
    "http://127.0.0.1:8080".to_owned()
}

fn default_signal_account_env() -> Option<String> {
    Some(SIGNAL_ACCOUNT_ENV.to_owned())
}

fn default_signal_service_url_env() -> Option<String> {
    Some(SIGNAL_SERVICE_URL_ENV.to_owned())
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
