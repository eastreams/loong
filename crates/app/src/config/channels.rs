use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::prompt::{
    render_default_system_prompt, render_system_prompt, PromptPersonality, PromptRenderInput,
    DEFAULT_PROMPT_PACK_ID,
};
use crate::CliResult;

use super::shared::{
    ConfigValidationCode, ConfigValidationIssue, EnvPointerValidationHint,
    read_secret_prefer_inline, validate_env_pointer_field,
};

const TELEGRAM_BOT_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
const FEISHU_APP_ID_ENV: &str = "FEISHU_APP_ID";
const FEISHU_APP_SECRET_ENV: &str = "FEISHU_APP_SECRET";
const FEISHU_VERIFICATION_TOKEN_ENV: &str = "FEISHU_VERIFICATION_TOKEN";
const FEISHU_ENCRYPT_KEY_ENV: &str = "FEISHU_ENCRYPT_KEY";

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
    #[serde(default = "default_telegram_base_url")]
    pub base_url: String,
    #[serde(default = "default_telegram_timeout_seconds")]
    pub polling_timeout_s: u64,
    #[serde(default)]
    pub allowed_chat_ids: Vec<i64>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub bot_token_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub polling_timeout_s: Option<u64>,
    #[serde(default)]
    pub allowed_chat_ids: Option<Vec<i64>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTelegramChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bot_token: Option<String>,
    pub bot_token_env: Option<String>,
    pub base_url: String,
    pub polling_timeout_s: u64,
    pub allowed_chat_ids: Vec<i64>,
}

impl ResolvedTelegramChannelConfig {
    pub fn bot_token(&self) -> Option<String> {
        read_secret_prefer_inline(self.bot_token.as_deref(), self.bot_token_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeishuAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub app_secret_env: Option<String>,
    #[serde(default)]
    pub domain: Option<FeishuDomain>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub receive_id_type: Option<String>,
    #[serde(default)]
    pub webhook_bind: Option<String>,
    #[serde(default)]
    pub webhook_path: Option<String>,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub verification_token_env: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub encrypt_key_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Option<Vec<String>>,
    #[serde(default)]
    pub ignore_bot_messages: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFeishuChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    pub app_id_env: Option<String>,
    pub app_secret_env: Option<String>,
    pub domain: FeishuDomain,
    pub base_url: Option<String>,
    pub receive_id_type: String,
    pub webhook_bind: String,
    pub webhook_path: String,
    pub verification_token: Option<String>,
    pub verification_token_env: Option<String>,
    pub encrypt_key: Option<String>,
    pub encrypt_key_env: Option<String>,
    pub allowed_chat_ids: Vec<String>,
    pub ignore_bot_messages: bool,
}

impl ResolvedFeishuChannelConfig {
    pub fn app_id(&self) -> Option<String> {
        read_secret_prefer_inline(self.app_id.as_deref(), self.app_id_env.as_deref())
    }

    pub fn app_secret(&self) -> Option<String> {
        read_secret_prefer_inline(self.app_secret.as_deref(), self.app_secret_env.as_deref())
    }

    pub fn verification_token(&self) -> Option<String> {
        read_secret_prefer_inline(
            self.verification_token.as_deref(),
            self.verification_token_env.as_deref(),
        )
    }

    pub fn encrypt_key(&self) -> Option<String> {
        read_secret_prefer_inline(self.encrypt_key.as_deref(), self.encrypt_key_env.as_deref())
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_secret: Option<String>,
    #[serde(default)]
    pub app_id_env: Option<String>,
    #[serde(default)]
    pub app_secret_env: Option<String>,
    #[serde(default)]
    pub domain: FeishuDomain,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_feishu_receive_id_type")]
    pub receive_id_type: String,
    #[serde(default = "default_feishu_webhook_bind")]
    pub webhook_bind: String,
    #[serde(default = "default_feishu_webhook_path")]
    pub webhook_path: String,
    #[serde(default)]
    pub verification_token: Option<String>,
    #[serde(default)]
    pub verification_token_env: Option<String>,
    #[serde(default)]
    pub encrypt_key: Option<String>,
    #[serde(default)]
    pub encrypt_key_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Vec<String>,
    #[serde(default = "default_true")]
    pub ignore_bot_messages: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, FeishuAccountConfig>,
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
            accounts: BTreeMap::new(),
        }
    }
}

impl TelegramChannelConfig {
    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
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
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_account_id(raw_account_id);
            let field_path = format!("telegram.accounts.{account_id}.bot_token_env");
            let inline_field_path = format!("telegram.accounts.{account_id}.bot_token");
            validate_telegram_env_pointer(
                &mut issues,
                field_path.as_str(),
                account.bot_token_env.as_deref(),
                inline_field_path.as_str(),
            );
        }
        issues
    }

    pub fn bot_token(&self) -> Option<String> {
        read_secret_prefer_inline(self.bot_token.as_deref(), self.bot_token_env.as_deref())
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
        })
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
            receive_id_type: default_feishu_receive_id_type(),
            webhook_bind: default_feishu_webhook_bind(),
            webhook_path: default_feishu_webhook_path(),
            verification_token: None,
            verification_token_env: Some(FEISHU_VERIFICATION_TOKEN_ENV.to_owned()),
            encrypt_key: None,
            encrypt_key_env: Some(FEISHU_ENCRYPT_KEY_ENV.to_owned()),
            allowed_chat_ids: Vec::new(),
            ignore_bot_messages: true,
            accounts: BTreeMap::new(),
        }
    }
}

