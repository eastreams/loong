use std::collections::BTreeMap;

use loong_contracts::SecretRef;
use serde::{Deserialize, Serialize};

use super::{
    ChannelAccountIdentity, ChannelAccountIdentitySource, ChannelDefaultAccountSelection,
    ChannelResolvedAccountRoute, ConfigValidationCode, ConfigValidationIssue,
    ConfigValidationSeverity, EnvPointerValidationHint, ONEBOT_ACCESS_TOKEN_ENV,
    ONEBOT_WEBSOCKET_URL_ENV, ResolvedConfiguredAccount, WEIXIN_BRIDGE_ACCESS_TOKEN_ENV,
    WEIXIN_BRIDGE_URL_ENV, WHATSAPP_PERSONAL_AUTH_DIR_ENV, WHATSAPP_PERSONAL_BRIDGE_URL_ENV,
    configured_account_ids, default_channel_account_identity, normalize_channel_account_id,
    resolve_account_for_session_account_id, resolve_channel_account_route,
    resolve_configured_account_identity, resolve_configured_account_selection,
    resolve_default_configured_account_selection, resolve_string_with_legacy_env,
    validate_channel_account_integrity, validate_env_pointer_field,
    validate_secret_ref_env_pointer_field,
};
use crate::CliResult;
use crate::secrets::resolve_secret_with_legacy_env;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeixinAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bridge_url: Option<String>,
    #[serde(default)]
    pub bridge_url_env: Option<String>,
    #[serde(default)]
    pub bridge_access_token: Option<SecretRef>,
    #[serde(default)]
    pub bridge_access_token_env: Option<String>,
    #[serde(default)]
    pub allowed_contact_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWeixinChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bridge_url: Option<String>,
    pub bridge_url_env: Option<String>,
    pub bridge_access_token: Option<SecretRef>,
    pub bridge_access_token_env: Option<String>,
    pub allowed_contact_ids: Vec<String>,
}

impl ResolvedWeixinChannelConfig {
    pub fn bridge_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.bridge_url.as_deref(), self.bridge_url_env.as_deref())
    }

    pub fn bridge_access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.bridge_access_token.as_ref(),
            self.bridge_access_token_env.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnebotAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub websocket_url: Option<String>,
    #[serde(default)]
    pub websocket_url_env: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default)]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub allowed_group_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOnebotChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub websocket_url: Option<String>,
    pub websocket_url_env: Option<String>,
    pub access_token: Option<SecretRef>,
    pub access_token_env: Option<String>,
    pub allowed_group_ids: Vec<String>,
}

impl ResolvedOnebotChannelConfig {
    pub fn websocket_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(
            self.websocket_url.as_deref(),
            self.websocket_url_env.as_deref(),
        )
    }

    pub fn access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.access_token.as_ref(), self.access_token_env.as_deref())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WhatsappPersonalAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub bridge_url: Option<String>,
    #[serde(default)]
    pub bridge_url_env: Option<String>,
    #[serde(default)]
    pub auth_dir: Option<String>,
    #[serde(default)]
    pub auth_dir_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWhatsappPersonalChannelConfig {
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub account: ChannelAccountIdentity,
    pub enabled: bool,
    pub bridge_url: Option<String>,
    pub bridge_url_env: Option<String>,
    pub auth_dir: Option<String>,
    pub auth_dir_env: Option<String>,
    pub allowed_chat_ids: Vec<String>,
}

