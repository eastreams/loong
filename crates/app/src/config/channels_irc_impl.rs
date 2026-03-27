use super::*;

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
        let resolved_nickname = self.nickname();
        validate_irc_nickname_field(&mut issues, "irc.nickname", resolved_nickname);
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
            let nickname = resolve_string_with_legacy_env(
                account.nickname.as_deref(),
                account.nickname_env.as_deref(),
            );
            validate_irc_nickname_field(&mut issues, nickname_field_path.as_str(), nickname);

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn validate_reports_whitespace_in_top_level_irc_nickname() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_IRC_NICKNAME", "loong\tclaw");

        let config = IrcChannelConfig {
            nickname_env: Some("TEST_IRC_NICKNAME".to_owned()),
            ..IrcChannelConfig::default()
        };

        let issues = config.validate();
        let nickname_issue = issues
            .iter()
            .find(|issue| issue.field_path == "irc.nickname")
            .expect("nickname validation issue");
        let invalid_reason = nickname_issue
            .extra_message_variables
            .get("invalid_reason")
            .expect("invalid reason");

        assert_eq!(invalid_reason, "nickname must not contain whitespace");
    }

    #[test]
    fn validate_reports_whitespace_in_account_irc_nickname() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("TEST_ACCOUNT_IRC_NICKNAME", "ops\tbot");

        let config_value = json!({
            "accounts": {
                "Ops": {
                    "nickname_env": "TEST_ACCOUNT_IRC_NICKNAME"
                }
            }
        });
        let config: IrcChannelConfig =
            serde_json::from_value(config_value).expect("deserialize irc config");

        let issues = config.validate();
        let nickname_issue = issues
            .iter()
            .find(|issue| issue.field_path == "irc.accounts.ops.nickname")
            .expect("account nickname validation issue");
        let invalid_reason = nickname_issue
            .extra_message_variables
            .get("invalid_reason")
            .expect("invalid reason");

        assert_eq!(invalid_reason, "nickname must not contain whitespace");
    }
}
