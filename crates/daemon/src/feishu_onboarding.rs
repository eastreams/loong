use std::collections::HashMap;
use std::time::Duration;

use loong_contracts::SecretRef;
use qrcode::QrCode;
use qrcode::render::unicode;
use serde_json::Value;

use crate::CliResult;
use crate::configured_account_keys::resolve_raw_configured_account_key;
use crate::feishu_support::load_feishu_daemon_context;
use crate::mvp;

const FEISHU_ACCOUNTS_BASE_URL: &str = "https://accounts.feishu.cn";
const LARK_ACCOUNTS_BASE_URL: &str = "https://accounts.larksuite.com";
const REGISTRATION_PATH: &str = "/oauth/v1/app/registration";
const DEFAULT_ONBOARD_TIMEOUT_S: u64 = 600;
const DEFAULT_ONBOARD_REQUEST_TIMEOUT_S: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeishuOnboardCredentialSource {
    QrRegistration,
    Manual,
}

impl FeishuOnboardCredentialSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QrRegistration => "qr_registration",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeishuOnboardApplyOptions {
    pub domain: mvp::config::FeishuDomain,
    pub mode: mvp::config::FeishuChannelServeMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuOnboardCredentials {
    pub app_id: String,
    pub app_secret: String,
    pub verification_token: Option<String>,
    pub encrypt_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuQrRegistrationResult {
    pub app_id: String,
    pub app_secret: String,
    pub domain: mvp::config::FeishuDomain,
    pub open_id: Option<String>,
    pub bot_name: Option<String>,
    pub bot_open_id: Option<String>,
    pub qr_url: String,
    pub qr_rendered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuOnboardResult {
    pub config_path: String,
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub runtime_account_id: String,
    pub domain: mvp::config::FeishuDomain,
    pub mode: mvp::config::FeishuChannelServeMode,
    pub credential_source: FeishuOnboardCredentialSource,
    pub owner_open_id: Option<String>,
    pub bot_name: Option<String>,
    pub bot_open_id: Option<String>,
    pub qr_url: Option<String>,
    pub qr_rendered: bool,
    pub owner_direct_chat_bootstrap_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuRegistrationBegin {
    device_code: String,
    qr_url: String,
    interval_s: u64,
    expire_in_s: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuOnboardingUrls {
    feishu_accounts_base_url: String,
    lark_accounts_base_url: String,
    feishu_open_base_url: String,
    lark_open_base_url: String,
}

impl Default for FeishuOnboardingUrls {
    fn default() -> Self {
        Self {
            feishu_accounts_base_url: FEISHU_ACCOUNTS_BASE_URL.to_owned(),
            lark_accounts_base_url: LARK_ACCOUNTS_BASE_URL.to_owned(),
            feishu_open_base_url: mvp::config::FeishuDomain::Feishu
                .default_base_url()
                .to_owned(),
            lark_open_base_url: mvp::config::FeishuDomain::Lark
                .default_base_url()
                .to_owned(),
        }
    }
}

impl FeishuOnboardingUrls {
    fn accounts_base_url(&self, domain: mvp::config::FeishuDomain) -> &str {
        match domain {
            mvp::config::FeishuDomain::Feishu => self.feishu_accounts_base_url.as_str(),
            mvp::config::FeishuDomain::Lark => self.lark_accounts_base_url.as_str(),
        }
    }

    fn open_base_url(&self, domain: mvp::config::FeishuDomain) -> &str {
        match domain {
            mvp::config::FeishuDomain::Feishu => self.feishu_open_base_url.as_str(),
            mvp::config::FeishuDomain::Lark => self.lark_open_base_url.as_str(),
        }
    }
}

pub async fn onboard_via_qr_registration(
    config_path: Option<&str>,
    account: Option<&str>,
    requested_domain: mvp::config::FeishuDomain,
    timeout_s: Option<u64>,
    mode: Option<mvp::config::FeishuChannelServeMode>,
) -> CliResult<FeishuOnboardResult> {
    let urls = FeishuOnboardingUrls::default();
    let result = qr_register_with_urls(
        requested_domain,
        timeout_s.unwrap_or(DEFAULT_ONBOARD_TIMEOUT_S),
        &urls,
    )
    .await?
    .ok_or_else(|| "Feishu/Lark QR registration did not complete".to_owned())?;

    let credentials = FeishuOnboardCredentials {
        app_id: result.app_id.clone(),
        app_secret: result.app_secret.clone(),
        verification_token: None,
        encrypt_key: None,
    };
    apply_onboard_result_to_config(
        config_path,
        account,
        &credentials,
        FeishuOnboardApplyOptions {
            domain: result.domain,
            mode: mode.unwrap_or(mvp::config::FeishuChannelServeMode::Websocket),
        },
        FeishuOnboardCredentialSource::QrRegistration,
        result.open_id.clone(),
        result.bot_name.clone(),
        result.bot_open_id.clone(),
        Some(result.qr_url.clone()),
        result.qr_rendered,
    )
}

pub fn apply_manual_feishu_onboarding(
    config_path: Option<&str>,
    account: Option<&str>,
    credentials: &FeishuOnboardCredentials,
    options: FeishuOnboardApplyOptions,
) -> CliResult<FeishuOnboardResult> {
    apply_onboard_result_to_config(
        config_path,
        account,
        credentials,
        options,
        FeishuOnboardCredentialSource::Manual,
        None,
        None,
        None,
        None,
        false,
    )
}

pub fn render_qr_instructions(url: &str, qr_rendered: bool) -> Vec<String> {
    if qr_rendered {
        return vec![
            "Scan the QR code above with Feishu/Lark on your phone,".to_owned(),
            format!("or open this link on your desktop: {url}"),
            "The command will keep polling until Feishu/Lark returns the generated bot credentials."
                .to_owned(),
        ];
    }

    vec![format!(
        "Open this link in Feishu/Lark to activate the bot: {url}"
    )]
}

fn apply_onboard_result_to_config(
    config_path: Option<&str>,
    account: Option<&str>,
    credentials: &FeishuOnboardCredentials,
    options: FeishuOnboardApplyOptions,
    credential_source: FeishuOnboardCredentialSource,
    owner_open_id: Option<String>,
    bot_name: Option<String>,
    bot_open_id: Option<String>,
    qr_url: Option<String>,
    qr_rendered: bool,
) -> CliResult<FeishuOnboardResult> {
    let context = load_feishu_daemon_context(config_path, account)?;
    let mut config = context.config.clone();
    let configured_account_id = context.resolved.configured_account_id.clone();
    let configured_account_label = context.resolved.configured_account_label.clone();
    let runtime_account_id = context.account_id().to_owned();

    apply_credentials_to_selected_account(
        &mut config.feishu,
        configured_account_id.as_str(),
        credentials,
        options,
    );
    let owner_direct_chat_bootstrap_applied = apply_owner_bootstrap_access(
        &mut config.feishu,
        configured_account_id.as_str(),
        credential_source,
        owner_open_id.as_deref(),
    );

    let config_path_string = context.config_path.display().to_string();
    let saved_path = mvp::config::write(Some(config_path_string.as_str()), &config, true)?;

    Ok(FeishuOnboardResult {
        config_path: saved_path.display().to_string(),
        configured_account_id,
        configured_account_label,
        runtime_account_id,
        domain: options.domain,
        mode: options.mode,
        credential_source,
        owner_open_id,
        bot_name,
        bot_open_id,
        qr_url,
        qr_rendered,
        owner_direct_chat_bootstrap_applied,
    })
}

fn apply_credentials_to_selected_account(
    channel: &mut mvp::config::FeishuChannelConfig,
    configured_account_id: &str,
    credentials: &FeishuOnboardCredentials,
    options: FeishuOnboardApplyOptions,
) {
    let raw_account_key =
        resolve_raw_configured_account_key(channel.accounts.keys(), configured_account_id);
    if let Some(raw_account_key) = raw_account_key.as_deref()
        && let Some(account) = channel.accounts.get_mut(raw_account_key)
    {
        account.enabled = Some(true);
        account.app_id = Some(SecretRef::Inline(credentials.app_id.clone()));
        account.app_secret = Some(SecretRef::Inline(credentials.app_secret.clone()));
        account.app_id_env = None;
        account.app_secret_env = None;
        account.domain = Some(options.domain);
        account.mode = Some(options.mode);
        if options.mode == mvp::config::FeishuChannelServeMode::Webhook {
            account.verification_token = credentials
                .verification_token
                .clone()
                .map(SecretRef::Inline);
            account.encrypt_key = credentials.encrypt_key.clone().map(SecretRef::Inline);
            account.verification_token_env = None;
            account.encrypt_key_env = None;
        }
        return;
    }

    channel.enabled = true;
    channel.app_id = Some(SecretRef::Inline(credentials.app_id.clone()));
    channel.app_secret = Some(SecretRef::Inline(credentials.app_secret.clone()));
    channel.app_id_env = None;
    channel.app_secret_env = None;
    channel.domain = options.domain;
    channel.mode = Some(options.mode);
    if options.mode == mvp::config::FeishuChannelServeMode::Webhook {
        channel.verification_token = credentials
            .verification_token
            .clone()
            .map(SecretRef::Inline);
        channel.encrypt_key = credentials.encrypt_key.clone().map(SecretRef::Inline);
        channel.verification_token_env = None;
        channel.encrypt_key_env = None;
    }
}

fn apply_owner_bootstrap_access(
    channel: &mut mvp::config::FeishuChannelConfig,
    configured_account_id: &str,
    credential_source: FeishuOnboardCredentialSource,
    owner_open_id: Option<&str>,
) -> bool {
    if credential_source != FeishuOnboardCredentialSource::QrRegistration {
        return false;
    }
    let Some(owner_open_id) = owner_open_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    let raw_account_key =
        resolve_raw_configured_account_key(channel.accounts.keys(), configured_account_id);
    if let Some(raw_account_key) = raw_account_key.as_deref()
        && let Some(account) = channel.accounts.get_mut(raw_account_key)
    {
        let chats_unset = account
            .allowed_chat_ids
            .as_ref()
            .is_none_or(|values| values.is_empty());
        let senders_unset = account
            .allowed_sender_ids
            .as_ref()
            .is_none_or(|values| values.is_empty());
        if !chats_unset || !senders_unset {
            return false;
        }
        account.allowed_chat_ids = Some(vec!["*".to_owned()]);
        account.allowed_sender_ids = Some(vec![owner_open_id.to_owned()]);
        return true;
    }

    if !channel.allowed_chat_ids.is_empty() || !channel.allowed_sender_ids.is_empty() {
        return false;
    }
    channel.allowed_chat_ids = vec!["*".to_owned()];
    channel.allowed_sender_ids = vec![owner_open_id.to_owned()];
    true
}

async fn qr_register_with_urls(
    initial_domain: mvp::config::FeishuDomain,
    timeout_s: u64,
    urls: &FeishuOnboardingUrls,
) -> CliResult<Option<FeishuQrRegistrationResult>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_ONBOARD_REQUEST_TIMEOUT_S))
        .build()
        .map_err(|error| format!("build Feishu/Lark onboarding client failed: {error}"))?;

    print!("  Connecting to Feishu / Lark...");
    _init_registration(&client, initial_domain, urls).await?;
    let begin = _begin_registration(&client, initial_domain, urls).await?;
    println!(" done.");
    println!();

    let qr_rendered = render_terminal_qr(begin.qr_url.as_str());
    for line in render_qr_instructions(begin.qr_url.as_str(), qr_rendered) {
        println!("  {line}");
    }
    println!();

    let mut result = match _poll_registration(
        &client,
        begin.device_code.as_str(),
        begin.interval_s,
        begin.expire_in_s.min(timeout_s),
        initial_domain,
        urls,
    )
    .await?
    {
        Some(result) => result,
        None => return Ok(None),
    };

    result.qr_url = begin.qr_url;
    result.qr_rendered = qr_rendered;
    Ok(Some(result))
}

async fn _init_registration(
    client: &reqwest::Client,
    domain: mvp::config::FeishuDomain,
    urls: &FeishuOnboardingUrls,
) -> CliResult<()> {
    let payload = post_registration(
        client,
        urls.accounts_base_url(domain),
        &[("action", "init")],
    )
    .await?;
    let methods = payload
        .get("supported_auth_methods")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if methods.contains(&"client_secret") {
        return Ok(());
    }
    Err(format!(
        "Feishu/Lark registration environment does not support client_secret auth. Supported: {}",
        if methods.is_empty() {
            "-".to_owned()
        } else {
            methods.join(", ")
        }
    ))
}

async fn _begin_registration(
    client: &reqwest::Client,
    domain: mvp::config::FeishuDomain,
    urls: &FeishuOnboardingUrls,
) -> CliResult<FeishuRegistrationBegin> {
    let payload = post_registration(
        client,
        urls.accounts_base_url(domain),
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;

    let device_code = required_string(&payload, "device_code")?;
    let mut qr_url = required_string(&payload, "verification_uri_complete")?;
    if qr_url.contains('?') {
        qr_url.push_str("&from=loong&tp=loong");
    } else {
        qr_url.push_str("?from=loong&tp=loong");
    }

    Ok(FeishuRegistrationBegin {
        device_code,
        qr_url,
        interval_s: optional_u64(&payload, "interval").unwrap_or(5),
        expire_in_s: optional_u64(&payload, "expire_in").unwrap_or(DEFAULT_ONBOARD_TIMEOUT_S),
    })
}

async fn _poll_registration(
    client: &reqwest::Client,
    device_code: &str,
    interval_s: u64,
    expire_in_s: u64,
    initial_domain: mvp::config::FeishuDomain,
    urls: &FeishuOnboardingUrls,
) -> CliResult<Option<FeishuQrRegistrationResult>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expire_in_s.max(1));
    let mut current_domain = initial_domain;
    let mut poll_count = 0_u64;
    let sleep_duration = Duration::from_secs(interval_s.max(1));

    loop {
        if tokio::time::Instant::now() >= deadline {
            if poll_count > 0 {
                println!();
            }
            return Ok(None);
        }

        let payload = match post_registration(
            client,
            urls.accounts_base_url(current_domain),
            &[
                ("action", "poll"),
                ("device_code", device_code),
                ("tp", "ob_app"),
            ],
        )
        .await
        {
            Ok(payload) => payload,
            Err(_) => {
                tokio::time::sleep(sleep_duration).await;
                continue;
            }
        };

        poll_count = poll_count.saturating_add(1);
        if poll_count == 1 {
            print!("  Fetching configuration results...");
        } else if poll_count.is_multiple_of(6) {
            print!(".");
        }

        if let Some(tenant_brand) = payload
            .get("user_info")
            .and_then(Value::as_object)
            .and_then(|user_info| user_info.get("tenant_brand"))
            .and_then(Value::as_str)
            && tenant_brand == "lark"
        {
            current_domain = mvp::config::FeishuDomain::Lark;
        }

        let app_id = payload
            .get("client_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let app_secret = payload
            .get("client_secret")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let (Some(app_id), Some(app_secret)) = (app_id, app_secret) {
            println!();
            let user_info = payload.get("user_info").and_then(Value::as_object);
            let open_id = user_info
                .and_then(|value| value.get("open_id"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let mut result = FeishuQrRegistrationResult {
                app_id: app_id.clone(),
                app_secret: app_secret.clone(),
                domain: current_domain,
                open_id,
                bot_name: None,
                bot_open_id: None,
                qr_url: String::new(),
                qr_rendered: false,
            };
            if let Some(bot_info) =
                probe_bot_with_urls(app_id.as_str(), app_secret.as_str(), current_domain, urls)
                    .await?
            {
                result.bot_name = bot_info.get("bot_name").cloned();
                result.bot_open_id = bot_info.get("bot_open_id").cloned();
            }
            return Ok(Some(result));
        }

        let error = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if error == "access_denied" || error == "expired_token" {
            println!();
            return Ok(None);
        }

        tokio::time::sleep(sleep_duration).await;
    }
}

async fn probe_bot_with_urls(
    app_id: &str,
    app_secret: &str,
    domain: mvp::config::FeishuDomain,
    urls: &FeishuOnboardingUrls,
) -> CliResult<Option<HashMap<String, String>>> {
    let client = mvp::channel::feishu::api::FeishuClient::new(
        urls.open_base_url(domain),
        app_id,
        app_secret,
        DEFAULT_ONBOARD_REQUEST_TIMEOUT_S as usize,
    )?;
    let tenant_access_token = match client.get_tenant_access_token().await {
        Ok(token) => token,
        Err(_) => return Ok(None),
    };
    let payload = match client
        .get_json(
            "/open-apis/bot/v3/info",
            Some(tenant_access_token.as_str()),
            &[],
        )
        .await
    {
        Ok(payload) => payload,
        Err(_) => return Ok(None),
    };
    Ok(parse_bot_probe_response(&payload))
}

fn parse_bot_probe_response(payload: &Value) -> Option<HashMap<String, String>> {
    let code = payload.get("code").and_then(Value::as_i64)?;
    if code != 0 {
        return None;
    }
    let bot = payload
        .get("bot")
        .or_else(|| payload.get("data").and_then(|data| data.get("bot")))?;
    let mut summary = HashMap::new();
    if let Some(bot_name) = bot.get("bot_name").and_then(Value::as_str) {
        summary.insert("bot_name".to_owned(), bot_name.to_owned());
    }
    if let Some(bot_open_id) = bot.get("open_id").and_then(Value::as_str) {
        summary.insert("bot_open_id".to_owned(), bot_open_id.to_owned());
    }
    Some(summary)
}

async fn post_registration(
    client: &reqwest::Client,
    base_url: &str,
    body: &[(&str, &str)],
) -> CliResult<Value> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), REGISTRATION_PATH);
    let response = client
        .post(url)
        .form(body)
        .send()
        .await
        .map_err(|error| format!("Feishu/Lark registration request failed: {error}"))?;
    let body_text = response
        .text()
        .await
        .map_err(|error| format!("read Feishu/Lark registration response failed: {error}"))?;
    serde_json::from_str(&body_text)
        .map_err(|error| format!("decode Feishu/Lark registration response failed: {error}"))
}

fn render_terminal_qr(url: &str) -> bool {
    let Some(rendered) = encode_terminal_qr(url) else {
        return false;
    };
    println!("{rendered}");
    true
}

fn encode_terminal_qr(url: &str) -> Option<String> {
    let code = QrCode::new(url.as_bytes()).ok()?;
    let rendered = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    let trimmed = rendered.trim().to_owned();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn required_string(payload: &Value, field: &str) -> CliResult<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("Feishu/Lark onboarding response missing `{field}`"))
}