impl ResolvedWhatsappPersonalChannelConfig {
    pub fn bridge_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.bridge_url.as_deref(), self.bridge_url_env.as_deref())
    }

    pub fn auth_dir(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.auth_dir.as_deref(), self.auth_dir_env.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WeixinChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub managed_bridge_plugin_id: Option<String>,
    #[serde(default)]
    pub bridge_url: Option<String>,
    #[serde(default = "default_weixin_bridge_url_env")]
    pub bridge_url_env: Option<String>,
    #[serde(default)]
    pub bridge_access_token: Option<SecretRef>,
    #[serde(default = "default_weixin_bridge_access_token_env")]
    pub bridge_access_token_env: Option<String>,
    #[serde(default)]
    pub allowed_contact_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, WeixinAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OnebotChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub managed_bridge_plugin_id: Option<String>,
    #[serde(default)]
    pub websocket_url: Option<String>,
    #[serde(default = "default_onebot_websocket_url_env")]
    pub websocket_url_env: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default = "default_onebot_access_token_env")]
    pub access_token_env: Option<String>,
    #[serde(default)]
    pub allowed_group_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, OnebotAccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WhatsappPersonalChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub default_account: Option<String>,
    #[serde(default)]
    pub managed_bridge_plugin_id: Option<String>,
    #[serde(default)]
    pub bridge_url: Option<String>,
    #[serde(default = "default_whatsapp_personal_bridge_url_env")]
    pub bridge_url_env: Option<String>,
    #[serde(default)]
    pub auth_dir: Option<String>,
    #[serde(default = "default_whatsapp_personal_auth_dir_env")]
    pub auth_dir_env: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, WhatsappPersonalAccountConfig>,
}

impl WeixinChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "weixin",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_effective_weixin_runtime_account_ids(&mut issues, self);
        validate_weixin_env_pointer(
            &mut issues,
            "weixin.bridge_url_env",
            self.bridge_url_env.as_deref(),
            "weixin.bridge_url",
        );
        validate_weixin_env_pointer(
            &mut issues,
            "weixin.bridge_access_token_env",
            self.bridge_access_token_env.as_deref(),
            "weixin.bridge_access_token",
        );
        validate_weixin_secret_ref_env_pointer(
            &mut issues,
            "weixin.bridge_access_token",
            self.bridge_access_token.as_ref(),
        );

        for (raw_account_id, account) in &self.accounts {
            let account_id = raw_account_id.as_str();
            let bridge_url_field_path = format!("weixin.accounts.{account_id}.bridge_url");
            let bridge_url_env_field_path = format!("{bridge_url_field_path}_env");
            validate_weixin_env_pointer(
                &mut issues,
                bridge_url_env_field_path.as_str(),
                account.bridge_url_env.as_deref(),
                bridge_url_field_path.as_str(),
            );

            let token_field_path = format!("weixin.accounts.{account_id}.bridge_access_token");
            let token_env_field_path = format!("{token_field_path}_env");
            validate_weixin_env_pointer(
                &mut issues,
                token_env_field_path.as_str(),
                account.bridge_access_token_env.as_deref(),
                token_field_path.as_str(),
            );
            validate_weixin_secret_ref_env_pointer(
                &mut issues,
                token_field_path.as_str(),
                account.bridge_access_token.as_ref(),
            );
        }

        issues
    }

    pub fn bridge_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.bridge_url.as_deref(), self.bridge_url_env.as_deref())
    }

    pub fn bridge_access_token(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.bridge_access_token.as_ref(),
            self.bridge_access_token_env.as_deref(),
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
            "default",
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
            "default",
            requested_account_id,
            selected_configured_account_id,
        )
    }

    pub fn resolve_account(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedWeixinChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = WeixinChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            managed_bridge_plugin_id: self.managed_bridge_plugin_id.clone(),
            bridge_url: account_override
                .and_then(|account| account.bridge_url.clone())
                .or_else(|| self.bridge_url.clone()),
            bridge_url_env: account_override
                .and_then(|account| account.bridge_url_env.clone())
                .or_else(|| self.bridge_url_env.clone()),
            bridge_access_token: account_override
                .and_then(|account| account.bridge_access_token.clone())
                .or_else(|| self.bridge_access_token.clone()),
            bridge_access_token_env: account_override
                .and_then(|account| account.bridge_access_token_env.clone())
                .or_else(|| self.bridge_access_token_env.clone()),
            allowed_contact_ids: account_override
                .and_then(|account| account.allowed_contact_ids.clone())
                .unwrap_or_else(|| self.allowed_contact_ids.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedWeixinChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            bridge_url: merged.bridge_url,
            bridge_url_env: merged.bridge_url_env,
            bridge_access_token: merged.bridge_access_token,
            bridge_access_token_env: merged.bridge_access_token_env,
            allowed_contact_ids: merged.allowed_contact_ids,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedWeixinChannelConfig> {
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
            "default",
        )
    }
}

