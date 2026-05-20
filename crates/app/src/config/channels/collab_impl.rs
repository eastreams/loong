use super::*;

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
            allowed_sender_ids: account_override
                .and_then(|account| account.allowed_sender_ids.clone())
                .unwrap_or_else(|| self.allowed_sender_ids.clone()),
            ack_reactions: account_override
                .and_then(|account| account.ack_reactions)
                .unwrap_or(self.ack_reactions),
            ignore_bot_messages: account_override
                .and_then(|account| account.ignore_bot_messages)
                .unwrap_or(self.ignore_bot_messages),
            require_mention: account_override
                .and_then(|account| account.require_mention)
                .unwrap_or(self.require_mention),
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
            mode: merged.mode.unwrap_or(FeishuChannelServeMode::Websocket),
            receive_id_type: merged.receive_id_type,
            webhook_bind: merged.webhook_bind,
            webhook_path: merged.webhook_path,
            verification_token: merged.verification_token,
            verification_token_env: merged.verification_token_env,
            encrypt_key: merged.encrypt_key,
            encrypt_key_env: merged.encrypt_key_env,
            allowed_chat_ids: merged.allowed_chat_ids,
            allowed_sender_ids: merged.allowed_sender_ids,
            ack_reactions: merged.ack_reactions,
            ignore_bot_messages: merged.ignore_bot_messages,
            require_mention: merged.require_mention,
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
            allowed_sender_ids: account_override
                .and_then(|account| account.allowed_sender_ids.clone())
                .unwrap_or_else(|| self.allowed_sender_ids.clone()),
            require_mention: account_override
                .and_then(|account| account.require_mention)
                .unwrap_or(self.require_mention),
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
            allowed_sender_ids: merged.allowed_sender_ids,
            require_mention: merged.require_mention,
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
            allowed_sender_ids: account_override
                .and_then(|account| account.allowed_sender_ids.clone())
                .unwrap_or_else(|| self.allowed_sender_ids.clone()),
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
            allowed_sender_ids: merged.allowed_sender_ids,
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

impl QqbotChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "qqbot",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        validate_effective_qqbot_runtime_account_ids(&mut issues, self);
        validate_qqbot_env_pointer(
            &mut issues,
            "qqbot.app_id_env",
            self.app_id_env.as_deref(),
            "qqbot.app_id",
        );
        validate_qqbot_secret_ref_env_pointer(&mut issues, "qqbot.app_id", self.app_id.as_ref());
        validate_qqbot_env_pointer(
            &mut issues,
            "qqbot.client_secret_env",
            self.client_secret_env.as_deref(),
            "qqbot.client_secret",
        );
        validate_qqbot_secret_ref_env_pointer(
            &mut issues,
            "qqbot.client_secret",
            self.client_secret.as_ref(),
        );

        for (raw_account_id, account) in &self.accounts {
            let account_id = raw_account_id.as_str();
            let app_id_field_path = format!("qqbot.accounts.{account_id}.app_id");
            let app_id_env_field_path = format!("{app_id_field_path}_env");
            validate_qqbot_env_pointer(
                &mut issues,
                app_id_env_field_path.as_str(),
                account.app_id_env.as_deref(),
                app_id_field_path.as_str(),
            );
            validate_qqbot_secret_ref_env_pointer(
                &mut issues,
                app_id_field_path.as_str(),
                account.app_id.as_ref(),
            );

            let secret_field_path = format!("qqbot.accounts.{account_id}.client_secret");
            let secret_env_field_path = format!("{secret_field_path}_env");
            validate_qqbot_env_pointer(
                &mut issues,
                secret_env_field_path.as_str(),
                account.client_secret_env.as_deref(),
                secret_field_path.as_str(),
            );
            validate_qqbot_secret_ref_env_pointer(
                &mut issues,
                secret_field_path.as_str(),
                account.client_secret.as_ref(),
            );
        }

        issues
    }

    pub fn app_id(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.app_id.as_ref(), self.app_id_env.as_deref())
    }

    pub fn client_secret(&self) -> Option<String> {
        resolve_secret_with_legacy_env(
            self.client_secret.as_ref(),
            self.client_secret_env.as_deref(),
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
        let fallback_account_id = self.resolved_account_identity().id;
        resolve_default_configured_account_selection(
            self.accounts.keys(),
            self.default_account.as_deref(),
            fallback_account_id.as_str(),
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
        let fallback_account_id = self.resolved_account_identity().id;
        resolve_channel_account_route(
            self.accounts.keys(),
            self.default_account.as_deref(),
            fallback_account_id.as_str(),
            requested_account_id,
            selected_configured_account_id,
        )
    }

    pub fn resolve_account(
        &self,
        requested_account_id: Option<&str>,
    ) -> CliResult<ResolvedQqbotChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = QqbotChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            managed_bridge_plugin_id: self.managed_bridge_plugin_id.clone(),
            app_id: account_override
                .and_then(|account| account.app_id.clone())
                .or_else(|| self.app_id.clone()),
            app_id_env: account_override
                .and_then(|account| account.app_id_env.clone())
                .or_else(|| self.app_id_env.clone()),
            client_secret: account_override
                .and_then(|account| account.client_secret.clone())
                .or_else(|| self.client_secret.clone()),
            client_secret_env: account_override
                .and_then(|account| account.client_secret_env.clone())
                .or_else(|| self.client_secret_env.clone()),
            allowed_peer_ids: account_override
                .and_then(|account| account.allowed_peer_ids.clone())
                .unwrap_or_else(|| self.allowed_peer_ids.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedQqbotChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            app_id: merged.app_id,
            app_id_env: merged.app_id_env,
            client_secret: merged.client_secret,
            client_secret_env: merged.client_secret_env,
            allowed_peer_ids: merged.allowed_peer_ids,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedQqbotChannelConfig> {
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

        let app_id = self.app_id();
        let app_id = app_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(app_id) = app_id {
            let normalized_account_id = normalize_channel_account_id(app_id);
            let account_id = format!("qqbot_{normalized_account_id}");
            let account_label = format!("qqbot:{app_id}");
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
        let fallback_account_id = self.resolved_account_identity().id;
        resolve_configured_account_selection(
            self.accounts.keys(),
            requested_account_id,
            self.default_account.as_deref(),
            fallback_account_id.as_str(),
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
                    "Use a valid RFC 5322 mailbox like `ops@example.com` or `Loong <ops@example.com>`.",
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
                        "Use a valid RFC 5322 mailbox like `ops@example.com` or `Loong <ops@example.com>`.",
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
