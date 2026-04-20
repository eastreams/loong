use super::*;

pub(super) fn default_twitch_api_base_url() -> String {
    "https://api.twitch.tv/helix".to_owned()
}

pub(super) fn default_twitch_oauth_base_url() -> String {
    "https://id.twitch.tv/oauth2".to_owned()
}

pub(super) fn default_twitch_access_token_env() -> Option<String> {
    Some(TWITCH_ACCESS_TOKEN_ENV.to_owned())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TwitchAccountConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub access_token: Option<SecretRef>,
    #[serde(default)]
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

pub(super) fn validate_twitch_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    env_key: Option<&str>,
    inline_field_path: &str,
) {
    let validation_result = validate_env_pointer_field(
        field_path,
        env_key,
        EnvPointerValidationHint {
            inline_field_path,
            example_env_name: TWITCH_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    );

    if let Err(issue) = validation_result {
        issues.push(*issue);
    }
}

pub(super) fn validate_twitch_secret_ref_env_pointer(
    issues: &mut Vec<ConfigValidationIssue>,
    field_path: &str,
    secret_ref: Option<&SecretRef>,
) {
    let validation_result = validate_secret_ref_env_pointer_field(
        field_path,
        secret_ref,
        EnvPointerValidationHint {
            inline_field_path: field_path,
            example_env_name: TWITCH_ACCESS_TOKEN_ENV,
            detect_telegram_token_shape: false,
        },
    );

    if let Err(issue) = validation_result {
        issues.push(*issue);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn twitch_resolves_access_token_and_base_urls_from_env_pointer() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_TWITCH_ACCESS_TOKEN", "twitch-user-token");

        let config_value = json!({
            "enabled": true,
            "account_id": "Twitch-Ops",
            "access_token_env": "TEST_TWITCH_ACCESS_TOKEN",
            "api_base_url": "https://api.twitch.test/helix",
            "oauth_base_url": "https://id.twitch.test/oauth2",
            "channel_names": ["streamer-a", "streamer-b"]
        });
        let config: TwitchChannelConfig =
            serde_json::from_value(config_value).expect("deserialize twitch config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default twitch account");
        let access_token = resolved.access_token();

        assert_eq!(resolved.configured_account_id, "twitch-ops");
        assert_eq!(resolved.account.id, "twitch-ops");
        assert_eq!(resolved.account.label, "Twitch-Ops");
        assert_eq!(access_token.as_deref(), Some("twitch-user-token"));
        assert_eq!(
            resolved.resolved_api_base_url(),
            "https://api.twitch.test/helix"
        );
        assert_eq!(
            resolved.resolved_oauth_base_url(),
            "https://id.twitch.test/oauth2"
        );
        assert_eq!(
            resolved.channel_names,
            vec!["streamer-a".to_owned(), "streamer-b".to_owned()]
        );
    }

    #[test]
    fn twitch_partial_deserialization_keeps_default_env_pointer_and_base_urls() {
        let config: TwitchChannelConfig = serde_json::from_value(json!({
            "enabled": true
        }))
        .expect("deserialize twitch config");

        assert_eq!(
            config.access_token_env.as_deref(),
            Some(TWITCH_ACCESS_TOKEN_ENV)
        );
        assert_eq!(
            config.resolved_api_base_url(),
            default_twitch_api_base_url()
        );
        assert_eq!(
            config.resolved_oauth_base_url(),
            default_twitch_oauth_base_url()
        );
    }

    #[test]
    fn twitch_multi_account_resolution_merges_base_and_account_overrides() {
        let config_value = json!({
            "enabled": true,
            "account_id": "Twitch-Shared",
            "access_token": "base-twitch-token",
            "api_base_url": "https://api.twitch.example.test/helix",
            "oauth_base_url": "https://id.twitch.example.test/oauth2",
            "channel_names": ["base-channel"],
            "default_account": "Ops",
            "accounts": {
                "Ops": {
                    "account_id": "Twitch-Ops",
                    "access_token": "ops-twitch-token"
                },
                "Backup": {
                    "enabled": false,
                    "api_base_url": "https://backup-api.twitch.example.test/helix",
                    "channel_names": ["backup-channel"]
                }
            }
        });
        let config: TwitchChannelConfig =
            serde_json::from_value(config_value).expect("deserialize twitch multi-account config");

        assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
        assert_eq!(config.default_configured_account_id(), "ops");

        let ops = config
            .resolve_account(None)
            .expect("resolve default twitch account");
        let ops_access_token = ops.access_token();

        assert_eq!(ops.configured_account_id, "ops");
        assert_eq!(ops.account.id, "twitch-ops");
        assert_eq!(ops.account.label, "Twitch-Ops");
        assert_eq!(ops_access_token.as_deref(), Some("ops-twitch-token"));
        assert_eq!(
            ops.resolved_api_base_url(),
            "https://api.twitch.example.test/helix"
        );
        assert_eq!(
            ops.resolved_oauth_base_url(),
            "https://id.twitch.example.test/oauth2"
        );
        assert_eq!(ops.channel_names, vec!["base-channel".to_owned()]);

        let backup = config
            .resolve_account(Some("Backup"))
            .expect("resolve explicit twitch account");
        let backup_access_token = backup.access_token();

        assert_eq!(backup.configured_account_id, "backup");
        assert!(!backup.enabled);
        assert_eq!(backup.account.id, "twitch-shared");
        assert_eq!(backup.account.label, "Twitch-Shared");
        assert_eq!(backup_access_token.as_deref(), Some("base-twitch-token"));
        assert_eq!(
            backup.resolved_api_base_url(),
            "https://backup-api.twitch.example.test/helix"
        );
        assert_eq!(
            backup.resolved_oauth_base_url(),
            "https://id.twitch.example.test/oauth2"
        );
        assert_eq!(backup.channel_names, vec!["backup-channel".to_owned()]);
    }

    #[test]
    fn twitch_empty_account_override_inherits_top_level_access_token_env() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("CUSTOM_TWITCH_TOKEN", "custom-top-level-token");

        let config_value = json!({
            "enabled": true,
            "access_token_env": "CUSTOM_TWITCH_TOKEN",
            "default_account": "Ops",
            "accounts": {
                "Ops": {}
            }
        });
        let config: TwitchChannelConfig =
            serde_json::from_value(config_value).expect("deserialize twitch config");

        let resolved = config
            .resolve_account(None)
            .expect("resolve default twitch account");
        let access_token = resolved.access_token();

        assert_eq!(resolved.configured_account_id, "ops");
        assert_eq!(
            resolved.access_token_env.as_deref(),
            Some("CUSTOM_TWITCH_TOKEN")
        );
        assert_eq!(access_token.as_deref(), Some("custom-top-level-token"));
    }
}