impl WhatsappPersonalChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "whatsapp_personal",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_effective_whatsapp_personal_runtime_account_ids(&mut issues, self);
        validate_whatsapp_personal_env_pointer(
            &mut issues,
            "whatsapp_personal.bridge_url_env",
            self.bridge_url_env.as_deref(),
            "whatsapp_personal.bridge_url",
        );
        validate_whatsapp_personal_env_pointer(
            &mut issues,
            "whatsapp_personal.auth_dir_env",
            self.auth_dir_env.as_deref(),
            "whatsapp_personal.auth_dir",
        );

        for (raw_account_id, account) in &self.accounts {
            let account_id = raw_account_id.as_str();
            let bridge_url_field_path =
                format!("whatsapp_personal.accounts.{account_id}.bridge_url");
            let bridge_url_env_field_path = format!("{bridge_url_field_path}_env");
            validate_whatsapp_personal_env_pointer(
                &mut issues,
                bridge_url_env_field_path.as_str(),
                account.bridge_url_env.as_deref(),
                bridge_url_field_path.as_str(),
            );

            let auth_dir_field_path = format!("whatsapp_personal.accounts.{account_id}.auth_dir");
            let auth_dir_env_field_path = format!("{auth_dir_field_path}_env");
            validate_whatsapp_personal_env_pointer(
                &mut issues,
                auth_dir_env_field_path.as_str(),
                account.auth_dir_env.as_deref(),
                auth_dir_field_path.as_str(),
            );
        }

        issues
    }

    pub fn bridge_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.bridge_url.as_deref(), self.bridge_url_env.as_deref())
    }

    pub fn auth_dir(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.auth_dir.as_deref(), self.auth_dir_env.as_deref())
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
            "default",
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
            "default",
            requested_account_id,
            selected_configured_account_id,
        )
    }

    pub fn resolve_account(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedWhatsappPersonalChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = WhatsappPersonalChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            managed_bridge_plugin_id: self.managed_bridge_plugin_id.clone(),
            bridge_url: account_override
                .and_then(|account| account.bridge_url.clone())
                .or_else(|| self.bridge_url.clone()),
            bridge_url_env: account_override
                .and_then(|account| account.bridge_url_env.clone())
                .or_else(|| self.bridge_url_env.clone()),
            auth_dir: account_override
                .and_then(|account| account.auth_dir.clone())
                .or_else(|| self.auth_dir.clone()),
            auth_dir_env: account_override
                .and_then(|account| account.auth_dir_env.clone())
                .or_else(|| self.auth_dir_env.clone()),
            allowed_chat_ids: account_override
                .and_then(|account| account.allowed_chat_ids.clone())
                .unwrap_or_else(|| self.allowed_chat_ids.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedWhatsappPersonalChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            bridge_url: merged.bridge_url,
            bridge_url_env: merged.bridge_url_env,
            auth_dir: merged.auth_dir,
            auth_dir_env: merged.auth_dir_env,
            allowed_chat_ids: merged.allowed_chat_ids,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedWhatsappPersonalChannelConfig> {
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

        let bridge_url = self.bridge_url();
        let bridge_url = bridge_url.as_deref();
        let authority = resolve_url_authority_label(bridge_url);
        if let Some(authority) = authority {
            let normalized_authority = normalize_channel_account_id(authority.as_str());
            let account_id = format!("whatsapp_personal_{normalized_authority}");
            let account_label = format!("whatsapp-personal:{authority}");
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
            "default",
        )
    }
}

impl OnebotChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "onebot",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_effective_onebot_runtime_account_ids(&mut issues, self);
        validate_onebot_env_pointer(
            &mut issues,
            "onebot.websocket_url_env",
            self.websocket_url_env.as_deref(),
            "onebot.websocket_url",
        );
        validate_onebot_env_pointer(
            &mut issues,
            "onebot.access_token_env",
            self.access_token_env.as_deref(),
            "onebot.access_token",
        );
        validate_onebot_secret_ref_env_pointer(
            &mut issues,
            "onebot.access_token",
            self.access_token.as_ref(),
        );

        for (raw_account_id, account) in &self.accounts {
            let account_id = raw_account_id.as_str();
            let websocket_url_field_path = format!("onebot.accounts.{account_id}.websocket_url");
            let websocket_url_env_field_path = format!("{websocket_url_field_path}_env");
            validate_onebot_env_pointer(
                &mut issues,
                websocket_url_env_field_path.as_str(),
                account.websocket_url_env.as_deref(),
                websocket_url_field_path.as_str(),
            );

            let access_token_field_path = format!("onebot.accounts.{account_id}.access_token");
            let access_token_env_field_path = format!("{access_token_field_path}_env");
            validate_onebot_env_pointer(
                &mut issues,
                access_token_env_field_path.as_str(),
                account.access_token_env.as_deref(),
                access_token_field_path.as_str(),
            );
            validate_onebot_secret_ref_env_pointer(
                &mut issues,
                access_token_field_path.as_str(),
                account.access_token.as_ref(),
            );
        }

        issues
    }

    pub fn websocket_url(&self) -> Option<String> {
        resolve_string_with_legacy_env(
            self.websocket_url.as_deref(),
            self.websocket_url_env.as_deref(),
        )
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
            "default",
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
            "default",
            requested_account_id,
            selected_configured_account_id,
        )
    }

    pub fn resolve_account(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedOnebotChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = OnebotChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            managed_bridge_plugin_id: self.managed_bridge_plugin_id.clone(),
            websocket_url: account_override
                .and_then(|account| account.websocket_url.clone())
                .or_else(|| self.websocket_url.clone()),
            websocket_url_env: account_override
                .and_then(|account| account.websocket_url_env.clone())
                .or_else(|| self.websocket_url_env.clone()),
            access_token: account_override
                .and_then(|account| account.access_token.clone())
                .or_else(|| self.access_token.clone()),
            access_token_env: account_override
                .and_then(|account| account.access_token_env.clone())
                .or_else(|| self.access_token_env.clone()),
            allowed_group_ids: account_override
                .and_then(|account| account.allowed_group_ids.clone())
                .unwrap_or_else(|| self.allowed_group_ids.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedOnebotChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            websocket_url: merged.websocket_url,
            websocket_url_env: merged.websocket_url_env,
            access_token: merged.access_token,
            access_token_env: merged.access_token_env,
            allowed_group_ids: merged.allowed_group_ids,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedOnebotChannelConfig> {
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

        let websocket_url = self.websocket_url();
        let websocket_url = websocket_url.as_deref();
        let authority = resolve_url_authority_label(websocket_url);
        if let Some(authority) = authority {
            let normalized_authority = normalize_channel_account_id(authority.as_str());
            let account_id = format!("onebot_{normalized_authority}");
            let account_label = format!("onebot:{authority}");
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
            "default",
        )
    }
}

fn validate_effective_weixin_runtime_account_ids(
    issues: &mut Vec<ConfigValidationIssue>,
    config: &WeixinChannelConfig,
) {
    let mut runtime_account_ids = BTreeMap::<String, Vec<String>>::new();
    for configured_account_id in config.configured_account_ids() {
        let resolved = config.resolve_account(Some(configured_account_id.as_str()));
        let Ok(resolved) = resolved else {
            continue;
        };
        let normalized_runtime_account_id =
            normalize_channel_account_id(resolved.account.id.as_str());
        runtime_account_ids
            .entry(normalized_runtime_account_id)
            .or_default()
            .push(resolved.configured_account_label);
    }
    push_duplicate_effective_runtime_account_id_issues(issues, "weixin", runtime_account_ids);
}

fn validate_effective_onebot_runtime_account_ids(
    issues: &mut Vec<ConfigValidationIssue>,
    config: &OnebotChannelConfig,
) {
    let mut runtime_account_ids = BTreeMap::<String, Vec<String>>::new();
    for configured_account_id in config.configured_account_ids() {
        let resolved = config.resolve_account(Some(configured_account_id.as_str()));
        let Ok(resolved) = resolved else {
            continue;
        };
        let normalized_runtime_account_id =
            normalize_channel_account_id(resolved.account.id.as_str());
        runtime_account_ids
            .entry(normalized_runtime_account_id)
            .or_default()
            .push(resolved.configured_account_label);
    }
    push_duplicate_effective_runtime_account_id_issues(issues, "onebot", runtime_account_ids);
}

fn validate_effective_whatsapp_personal_runtime_account_ids(
    issues: &mut Vec<ConfigValidationIssue>,
    config: &WhatsappPersonalChannelConfig,
) {
    let mut runtime_account_ids = BTreeMap::<String, Vec<String>>::new();
    for configured_account_id in config.configured_account_ids() {
        let resolved = config.resolve_account(Some(configured_account_id.as_str()));
        let Ok(resolved) = resolved else {
            continue;
        };
        let normalized_runtime_account_id =
            normalize_channel_account_id(resolved.account.id.as_str());
        runtime_account_ids
            .entry(normalized_runtime_account_id)
            .or_default()
            .push(resolved.configured_account_label);
    }
    push_duplicate_effective_runtime_account_id_issues(
        issues,
        "whatsapp_personal",
        runtime_account_ids,
    );
}

fn push_duplicate_effective_runtime_account_id_issues(
    issues: &mut Vec<ConfigValidationIssue>,
    channel_key: &str,
    runtime_account_ids: BTreeMap<String, Vec<String>>,
) {
    for (normalized_account_id, labels) in runtime_account_ids {
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
            severity: ConfigValidationSeverity::Error,
            code: ConfigValidationCode::DuplicateChannelAccountId,
            field_path: format!("{channel_key}.accounts"),
            inline_field_path: format!("{channel_key}.accounts.{normalized_account_id}"),
            example_env_name: String::new(),
            suggested_env_name: None,
            extra_message_variables,
        });
    }
}