fn optional_u64(payload: &Value, field: &str) -> Option<u64> {
    payload.get(field).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::extract::State;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct RegistrationServerState {
        poll_requests: Arc<Mutex<Vec<String>>>,
    }

    #[tokio::test]
    async fn begin_registration_appends_loong_tracking_query() {
        let state = RegistrationServerState::default();
        let base_url = spawn_registration_server(state).await;
        let urls = test_urls(base_url.as_str());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("client");

        let begin = _begin_registration(&client, mvp::config::FeishuDomain::Feishu, &urls)
            .await
            .expect("begin");

        assert_eq!(begin.device_code, "device-123");
        assert!(
            begin
                .qr_url
                .contains("https://scan.example/activate?device=device-123&from=loong&tp=loong")
        );
    }

    #[tokio::test]
    async fn poll_registration_switches_to_lark_when_tenant_brand_requests_it() {
        let state = RegistrationServerState::default();
        let base_url = spawn_registration_server(state).await;
        let bot_base_url = spawn_bot_probe_server().await;
        let urls = FeishuOnboardingUrls {
            feishu_accounts_base_url: base_url.clone(),
            lark_accounts_base_url: base_url,
            feishu_open_base_url: bot_base_url.clone(),
            lark_open_base_url: bot_base_url,
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("client");

        let result = _poll_registration(
            &client,
            "device-123",
            0,
            2,
            mvp::config::FeishuDomain::Feishu,
            &urls,
        )
        .await
        .expect("poll")
        .expect("registration result");

        assert_eq!(result.domain, mvp::config::FeishuDomain::Lark);
        assert_eq!(result.app_id, "cli_lark_123");
        assert_eq!(result.app_secret, "secret_lark_123");
        assert_eq!(result.open_id.as_deref(), Some("ou_owner_1"));
        assert_eq!(result.bot_name.as_deref(), Some("Loong Bot"));
        assert_eq!(result.bot_open_id.as_deref(), Some("ou_bot_1"));
    }

    #[test]
    fn render_qr_instructions_shows_both_scan_and_desktop_paths_when_qr_rendered() {
        let rendered = render_qr_instructions("https://scan.example/activate", true).join("\n");

        assert!(rendered.contains("Scan the QR code above"));
        assert!(rendered.contains("open this link on your desktop"));
        assert!(rendered.contains("https://scan.example/activate"));
    }

    #[test]
    fn render_qr_instructions_falls_back_to_link_only_when_qr_is_unavailable() {
        let rendered = render_qr_instructions("https://scan.example/activate", false).join("\n");

        assert!(rendered.contains("https://scan.example/activate"));
        assert!(!rendered.contains("qrencode"));
    }

    #[test]
    fn encode_terminal_qr_returns_non_empty_unicode_art() {
        let rendered =
            encode_terminal_qr("https://scan.example/activate?device=device-123&from=loong")
                .expect("qr encoding should succeed");

        assert!(!rendered.is_empty());
        assert!(rendered.lines().count() >= 10);
    }

    #[test]
    fn apply_credentials_updates_root_channel_when_no_named_account_exists() {
        let mut config = mvp::config::FeishuChannelConfig::default();
        apply_credentials_to_selected_account(
            &mut config,
            "feishu_cli_default",
            &FeishuOnboardCredentials {
                app_id: "cli_root_123".to_owned(),
                app_secret: "root_secret_123".to_owned(),
                verification_token: None,
                encrypt_key: None,
            },
            FeishuOnboardApplyOptions {
                domain: mvp::config::FeishuDomain::Lark,
                mode: mvp::config::FeishuChannelServeMode::Websocket,
            },
        );

        assert!(config.enabled);
        assert_eq!(config.domain, mvp::config::FeishuDomain::Lark);
        assert_eq!(
            config.mode,
            Some(mvp::config::FeishuChannelServeMode::Websocket)
        );
        assert_eq!(
            config
                .app_id
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("cli_root_123")
        );
        assert_eq!(
            config
                .app_secret
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("root_secret_123")
        );
        assert_eq!(config.app_id_env, None);
        assert_eq!(config.app_secret_env, None);
    }

    #[test]
    fn apply_credentials_updates_selected_named_account() {
        let mut config = mvp::config::FeishuChannelConfig {
            accounts: BTreeMap::from([(
                "work".to_owned(),
                mvp::config::FeishuAccountConfig::default(),
            )]),
            ..mvp::config::FeishuChannelConfig::default()
        };
        apply_credentials_to_selected_account(
            &mut config,
            "work",
            &FeishuOnboardCredentials {
                app_id: "cli_work_123".to_owned(),
                app_secret: "work_secret_123".to_owned(),
                verification_token: None,
                encrypt_key: None,
            },
            FeishuOnboardApplyOptions {
                domain: mvp::config::FeishuDomain::Feishu,
                mode: mvp::config::FeishuChannelServeMode::Websocket,
            },
        );

        let account = config.accounts.get("work").expect("named account");
        assert_eq!(account.enabled, Some(true));
        assert_eq!(
            account
                .app_id
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("cli_work_123")
        );
        assert_eq!(
            account
                .app_secret
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("work_secret_123")
        );
        assert_eq!(account.domain, Some(mvp::config::FeishuDomain::Feishu));
        assert_eq!(
            account.mode,
            Some(mvp::config::FeishuChannelServeMode::Websocket)
        );
    }

    #[test]
    fn apply_owner_bootstrap_access_updates_root_channel_for_qr_onboarding() {
        let mut config = mvp::config::FeishuChannelConfig::default();

        let applied = apply_owner_bootstrap_access(
            &mut config,
            "feishu_cli_default",
            FeishuOnboardCredentialSource::QrRegistration,
            Some("ou_owner_1"),
        );

        assert!(applied);
        assert_eq!(config.allowed_chat_ids, vec!["*".to_owned()]);
        assert_eq!(config.allowed_sender_ids, vec!["ou_owner_1".to_owned()]);
    }

    #[test]
    fn apply_owner_bootstrap_access_updates_named_account_for_qr_onboarding() {
        let mut config = mvp::config::FeishuChannelConfig {
            accounts: BTreeMap::from([(
                "work".to_owned(),
                mvp::config::FeishuAccountConfig::default(),
            )]),
            ..mvp::config::FeishuChannelConfig::default()
        };

        let applied = apply_owner_bootstrap_access(
            &mut config,
            "work",
            FeishuOnboardCredentialSource::QrRegistration,
            Some("ou_owner_1"),
        );

        assert!(applied);
        let account = config.accounts.get("work").expect("named account");
        assert_eq!(
            account.allowed_chat_ids.clone().unwrap_or_default(),
            vec!["*".to_owned()]
        );
        assert_eq!(
            account.allowed_sender_ids.clone().unwrap_or_default(),
            vec!["ou_owner_1".to_owned()]
        );
    }

    #[test]
    fn apply_credentials_to_selected_account_updates_display_label_named_account() {
        let mut config = mvp::config::FeishuChannelConfig {
            accounts: BTreeMap::from([(
                "Work Bot".to_owned(),
                mvp::config::FeishuAccountConfig::default(),
            )]),
            ..mvp::config::FeishuChannelConfig::default()
        };

        apply_credentials_to_selected_account(
            &mut config,
            "work-bot",
            &FeishuOnboardCredentials {
                app_id: "cli_work_123".to_owned(),
                app_secret: "work_secret_123".to_owned(),
                verification_token: None,
                encrypt_key: None,
            },
            FeishuOnboardApplyOptions {
                domain: mvp::config::FeishuDomain::Feishu,
                mode: mvp::config::FeishuChannelServeMode::Websocket,
            },
        );

        let account = config.accounts.get("Work Bot").expect("named account");
        assert_eq!(account.enabled, Some(true));
        assert_eq!(
            account
                .app_id
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("cli_work_123")
        );
        assert_eq!(
            account
                .app_secret
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("work_secret_123")
        );
    }

    #[test]
    fn apply_owner_bootstrap_access_updates_display_label_named_account() {
        let mut config = mvp::config::FeishuChannelConfig {
            accounts: BTreeMap::from([(
                "Work Bot".to_owned(),
                mvp::config::FeishuAccountConfig::default(),
            )]),
            ..mvp::config::FeishuChannelConfig::default()
        };

        let applied = apply_owner_bootstrap_access(
            &mut config,
            "work-bot",
            FeishuOnboardCredentialSource::QrRegistration,
            Some("ou_owner_1"),
        );

        assert!(applied);
        let account = config.accounts.get("Work Bot").expect("named account");
        assert_eq!(
            account.allowed_chat_ids.clone().unwrap_or_default(),
            vec!["*".to_owned()]
        );
        assert_eq!(
            account.allowed_sender_ids.clone().unwrap_or_default(),
            vec!["ou_owner_1".to_owned()]
        );
    }

    #[test]
    fn apply_owner_bootstrap_access_preserves_existing_restrictions() {
        let mut config = mvp::config::FeishuChannelConfig {
            allowed_chat_ids: vec!["oc_ops_room".to_owned()],
            ..mvp::config::FeishuChannelConfig::default()
        };

        let applied = apply_owner_bootstrap_access(
            &mut config,
            "feishu_cli_default",
            FeishuOnboardCredentialSource::QrRegistration,
            Some("ou_owner_1"),
        );

        assert!(!applied);
        assert_eq!(config.allowed_chat_ids, vec!["oc_ops_room".to_owned()]);
        assert!(config.allowed_sender_ids.is_empty());
    }

    async fn spawn_registration_server(state: RegistrationServerState) -> String {
        async fn handle_registration(
            State(state): State<RegistrationServerState>,
            body: String,
        ) -> Json<Value> {
            let form = body
                .split('&')
                .filter_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    Some((key.to_owned(), value.to_owned()))
                })
                .collect::<HashMap<String, String>>();
            match form.get("action").map(String::as_str) {
                Some("init") => Json(json!({
                    "supported_auth_methods": ["client_secret", "oauth"]
                })),
                Some("begin") => Json(json!({
                    "device_code": "device-123",
                    "verification_uri_complete": "https://scan.example/activate?device=device-123",
                    "interval": 0,
                    "expire_in": 10,
                })),
                Some("poll") => {
                    let mut guard = state.poll_requests.lock().await;
                    guard.push(
                        form.get("device_code")
                            .cloned()
                            .unwrap_or_else(|| "-".to_owned()),
                    );
                    let response = if guard.len() == 1 {
                        json!({
                            "error": "authorization_pending",
                            "user_info": {
                                "tenant_brand": "lark"
                            }
                        })
                    } else {
                        json!({
                            "client_id": "cli_lark_123",
                            "client_secret": "secret_lark_123",
                            "user_info": {
                                "tenant_brand": "lark",
                                "open_id": "ou_owner_1"
                            }
                        })
                    };
                    Json(response)
                }
                other => Json(json!({
                    "error": format!("unexpected action: {}", other.unwrap_or("-"))
                })),
            }
        }

        let router = Router::new()
            .route(REGISTRATION_PATH, post(handle_registration))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve registration");
        });
        format!("http://{}", addr)
    }

    async fn spawn_bot_probe_server() -> String {
        async fn token() -> Json<Value> {
            Json(json!({
                "code": 0,
                "tenant_access_token": "tenant_token_123"
            }))
        }

        async fn bot() -> Json<Value> {
            Json(json!({
                "code": 0,
                "bot": {
                    "bot_name": "Loong Bot",
                    "open_id": "ou_bot_1"
                }
            }))
        }

        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post(token),
            )
            .route("/open-apis/bot/v3/info", get(bot));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve probe");
        });
        format!("http://{}", addr)
    }

    fn test_urls(base_url: &str) -> FeishuOnboardingUrls {
        FeishuOnboardingUrls {
            feishu_accounts_base_url: base_url.to_owned(),
            lark_accounts_base_url: base_url.to_owned(),
            feishu_open_base_url: base_url.to_owned(),
            lark_open_base_url: base_url.to_owned(),
        }
    }
}
