use serde::{Deserialize, Serialize};

use crate::prompt::{
    PromptPersonality, PromptRenderInput, render_default_system_prompt, render_system_prompt,
};

use super::defaults::{
    default_exit_commands, default_prompt_pack_id, default_prompt_personality,
    default_system_prompt, default_true,
};

pub(crate) const TELEGRAM_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
pub(crate) const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
pub(crate) const DISCORD_APPLICATION_ID_ENV: &str = "DISCORD_APPLICATION_ID";
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
pub(crate) const TWITCH_ACCESS_TOKEN_ENV: &str = "TWITCH_ACCESS_TOKEN";
pub(crate) const SLACK_BOT_TOKEN_ENV: &str = "SLACK_BOT_TOKEN";
pub(crate) const TEAMS_APP_ID_ENV: &str = "TEAMS_APP_ID";
pub(crate) const TEAMS_APP_PASSWORD_ENV: &str = "TEAMS_APP_PASSWORD";
pub(crate) const TEAMS_TENANT_ID_ENV: &str = "TEAMS_TENANT_ID";
pub(crate) const TEAMS_WEBHOOK_URL_ENV: &str = "TEAMS_WEBHOOK_URL";
pub(crate) const IMESSAGE_BRIDGE_URL_ENV: &str = "IMESSAGE_BRIDGE_URL";
pub(crate) const IMESSAGE_BRIDGE_TOKEN_ENV: &str = "IMESSAGE_BRIDGE_TOKEN";
pub(crate) const NOSTR_RELAY_URLS_ENV: &str = "NOSTR_RELAY_URLS";
pub(crate) const NOSTR_PRIVATE_KEY_ENV: &str = "NOSTR_PRIVATE_KEY";
pub(crate) const TLON_SHIP_ENV: &str = "TLON_SHIP";
pub(crate) const TLON_URL_ENV: &str = "TLON_URL";
pub(crate) const TLON_CODE_ENV: &str = "TLON_CODE";
pub(crate) const WEIXIN_BRIDGE_URL_ENV: &str = "WEIXIN_BRIDGE_URL";
pub(crate) const WEIXIN_BRIDGE_ACCESS_TOKEN_ENV: &str = "WEIXIN_BRIDGE_ACCESS_TOKEN";
pub(crate) const QQBOT_APP_ID_ENV: &str = "QQBOT_APP_ID";
pub(crate) const QQBOT_CLIENT_SECRET_ENV: &str = "QQBOT_CLIENT_SECRET";
pub(crate) const ONEBOT_WEBSOCKET_URL_ENV: &str = "ONEBOT_WEBSOCKET_URL";
pub(crate) const ONEBOT_ACCESS_TOKEN_ENV: &str = "ONEBOT_ACCESS_TOKEN";
pub(crate) const WHATSAPP_ACCESS_TOKEN_ENV: &str = "WHATSAPP_ACCESS_TOKEN";
pub(crate) const WHATSAPP_PHONE_NUMBER_ID_ENV: &str = "WHATSAPP_PHONE_NUMBER_ID";
pub(crate) const WHATSAPP_VERIFY_TOKEN_ENV: &str = "WHATSAPP_VERIFY_TOKEN";
pub(crate) const WHATSAPP_APP_SECRET_ENV: &str = "WHATSAPP_APP_SECRET";
pub(crate) const WHATSAPP_PERSONAL_BRIDGE_URL_ENV: &str = "WHATSAPP_PERSONAL_BRIDGE_URL";
pub(crate) const WHATSAPP_PERSONAL_AUTH_DIR_ENV: &str = "WHATSAPP_PERSONAL_AUTH_DIR";
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