fn default_weixin_bridge_url_env() -> Option<String> {
    Some(WEIXIN_BRIDGE_URL_ENV.to_owned())
}

fn default_weixin_bridge_access_token_env() -> Option<String> {
    Some(WEIXIN_BRIDGE_ACCESS_TOKEN_ENV.to_owned())
}

fn default_onebot_websocket_url_env() -> Option<String> {
    Some(ONEBOT_WEBSOCKET_URL_ENV.to_owned())
}

fn default_onebot_access_token_env() -> Option<String> {
    Some(ONEBOT_ACCESS_TOKEN_ENV.to_owned())
}

fn default_whatsapp_personal_bridge_url_env() -> Option<String> {
    Some(WHATSAPP_PERSONAL_BRIDGE_URL_ENV.to_owned())
}

fn default_whatsapp_personal_auth_dir_env() -> Option<String> {
    Some(WHATSAPP_PERSONAL_AUTH_DIR_ENV.to_owned())
}

fn resolve_url_authority_label(raw_url: Option<&str>) -> Option<String> {
    let url = raw_url.map(str::trim).filter(|value| !value.is_empty())?;
    let parsed_url = reqwest::Url::parse(url).ok()?;
    let host = parsed_url.host_str().map(str::trim)?;
    if host.is_empty() {
        return None;
    }

    let host_label = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };

    if let Some(port) = parsed_url.port() {
        return Some(format!("{host_label}:{port}"));
    }

    Some(host_label)
}

