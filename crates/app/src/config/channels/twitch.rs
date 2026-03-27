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