impl FeishuChannelConfig {
    pub(super) fn validate(&self) -> Vec<ConfigValidationIssue> {
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
        validate_feishu_env_pointer(
            &mut issues,
            "feishu.app_secret_env",
            self.app_secret_env.as_deref(),
            "feishu.app_secret",
        );
        validate_feishu_env_pointer(
            &mut issues,
            "feishu.verification_token_env",
            self.verification_token_env.as_deref(),
            "feishu.verification_token",
        );
        validate_feishu_env_pointer(
            &mut issues,
            "feishu.encrypt_key_env",
            self.encrypt_key_env.as_deref(),
            "feishu.encrypt_key",
        );
        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_account_id(raw_account_id);
            validate_feishu_env_pointer(
                &mut issues,
                format!("feishu.accounts.{account_id}.app_id_env").as_str(),
                account.app_id_env.as_deref(),
                format!("feishu.accounts.{account_id}.app_id").as_str(),
            );
            validate_feishu_env_pointer(
                &mut issues,
                format!("feishu.accounts.{account_id}.app_secret_env").as_str(),
                account.app_secret_env.as_deref(),
                format!("feishu.accounts.{account_id}.app_secret").as_str(),
            );
            validate_feishu_env_pointer(
                &mut issues,
                format!("feishu.accounts.{account_id}.verification_token_env").as_str(),
                account.verification_token_env.as_deref(),
                format!("feishu.accounts.{account_id}.verification_token").as_str(),
            );
            validate_feishu_env_pointer(
                &mut issues,
                format!("feishu.accounts.{account_id}.encrypt_key_env").as_str(),
                account.encrypt_key_env.as_deref(),
                format!("feishu.accounts.{account_id}.encrypt_key").as_str(),
            );
        }
        issues
    }

    pub fn app_id(&self) -> Option<String> {
        read_secret_prefer_inline(self.app_id.as_deref(), self.app_id_env.as_deref())
    }

    pub fn app_secret(&self) -> Option<String> {
        read_secret_prefer_inline(self.app_secret.as_deref(), self.app_secret_env.as_deref())
    }

    pub fn verification_token(&self) -> Option<String> {
        read_secret_prefer_inline(
            self.verification_token.as_deref(),
            self.verification_token_env.as_deref(),
        )
    }

    pub fn encrypt_key(&self) -> Option<String> {
        read_secret_prefer_inline(self.encrypt_key.as_deref(), self.encrypt_key_env.as_deref())
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
            receive_id_type: merged.receive_id_type,
            webhook_bind: merged.webhook_bind,
            webhook_path: merged.webhook_path,
            verification_token: merged.verification_token,
            verification_token_env: merged.verification_token_env,
            encrypt_key: merged.encrypt_key,
            encrypt_key_env: merged.encrypt_key_env,
            allowed_chat_ids: merged.allowed_chat_ids,
            ignore_bot_messages: merged.ignore_bot_messages,
        })
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
                id: format!("{}_{}", self.domain.as_str(), normalize_account_id(app_id)),
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