fn validate_weixin_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("bridge_url_env") {
        WEIXIN_BRIDGE_URL_ENV
    } else {
        WEIXIN_BRIDGE_ACCESS_TOKEN_ENV
    };

    let validation_result = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    );
    if let Err(issue) = validation_result {
        issues.push(*issue);
    }
}

fn validate_weixin_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let validation_result = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: WEIXIN_BRIDGE_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    );
    if let Err(issue) = validation_result {
        issues.push(*issue);
    }
}

fn validate_onebot_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("access_token_env") {
        ONEBOT_ACCESS_TOKEN_ENV
    } else {
        ONEBOT_WEBSOCKET_URL_ENV
    };

    let validation_result = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    );
    if let Err(issue) = validation_result {
        issues.push(*issue);
    }
}

fn validate_whatsapp_personal_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let example_env_name = if field_path.ends_with("auth_dir_env") {
        WHATSAPP_PERSONAL_AUTH_DIR_ENV
    } else {
        WHATSAPP_PERSONAL_BRIDGE_URL_ENV
    };

    let validation_result = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name,
            detect_telegram_token_shape: false,
        },
    );
    if let Err(issue) = validation_result {
        issues.push(*issue);
    }
}

fn validate_onebot_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let validation_result = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: ONEBOT_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    );
    if let Err(issue) = validation_result {
        issues.push(*issue);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn weixin_partial_deserialization_keeps_default_env_pointers() {
        let config: WeixinChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize weixin config");

        assert_eq!(
            config.bridge_url_env.as_deref(),
            Some(WEIXIN_BRIDGE_URL_ENV)
        );
        assert_eq!(
            config.bridge_access_token_env.as_deref(),
            Some(WEIXIN_BRIDGE_ACCESS_TOKEN_ENV)
        );
    }

    #[test]
    fn onebot_partial_deserialization_keeps_default_env_pointers() {
        let config: OnebotChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize onebot config");

        assert_eq!(
            config.websocket_url_env.as_deref(),
            Some(ONEBOT_WEBSOCKET_URL_ENV)
        );
        assert_eq!(
            config.access_token_env.as_deref(),
            Some(ONEBOT_ACCESS_TOKEN_ENV)
        );
    }

    #[test]
    fn weixin_deserializes_managed_bridge_plugin_id() {
        let config: WeixinChannelConfig = serde_json::from_value(json!({
            "managed_bridge_plugin_id": "weixin-clawbot"
        }))
        .expect("deserialize weixin config");

        assert_eq!(
            config.managed_bridge_plugin_id.as_deref(),
            Some("weixin-clawbot")
        );
    }

    #[test]
    fn onebot_resolves_websocket_url_from_env_pointer() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_ONEBOT_WS_URL", "ws://127.0.0.1:5700");

        let config_value = json!({
            "enabled": true,
            "websocket_url_env": "TEST_ONEBOT_WS_URL"
        });
        let config: OnebotChannelConfig =
            serde_json::from_value(config_value).expect("deserialize onebot config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve onebot account from env pointer");
        let websocket_url = resolved.websocket_url();

        assert_eq!(resolved.configured_account_id, "default");
        assert_eq!(resolved.account.id, "onebot_127-0-0-1-5700");
        assert_eq!(resolved.account.label, "onebot:127.0.0.1:5700");
        assert_eq!(websocket_url.as_deref(), Some("ws://127.0.0.1:5700"));
    }

    #[test]
    fn weixin_validate_rejects_duplicate_effective_runtime_account_ids() {
        let config: WeixinChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bridge_url": "https://bridge.example.test/weixin",
            "accounts": {
                "alpha": {},
                "beta": {}
            }
        }))
        .expect("deserialize weixin config");

        let issues = config.validate();

        assert!(
            issues.iter().any(|issue| {
                issue.field_path == "weixin.accounts"
                    && issue
                        .extra_message_variables
                        .get("normalized_account_id")
                        .map(|value| value == "default")
                        .unwrap_or(false)
            }),
            "validation should reject duplicate effective weixin account ids: {issues:#?}"
        );
    }

    #[test]
    fn onebot_validate_rejects_duplicate_effective_runtime_account_ids() {
        let config: OnebotChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "websocket_url": "ws://127.0.0.1:5700",
            "accounts": {
                "alpha": {},
                "beta": {}
            }
        }))
        .expect("deserialize onebot config");

        let issues = config.validate();

        assert!(
            issues.iter().any(|issue| {
                issue.field_path == "onebot.accounts"
                    && issue
                        .extra_message_variables
                        .get("normalized_account_id")
                        .map(|value| value == "onebot_127-0-0-1-5700")
                        .unwrap_or(false)
            }),
            "validation should reject duplicate effective onebot account ids: {issues:#?}"
        );
    }

    #[test]
    fn weixin_validate_uses_raw_account_key_in_env_pointer_paths() {
        let config: WeixinChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "accounts": {
                "Ops Team": {
                    "bridge_url_env": "BAD ENV"
                }
            }
        }))
        .expect("deserialize weixin config");

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "weixin.accounts.Ops Team.bridge_url_env"),
            "validation should preserve raw weixin account key in issue path: {issues:#?}"
        );
    }

    #[test]
    fn whatsapp_personal_partial_deserialization_keeps_default_env_pointers() {
        let config: WhatsappPersonalChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize whatsapp personal config");

        assert_eq!(
            config.bridge_url_env.as_deref(),
            Some(WHATSAPP_PERSONAL_BRIDGE_URL_ENV)
        );
        assert_eq!(
            config.auth_dir_env.as_deref(),
            Some(WHATSAPP_PERSONAL_AUTH_DIR_ENV)
        );
    }

    #[test]
    fn whatsapp_personal_resolves_bridge_url_from_env_pointer() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set(
            "TEST_WHATSAPP_PERSONAL_BRIDGE_URL",
            "http://127.0.0.1:39731/bridge",
        );

        let config_value = json!({
            "enabled": true,
            "bridge_url_env": "TEST_WHATSAPP_PERSONAL_BRIDGE_URL"
        });
        let config: WhatsappPersonalChannelConfig =
            serde_json::from_value(config_value).expect("deserialize whatsapp personal config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve whatsapp personal account from env pointer");
        let bridge_url = resolved.bridge_url();

        assert_eq!(
            resolved.configured_account_id,
            "whatsapp_personal_127-0-0-1-39731"
        );
        assert_eq!(resolved.account.id, "whatsapp_personal_127-0-0-1-39731");
        assert_eq!(bridge_url.as_deref(), Some("http://127.0.0.1:39731/bridge"));
    }

    #[test]
    fn whatsapp_personal_validate_rejects_duplicate_effective_runtime_account_ids() {
        let config: WhatsappPersonalChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "bridge_url": "http://127.0.0.1:39731/bridge",
            "accounts": {
                "alpha": {},
                "beta": {}
            }
        }))
        .expect("deserialize whatsapp personal config");

        let issues = config.validate();

        assert!(
            issues.iter().any(|issue| {
                issue.field_path == "whatsapp_personal.accounts"
                    && issue
                        .extra_message_variables
                        .get("normalized_account_id")
                        .map(|value| value == "whatsapp_personal_127-0-0-1-39731")
                        .unwrap_or(false)
            }),
            "validation should reject duplicate effective whatsapp personal account ids: {issues:#?}"
        );
    }

    #[test]
    fn whatsapp_personal_validate_uses_raw_account_key_in_env_pointer_paths() {
        let config: WhatsappPersonalChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "accounts": {
                "Ops Team": {
                    "bridge_url_env": "BAD ENV"
                }
            }
        }))
        .expect("deserialize whatsapp personal config");

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path
                    == "whatsapp_personal.accounts.Ops Team.bridge_url_env"),
            "validation should preserve raw whatsapp personal account key in issue path: {issues:#?}"
        );
    }

    #[test]
    fn onebot_validate_uses_raw_account_key_in_env_pointer_paths() {
        let config: OnebotChannelConfig = serde_json::from_value(json!({
            "enabled": true,
            "accounts": {
                "Ops Team": {
                    "websocket_url_env": "BAD ENV"
                }
            }
        }))
        .expect("deserialize onebot config");

        let issues = config.validate();

        assert!(
            issues
                .iter()
                .any(|issue| issue.field_path == "onebot.accounts.Ops Team.websocket_url_env"),
            "validation should preserve raw onebot account key in issue path: {issues:#?}"
        );
    }
}
