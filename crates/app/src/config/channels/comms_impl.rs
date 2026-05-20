use super::*;

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
        validate_discord_env_pointer(
            &mut issues,
            "discord.application_id_env",
            self.application_id_env.as_deref(),
            "discord.application_id",
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
            let application_id_field_path = format!("discord.accounts.{account_id}.application_id");
            let application_id_env_field_path = format!("{application_id_field_path}_env");
            validate_discord_env_pointer(
                &mut issues,
                application_id_env_field_path.as_str(),
                account.application_id_env.as_deref(),
                application_id_field_path.as_str(),
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
            application_id: account_override
                .and_then(|account| account.application_id.clone())
                .or_else(|| self.application_id.clone()),
            application_id_env: account_override
                .and_then(|account| account.application_id_env.clone())
                .or_else(|| self.application_id_env.clone()),
            allowed_guild_ids: account_override
                .and_then(|account| account.allowed_guild_ids.clone())
                .unwrap_or_else(|| self.allowed_guild_ids.clone()),
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
            application_id: merged.application_id,
            application_id_env: merged.application_id_env,
            allowed_guild_ids: merged.allowed_guild_ids,
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
