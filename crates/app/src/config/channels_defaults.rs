use super::*;

pub(super) fn default_telegram_base_url() -> String {
    "https://api.telegram.org".to_owned()
}

pub(super) const fn default_telegram_timeout_seconds() -> u64 {
    15
}

pub(super) const fn default_true() -> bool {
    true
}

pub(super) fn default_feishu_receive_id_type() -> String {
    "chat_id".to_owned()
}

pub(super) fn default_feishu_webhook_bind() -> String {
    "127.0.0.1:8080".to_owned()
}

pub(super) fn default_feishu_webhook_path() -> String {
    "/feishu/events".to_owned()
}

pub(super) const fn default_matrix_sync_timeout_seconds() -> u64 {
    30
}

pub(super) fn default_wecom_websocket_url() -> String {
    "wss://openws.work.weixin.qq.com".to_owned()
}

pub(super) const fn default_wecom_ping_interval_seconds() -> u64 {
    30
}

pub(super) const fn default_wecom_reconnect_interval_seconds() -> u64 {
    5
}

pub(super) fn default_discord_api_base_url() -> String {
    "https://discord.com/api/v10".to_owned()
}

pub(super) fn default_discord_bot_token_env() -> Option<String> {
    Some(DISCORD_BOT_TOKEN_ENV.to_owned())
}

pub(super) fn default_line_api_base_url() -> String {
    "https://api.line.me/v2/bot".to_owned()
}

pub(super) fn default_email_smtp_username_env() -> Option<String> {
    Some(EMAIL_SMTP_USERNAME_ENV.to_owned())
}

pub(super) fn default_email_smtp_password_env() -> Option<String> {
    Some(EMAIL_SMTP_PASSWORD_ENV.to_owned())
}

pub(super) fn default_email_imap_username_env() -> Option<String> {
    Some(EMAIL_IMAP_USERNAME_ENV.to_owned())
}

pub(super) fn default_email_imap_password_env() -> Option<String> {
    Some(EMAIL_IMAP_PASSWORD_ENV.to_owned())
}

pub(super) fn default_webhook_endpoint_url_env() -> Option<String> {
    Some(WEBHOOK_ENDPOINT_URL_ENV.to_owned())
}

pub(super) fn default_webhook_auth_token_env() -> Option<String> {
    Some(WEBHOOK_AUTH_TOKEN_ENV.to_owned())
}

pub(super) fn default_webhook_signing_secret_env() -> Option<String> {
    Some(WEBHOOK_SIGNING_SECRET_ENV.to_owned())
}

pub(super) fn default_webhook_auth_header_name() -> String {
    "Authorization".to_owned()
}

pub(super) fn default_webhook_auth_token_prefix() -> String {
    "Bearer ".to_owned()
}

pub(super) fn default_webhook_payload_text_field() -> String {
    "text".to_owned()
}

pub(super) fn default_teams_webhook_url_env() -> Option<String> {
    Some(TEAMS_WEBHOOK_URL_ENV.to_owned())
}

pub(super) fn default_teams_app_id_env() -> Option<String> {
    Some(TEAMS_APP_ID_ENV.to_owned())
}

pub(super) fn default_teams_app_password_env() -> Option<String> {
    Some(TEAMS_APP_PASSWORD_ENV.to_owned())
}

pub(super) fn default_teams_tenant_id_env() -> Option<String> {
    Some(TEAMS_TENANT_ID_ENV.to_owned())
}

pub(super) fn default_imessage_bridge_url_env() -> Option<String> {
    Some(IMESSAGE_BRIDGE_URL_ENV.to_owned())
}

pub(super) fn default_imessage_bridge_token_env() -> Option<String> {
    Some(IMESSAGE_BRIDGE_TOKEN_ENV.to_owned())
}

pub(super) fn default_slack_api_base_url() -> String {
    "https://slack.com/api".to_owned()
}

pub(super) fn default_slack_bot_token_env() -> Option<String> {
    Some(SLACK_BOT_TOKEN_ENV.to_owned())
}

pub(super) fn default_whatsapp_api_base_url() -> String {
    "https://graph.facebook.com/v25.0".to_owned()
}

pub(super) fn default_whatsapp_access_token_env() -> Option<String> {
    Some(WHATSAPP_ACCESS_TOKEN_ENV.to_owned())
}

pub(super) fn default_whatsapp_phone_number_id_env() -> Option<String> {
    Some(WHATSAPP_PHONE_NUMBER_ID_ENV.to_owned())
}

pub(super) fn default_whatsapp_verify_token_env() -> Option<String> {
    Some(WHATSAPP_VERIFY_TOKEN_ENV.to_owned())
}

pub(super) fn default_whatsapp_app_secret_env() -> Option<String> {
    Some(WHATSAPP_APP_SECRET_ENV.to_owned())
}

pub(super) fn default_system_prompt() -> String {
    render_default_system_prompt()
}

pub(super) fn default_prompt_pack_id() -> Option<String> {
    Some(DEFAULT_PROMPT_PACK_ID.to_owned())
}

pub(super) fn default_prompt_personality() -> Option<PromptPersonality> {
    Some(PromptPersonality::default())
}

pub(super) fn default_exit_commands() -> Vec<String> {
    vec!["/exit".to_owned(), "/quit".to_owned()]
}
