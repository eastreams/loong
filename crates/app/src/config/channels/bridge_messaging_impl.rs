use super::*;

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
            webhook_bind: account_override
                .and_then(|account| account.webhook_bind.clone())
                .or_else(|| self.webhook_bind.clone()),
            webhook_path: account_override
                .and_then(|account| account.webhook_path.clone())
                .or_else(|| self.webhook_path.clone()),
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
            webhook_bind: merged.webhook_bind,
            webhook_path: merged.webhook_path,
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

impl TlonChannelConfig {
    pub(crate) fn validate(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = Vec::new();
        validate_channel_account_integrity(
            &mut issues,
            "tlon",
            self.default_account.as_deref(),
            self.accounts.keys(),
        );
        tlon_support::validate_tlon_env_pointer(
            &mut issues,
            "tlon.ship_env",
            self.ship_env.as_deref(),
            "tlon.ship",
        );
        tlon_support::validate_tlon_env_pointer(
            &mut issues,
            "tlon.url_env",
            self.url_env.as_deref(),
            "tlon.url",
        );
        tlon_support::validate_tlon_env_pointer(
            &mut issues,
            "tlon.code_env",
            self.code_env.as_deref(),
            "tlon.code",
        );
        tlon_support::validate_tlon_secret_ref_env_pointer(
            &mut issues,
            "tlon.code",
            self.code.as_ref(),
        );

        for (raw_account_id, account) in &self.accounts {
            let account_id = normalize_channel_account_id(raw_account_id);

            let ship_field_path = format!("tlon.accounts.{account_id}.ship");
            let ship_env_field_path = format!("{ship_field_path}_env");
            tlon_support::validate_tlon_env_pointer(
                &mut issues,
                ship_env_field_path.as_str(),
                account.ship_env.as_deref(),
                ship_field_path.as_str(),
            );

            let url_field_path = format!("tlon.accounts.{account_id}.url");
            let url_env_field_path = format!("{url_field_path}_env");
            tlon_support::validate_tlon_env_pointer(
                &mut issues,
                url_env_field_path.as_str(),
                account.url_env.as_deref(),
                url_field_path.as_str(),
            );

            let code_field_path = format!("tlon.accounts.{account_id}.code");
            let code_env_field_path = format!("{code_field_path}_env");
            tlon_support::validate_tlon_env_pointer(
                &mut issues,
                code_env_field_path.as_str(),
                account.code_env.as_deref(),
                code_field_path.as_str(),
            );
            tlon_support::validate_tlon_secret_ref_env_pointer(
                &mut issues,
                code_field_path.as_str(),
                account.code.as_ref(),
            );
        }

        issues
    }

    pub fn ship(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.ship.as_deref(), self.ship_env.as_deref())
    }

    pub fn url(&self) -> Option<String> {
        resolve_string_with_legacy_env(self.url.as_deref(), self.url_env.as_deref())
    }

    pub fn code(&self) -> Option<String> {
        resolve_secret_with_legacy_env(self.code.as_ref(), self.code_env.as_deref())
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
        let selection = self.default_configured_account_selection();
        selection.id
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
    ) -> CliResult<ResolvedTlonChannelConfig> {
        let configured = self.resolve_configured_account_selection(requested_account_id)?;
        let account_override = configured
            .account_key
            .as_deref()
            .and_then(|key| self.accounts.get(key));

        let merged = TlonChannelConfig {
            enabled: self.enabled
                && account_override
                    .and_then(|account| account.enabled)
                    .unwrap_or(true),
            account_id: account_override
                .and_then(|account| account.account_id.clone())
                .or_else(|| self.account_id.clone()),
            default_account: None,
            ship: account_override
                .and_then(|account| account.ship.clone())
                .or_else(|| self.ship.clone()),
            ship_env: account_override
                .and_then(|account| account.ship_env.clone())
                .or_else(|| self.ship_env.clone()),
            url: account_override
                .and_then(|account| account.url.clone())
                .or_else(|| self.url.clone()),
            url_env: account_override
                .and_then(|account| account.url_env.clone())
                .or_else(|| self.url_env.clone()),
            code: account_override
                .and_then(|account| account.code.clone())
                .or_else(|| self.code.clone()),
            code_env: account_override
                .and_then(|account| account.code_env.clone())
                .or_else(|| self.code_env.clone()),
            accounts: BTreeMap::new(),
        };
        let account = merged.resolved_account_identity();

        Ok(ResolvedTlonChannelConfig {
            configured_account_id: configured.id,
            configured_account_label: configured.label,
            account,
            enabled: merged.enabled,
            ship: merged.ship,
            ship_env: merged.ship_env,
            url: merged.url,
            url_env: merged.url_env,
            code: merged.code,
            code_env: merged.code_env,
        })
    }

    pub fn resolve_account_for_session_account_id(
        &self,
        session_account_id: Option<&str>,
    ) -> CliResult<ResolvedTlonChannelConfig> {
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

        let ship = self.ship();
        let ship = ship
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(ship) = ship {
            let trimmed_ship = ship.trim_start_matches('~');
            let normalized_ship = normalize_channel_account_id(trimmed_ship);
            let account_id = format!("tlon_{normalized_ship}");
            let account_label = format!("ship:{ship}");
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
