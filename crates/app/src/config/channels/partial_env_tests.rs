use super::*;

use serde_json::json;

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
fn webhook_partial_deserialization_keeps_default_env_pointers() {
    let config: WebhookChannelConfig = serde_json::from_value(json!({
        "enabled": true
    }))
    .expect("deserialize webhook config");

    assert_eq!(
        config.endpoint_url_env.as_deref(),
        Some(WEBHOOK_ENDPOINT_URL_ENV)
    );
    assert_eq!(
        config.auth_token_env.as_deref(),
        Some(WEBHOOK_AUTH_TOKEN_ENV)
    );
    assert_eq!(
        config.signing_secret_env.as_deref(),
        Some(WEBHOOK_SIGNING_SECRET_ENV)
    );
    assert_eq!(config.auth_header_name, "Authorization");
    assert_eq!(config.auth_token_prefix, "Bearer ");
    assert_eq!(config.payload_format, WebhookPayloadFormat::JsonText);
    assert_eq!(config.payload_text_field, "text");
}

#[test]
fn teams_partial_deserialization_keeps_default_env_pointers() {
    let config: TeamsChannelConfig = serde_json::from_value(json!({
        "enabled": true
    }))
    .expect("deserialize teams config");

    assert_eq!(
        config.webhook_url_env.as_deref(),
        Some(TEAMS_WEBHOOK_URL_ENV)
    );
    assert_eq!(config.app_id_env.as_deref(), Some(TEAMS_APP_ID_ENV));
    assert_eq!(
        config.app_password_env.as_deref(),
        Some(TEAMS_APP_PASSWORD_ENV)
    );
    assert_eq!(config.tenant_id_env.as_deref(), Some(TEAMS_TENANT_ID_ENV));
}