fn default_telegram_base_url() -> String {
    "https://api.telegram.org".to_owned()
}

const fn default_telegram_timeout_seconds() -> u64 {
    15
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

const fn default_true() -> bool {
    true
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
    Some((normalize_account_id(label), label.to_owned()))
}

fn resolve_telegram_bot_id_from_token(token: &str) -> Option<&str> {
    let bot_id = token.split(':').next()?.trim();
    if bot_id.is_empty() || !bot_id.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    Some(bot_id)
}

fn normalize_account_id(raw: &str) -> String {
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
            .entry(normalize_account_id(label))
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
    let normalized_default_account = normalize_account_id(requested_default_account);
    if normalized_to_labels.contains_key(&normalized_default_account) {
        return;
    }

    let mut extra_message_variables = BTreeMap::new();
    extra_message_variables.insert(
        "requested_account_id".to_owned(),
        normalized_default_account.clone(),
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
        .map(|value| normalize_account_id(value))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn normalize_optional_account_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_account_id)
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
            id: normalize_account_id(fallback),
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
            .unwrap_or_else(|| normalize_account_id(fallback)),
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
        selected_configured_account_id: normalize_account_id(selected_configured_account_id),
        default_account_source: default_selection.source,
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
            Some((normalize_account_id(label), label.to_owned(), raw_key))
        })
        .collect::<Vec<_>>();
    let configured_ids = entries
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect::<Vec<_>>();

    if let Some(requested) = requested_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_account_id)
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
            bot_token: Some("987654:token-value".to_owned()),
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
            app_id: Some("cli_a1b2c3".to_owned()),
            app_id_env: None,
            domain: FeishuDomain::Lark,
            ..FeishuChannelConfig::default()
        };

        let identity = config.resolved_account_identity();
        assert_eq!(identity.id, "lark_cli_a1b2c3");
        assert_eq!(identity.label, "lark:cli_a1b2c3");
    }

    #[test]
    fn telegram_multi_account_resolution_merges_base_and_account_overrides() {
        let config: TelegramChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bot_token_env": "BASE_TELEGRAM_TOKEN",
            "polling_timeout_s": 25,
            "allowed_chat_ids": [1001],
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "account_id": "Ops-Bot",
                    "bot_token_env": "WORK_TELEGRAM_TOKEN",
                    "allowed_chat_ids": [2002]
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
        assert_eq!(resolved.polling_timeout_s, 25);

        let disabled = config
            .resolve_account(Some("Personal"))
            .expect("resolve explicit telegram account");
        assert_eq!(disabled.configured_account_id, "personal");
        assert!(!disabled.enabled);
        assert_eq!(disabled.allowed_chat_ids, vec![1001]);
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
            "app_id_env": "BASE_FEISHU_APP_ID",
            "app_secret_env": "BASE_FEISHU_APP_SECRET",
            "verification_token_env": "BASE_FEISHU_VERIFY",
            "encrypt_key_env": "BASE_FEISHU_ENCRYPT",
            "receive_id_type": "chat_id",
            "webhook_bind": "127.0.0.1:8080",
            "webhook_path": "/feishu/events",
            "allowed_chat_ids": ["oc_base"],
            "default_account": "Lark Prod",
            "accounts": {
                "Lark Prod": {
                    "domain": "lark",
                    "app_id": "cli_lark_123",
                    "app_secret": "secret",
                    "verification_token": "verify",
                    "encrypt_key": "encrypt",
                    "allowed_chat_ids": ["oc_lark"]
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
        assert_eq!(resolved.receive_id_type, "chat_id");
        assert_eq!(resolved.resolved_base_url(), "https://open.larksuite.com");

        let disabled = config
            .resolve_account(Some("Feishu Backup"))
            .expect("resolve explicit feishu account");
        assert_eq!(disabled.configured_account_id, "feishu-backup");
        assert!(!disabled.enabled);
        assert_eq!(disabled.allowed_chat_ids, vec!["oc_base".to_owned()]);
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
}
