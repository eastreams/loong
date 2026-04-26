use std::time::Duration;

use loong_contracts::SecretRef;
use qrcode::QrCode;
use qrcode::render::unicode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::Value;

use crate::CliResult;
use crate::configured_account_keys::resolve_raw_configured_account_key;
use crate::mvp;

const DEFAULT_WEIXIN_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const DEFAULT_ONBOARD_TIMEOUT_S: u64 = 600;
const DEFAULT_ONBOARD_REQUEST_TIMEOUT_S: u64 = 10;
const DEFAULT_POLL_INTERVAL_S: u64 = 1;
const MAX_QR_REFRESH_COUNT: u8 = 3;
const WEIXIN_CHANNEL_VERSION: &str = "2.1.1";
const ILINK_APP_ID: &str = "bot";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeixinQrRegistrationResult {
    pub bridge_url: String,
    pub bot_token: String,
    pub bot_id: Option<String>,
    pub user_id: Option<String>,
    pub qr_url: String,
    pub qr_rendered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeixinOnboardResult {
    pub config_path: String,
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub runtime_account_id: String,
    pub bridge_url: String,
    pub bot_id: Option<String>,
    pub user_id: Option<String>,
    pub qr_url: String,
    pub qr_rendered: bool,
    pub owner_contact_bootstrap_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WeixinQrCode {
    id: String,
    scan_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WeixinQrPollStatus {
    status: String,
    bot_token: Option<String>,
    bot_id: Option<String>,
    user_id: Option<String>,
    bridge_url: Option<String>,
    redirect_host: Option<String>,
}

pub async fn onboard_via_qr_registration(
    config_path: Option<&str>,
    account: Option<&str>,
    timeout_s: Option<u64>,
) -> CliResult<WeixinOnboardResult> {
    let result = qr_register(
        DEFAULT_WEIXIN_BASE_URL,
        timeout_s.unwrap_or(DEFAULT_ONBOARD_TIMEOUT_S),
    )
    .await?
    .ok_or_else(|| "Weixin / iLink QR registration did not complete".to_owned())?;

    apply_onboard_result_to_config(config_path, account, &result)
}

pub fn render_qr_instructions(url: &str, qr_rendered: bool) -> Vec<String> {
    if qr_rendered {
        return vec![
            "Scan the QR code above with WeChat on your phone.".to_owned(),
            format!("If the terminal QR is unreadable, open this scan URL instead: {url}"),
            "Loong will keep polling until iLink returns the bridge token and runtime base URL."
                .to_owned(),
        ];
    }

    vec![format!(
        "Open or scan this WeChat / iLink QR URL to activate the bridge: {url}"
    )]
}

fn apply_onboard_result_to_config(
    config_path: Option<&str>,
    account: Option<&str>,
    result: &WeixinQrRegistrationResult,
) -> CliResult<WeixinOnboardResult> {
    let (config_path, mut config) = mvp::config::load(config_path)?;
    let configured_account_id = ensure_selected_account_exists(&mut config.weixin, account)?;
    let owner_contact_bootstrap_applied =
        apply_registration_to_selected_account(&mut config.weixin, &configured_account_id, result);
    let resolved = config
        .weixin
        .resolve_account(Some(configured_account_id.as_str()))?;

    let config_path_string = config_path.display().to_string();
    let saved_path = mvp::config::write(Some(config_path_string.as_str()), &config, true)?;

    Ok(WeixinOnboardResult {
        config_path: saved_path.display().to_string(),
        configured_account_id: resolved.configured_account_id,
        configured_account_label: resolved.configured_account_label,
        runtime_account_id: resolved.account.id,
        bridge_url: result.bridge_url.clone(),
        bot_id: result.bot_id.clone(),
        user_id: result.user_id.clone(),
        qr_url: result.qr_url.clone(),
        qr_rendered: result.qr_rendered,
        owner_contact_bootstrap_applied,
    })
}

fn ensure_selected_account_exists(
    channel: &mut mvp::config::WeixinChannelConfig,
    account: Option<&str>,
) -> CliResult<String> {
    let Some(requested_account_label) = trimmed_opt(account) else {
        return Ok(channel.default_configured_account_id());
    };

    let configured_account_id = mvp::config::normalize_channel_account_id(requested_account_label);
    let existing_raw_account_key =
        resolve_raw_configured_account_key(channel.accounts.keys(), configured_account_id.as_str());
    if existing_raw_account_key.is_some() {
        return Ok(configured_account_id);
    }

    let should_promote_to_default = channel.accounts.is_empty()
        && channel.default_account.is_none()
        && !root_weixin_account_is_materially_configured(channel);
    channel.accounts.insert(
        requested_account_label.to_owned(),
        mvp::config::WeixinAccountConfig::default(),
    );
    if should_promote_to_default {
        channel.default_account = Some(requested_account_label.to_owned());
    }

    Ok(configured_account_id)
}

fn apply_registration_to_selected_account(
    channel: &mut mvp::config::WeixinChannelConfig,
    configured_account_id: &str,
    result: &WeixinQrRegistrationResult,
) -> bool {
    let preferred_runtime_account_id = preferred_runtime_account_id(result);
    let raw_account_key =
        resolve_raw_configured_account_key(channel.accounts.keys(), configured_account_id);

    if let Some(raw_account_key) = raw_account_key.as_deref()
        && let Some(account) = channel.accounts.get_mut(raw_account_key)
    {
        account.enabled = Some(true);
        account.bridge_url = Some(result.bridge_url.clone());
        account.bridge_url_env = None;
        account.bridge_access_token = Some(SecretRef::Inline(result.bot_token.clone()));
        account.bridge_access_token_env = None;
        if account.account_id.as_deref().is_none_or(is_blank)
            && let Some(runtime_account_id) = preferred_runtime_account_id.as_deref()
        {
            account.account_id = Some(runtime_account_id.to_owned());
        }
        return apply_user_bootstrap_to_account(account, result.user_id.as_deref());
    }

    channel.enabled = true;
    channel.bridge_url = Some(result.bridge_url.clone());
    channel.bridge_url_env = None;
    channel.bridge_access_token = Some(SecretRef::Inline(result.bot_token.clone()));
    channel.bridge_access_token_env = None;
    if channel.account_id.as_deref().is_none_or(is_blank)
        && let Some(runtime_account_id) = preferred_runtime_account_id.as_deref()
    {
        channel.account_id = Some(runtime_account_id.to_owned());
    }
    apply_user_bootstrap_to_root(channel, result.user_id.as_deref())
}

fn apply_user_bootstrap_to_account(
    account: &mut mvp::config::WeixinAccountConfig,
    user_id: Option<&str>,
) -> bool {
    let Some(user_id) = user_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let ids_unset = account
        .allowed_contact_ids
        .as_ref()
        .is_none_or(|values| values.is_empty());
    if !ids_unset {
        return false;
    }
    account.allowed_contact_ids = Some(vec![user_id.to_owned()]);
    true
}

fn apply_user_bootstrap_to_root(
    channel: &mut mvp::config::WeixinChannelConfig,
    user_id: Option<&str>,
) -> bool {
    let Some(user_id) = user_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    if !channel.allowed_contact_ids.is_empty() {
        return false;
    }
    channel.allowed_contact_ids = vec![user_id.to_owned()];
    true
}

fn root_weixin_account_is_materially_configured(
    channel: &mvp::config::WeixinChannelConfig,
) -> bool {
    channel.enabled
        || channel
            .account_id
            .as_deref()
            .is_some_and(|value| !is_blank(value))
        || channel
            .bridge_url
            .as_deref()
            .is_some_and(|value| !is_blank(value))
        || channel.bridge_access_token.is_some()
        || !channel.allowed_contact_ids.is_empty()
}

fn preferred_runtime_account_id(result: &WeixinQrRegistrationResult) -> Option<String> {
    result
        .bot_id
        .as_deref()
        .and_then(|value| trimmed_opt(Some(value)))
        .or_else(|| {
            result
                .user_id
                .as_deref()
                .and_then(|value| trimmed_opt(Some(value)))
        })
        .map(str::to_owned)
}

async fn qr_register(
    initial_base_url: &str,
    timeout_s: u64,
) -> CliResult<Option<WeixinQrRegistrationResult>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_ONBOARD_REQUEST_TIMEOUT_S))
        .build()
        .map_err(|error| format!("build Weixin / iLink onboarding client failed: {error}"))?;

    print!("  Requesting Weixin / iLink QR code...");
    let mut qr_code = fetch_qr_code(&client, initial_base_url).await?;
    println!(" done.");
    println!();

    let qr_rendered = render_terminal_qr(qr_code.scan_url.as_str());
    for line in render_qr_instructions(qr_code.scan_url.as_str(), qr_rendered) {
        println!("  {line}");
    }
    println!();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_s.max(1));
    let mut current_poll_base_url = initial_base_url.to_owned();
    let mut refresh_count = 0_u8;
    let mut poll_count = 0_u64;

    loop {
        if tokio::time::Instant::now() >= deadline {
            if poll_count > 0 {
                println!();
            }
            return Ok(None);
        }

        let poll_result =
            poll_qr_status(&client, current_poll_base_url.as_str(), qr_code.id.as_str()).await;
        let status = match poll_result {
            Ok(status) => status,
            Err(QrPollError::Retryable(_)) => {
                tokio::time::sleep(Duration::from_secs(DEFAULT_POLL_INTERVAL_S)).await;
                continue;
            }
            Err(QrPollError::Fatal(message)) => return Err(message),
        };

        poll_count = poll_count.saturating_add(1);
        if poll_count == 1 {
            print!("  Waiting for QR confirmation...");
        } else if poll_count.is_multiple_of(6) {
            print!(".");
        }

        match status.status.as_str() {
            "confirmed" => {
                let bot_token = status
                    .bot_token
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        "Weixin / iLink confirmed the QR scan but did not return `bot_token`"
                            .to_owned()
                    })?;
                let bridge_url = status
                    .bridge_url
                    .as_deref()
                    .and_then(normalize_base_url)
                    .unwrap_or_else(|| current_poll_base_url.clone());
                println!();
                return Ok(Some(WeixinQrRegistrationResult {
                    bridge_url,
                    bot_token,
                    bot_id: status
                        .bot_id
                        .and_then(|value| trimmed_opt(Some(value.as_str())).map(str::to_owned)),
                    user_id: status
                        .user_id
                        .and_then(|value| trimmed_opt(Some(value.as_str())).map(str::to_owned)),
                    qr_url: qr_code.scan_url,
                    qr_rendered,
                }));
            }
            "scaned_but_redirect" => {
                if let Some(redirected) =
                    status.redirect_host.as_deref().and_then(normalize_base_url)
                    && redirected != current_poll_base_url
                {
                    current_poll_base_url = redirected;
                }
            }
            "expired" => {
                refresh_count = refresh_count.saturating_add(1);
                if refresh_count > MAX_QR_REFRESH_COUNT {
                    println!();
                    return Ok(None);
                }
                println!();
                qr_code = fetch_qr_code(&client, initial_base_url).await?;
                current_poll_base_url = initial_base_url.to_owned();
                let _ = render_terminal_qr(qr_code.scan_url.as_str());
                for line in render_qr_instructions(qr_code.scan_url.as_str(), qr_rendered) {
                    println!("  {line}");
                }
            }
            _ => {}
        }

        tokio::time::sleep(Duration::from_secs(DEFAULT_POLL_INTERVAL_S)).await;
    }
}

async fn fetch_qr_code(client: &reqwest::Client, base_url: &str) -> CliResult<WeixinQrCode> {
    let payload = api_get_json(
        client,
        base_url,
        "ilink/bot/get_bot_qrcode",
        &[("bot_type".to_owned(), "3".to_owned())],
    )
    .await?;
    let qrcode = required_string(&payload, "qrcode", "Weixin / iLink QR response")?;
    let scan_url =
        optional_string(&payload, "qrcode_img_content").unwrap_or_else(|| qrcode.clone());

    Ok(WeixinQrCode {
        id: qrcode,
        scan_url,
    })
}

async fn poll_qr_status(
    client: &reqwest::Client,
    base_url: &str,
    qr_code: &str,
) -> Result<WeixinQrPollStatus, QrPollError> {
    let payload = api_get_json_with_retry(
        client,
        base_url,
        "ilink/bot/get_qrcode_status",
        &[("qrcode".to_owned(), qr_code.to_owned())],
    )
    .await?;

    Ok(WeixinQrPollStatus {
        status: optional_string(&payload, "status").unwrap_or_default(),
        bot_token: optional_string(&payload, "bot_token"),
        bot_id: optional_string(&payload, "ilink_bot_id"),
        user_id: optional_string(&payload, "ilink_user_id"),
        bridge_url: optional_string(&payload, "baseurl"),
        redirect_host: optional_string(&payload, "redirect_host"),
    })
}

async fn api_get_json(
    client: &reqwest::Client,
    base_url: &str,
    endpoint: &str,
    query: &[(String, String)],
) -> CliResult<Value> {
    let url = build_endpoint_url(base_url, endpoint)?;
    let response = client
        .get(url.clone())
        .query(query)
        .headers(build_ilink_headers(None)?)
        .send()
        .await
        .map_err(|error| format!("Weixin / iLink request to {endpoint} failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("read Weixin / iLink response for {endpoint} failed: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Weixin / iLink request to {endpoint} failed with {status}: {body}"
        ));
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("decode Weixin / iLink response for {endpoint} failed: {error}"))
}

async fn api_get_json_with_retry(
    client: &reqwest::Client,
    base_url: &str,
    endpoint: &str,
    query: &[(String, String)],
) -> Result<Value, QrPollError> {
    let url = build_endpoint_url(base_url, endpoint).map_err(QrPollError::Fatal)?;
    let response = client
        .get(url.clone())
        .query(query)
        .headers(build_ilink_headers(None).map_err(QrPollError::Fatal)?)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() || error.is_connect() || error.is_request() || error.is_body() {
                QrPollError::Retryable(format!(
                    "Weixin / iLink polling request to {endpoint} failed: {error}"
                ))
            } else {
                QrPollError::Fatal(format!(
                    "Weixin / iLink polling request to {endpoint} failed: {error}"
                ))
            }
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        QrPollError::Fatal(format!(
            "read Weixin / iLink polling response for {endpoint} failed: {error}"
        ))
    })?;
    if status.is_server_error() {
        return Err(QrPollError::Retryable(format!(
            "Weixin / iLink polling request to {endpoint} returned {status}: {body}"
        )));
    }
    if !status.is_success() {
        return Err(QrPollError::Fatal(format!(
            "Weixin / iLink polling request to {endpoint} failed with {status}: {body}"
        )));
    }
    serde_json::from_str(&body).map_err(|error| {
        QrPollError::Fatal(format!(
            "decode Weixin / iLink polling response for {endpoint} failed: {error}"
        ))
    })
}

fn build_endpoint_url(base_url: &str, endpoint: &str) -> CliResult<String> {
    let normalized_base_url = normalize_base_url(base_url)
        .ok_or_else(|| format!("invalid Weixin / iLink base url `{base_url}`"))?;
    Ok(format!(
        "{}/{}",
        normalized_base_url.trim_end_matches('/'),
        endpoint.trim_start_matches('/'),
    ))
}

fn build_ilink_headers(auth_token: Option<&str>) -> CliResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-WECHAT-UIN",
        HeaderValue::from_str(random_wechat_uin().as_str())
            .map_err(|error| format!("build Weixin / iLink X-WECHAT-UIN header failed: {error}"))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "AuthorizationType",
        HeaderValue::from_static("ilink_bot_token"),
    );
    headers.insert("iLink-App-Id", HeaderValue::from_static(ILINK_APP_ID));
    let client_version = build_client_version(WEIXIN_CHANNEL_VERSION).to_string();
    headers.insert(
        "iLink-App-ClientVersion",
        HeaderValue::from_str(client_version.as_str()).map_err(|error| {
            format!("build Weixin / iLink client version header failed: {error}")
        })?,
    );
    if let Some(auth_token) = auth_token.map(str::trim).filter(|value| !value.is_empty()) {
        let value = format!("Bearer {auth_token}");
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(value.as_str()).map_err(|error| {
                format!("build Weixin / iLink Authorization header failed: {error}")
            })?,
        );
    }
    Ok(headers)
}

fn build_client_version(version: &str) -> u32 {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or_default();
    let minor = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or_default();
    let patch = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or_default();
    ((major & 0xFF) << 16) | ((minor & 0xFF) << 8) | (patch & 0xFF)
}

fn random_wechat_uin() -> String {
    use base64::Engine as _;

    let raw = u32::from_be_bytes(rand::random::<[u8; 4]>());
    base64::engine::general_purpose::STANDARD.encode(raw.to_string())
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

fn normalize_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let parsed = reqwest::Url::parse(candidate.as_str()).ok()?;
    let mut normalized = format!("{}://{}", parsed.scheme(), parsed.host_str()?);
    if let Some(port) = parsed.port() {
        normalized.push(':');
        normalized.push_str(port.to_string().as_str());
    }
    Some(normalized)
}

fn required_string(payload: &Value, field: &str, context: &str) -> CliResult<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{context} missing `{field}`"))
}

fn optional_string(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn trimmed_opt(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

fn is_blank(raw: &str) -> bool {
    raw.trim().is_empty()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QrPollError {
    Retryable(String),
    Fatal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::extract::{Query, State};
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct QrServerState {
        status_responses: Arc<Mutex<Vec<Value>>>,
        status_hosts: Arc<Mutex<Vec<String>>>,
        qr_requests: Arc<Mutex<u32>>,
    }

    #[tokio::test]
    async fn qr_register_handles_redirect_then_confirmation() {
        let redirect_server = spawn_static_json_server(json!({
            "status": "confirmed",
            "bot_token": "token-2",
            "ilink_bot_id": "bot-2",
            "ilink_user_id": "wxid-owner",
            "baseurl": "https://bridge.redirect.test"
        }))
        .await;
        let state = QrServerState {
            status_responses: Arc::new(Mutex::new(vec![json!({
                "status": "scaned_but_redirect",
                "redirect_host": redirect_server
            })])),
            ..QrServerState::default()
        };
        let base_url = spawn_qr_server(state.clone()).await;

        let result = qr_register(base_url.as_str(), 5)
            .await
            .expect("qr register");
        let result = result.expect("registration result");

        assert_eq!(result.bot_id.as_deref(), Some("bot-2"));
        assert_eq!(result.user_id.as_deref(), Some("wxid-owner"));
        assert_eq!(result.bot_token, "token-2");
        assert_eq!(result.bridge_url, "https://bridge.redirect.test");
        assert!(!result.qr_url.is_empty());

        let hosts = state.status_hosts.lock().await.clone();
        assert!(hosts.iter().any(|host| host.contains("127.0.0.1")));
    }

    #[tokio::test]
    async fn apply_registration_updates_root_channel_and_bootstraps_owner() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("loong.toml");
        mvp::config::write_template(Some(config_path.to_string_lossy().as_ref()), true)
            .expect("write template");

        let result = apply_onboard_result_to_config(
            Some(config_path.to_string_lossy().as_ref()),
            None,
            &WeixinQrRegistrationResult {
                bridge_url: "https://bridge.example.test".to_owned(),
                bot_token: "token-root".to_owned(),
                bot_id: Some("bot-root".to_owned()),
                user_id: Some("wxid-root".to_owned()),
                qr_url: "https://scan.example/qr".to_owned(),
                qr_rendered: true,
            },
        )
        .expect("apply onboarding");

        let (_, config) =
            mvp::config::load(Some(result.config_path.as_str())).expect("load config");
        assert!(config.weixin.enabled);
        assert_eq!(
            config.weixin.bridge_url.as_deref(),
            Some("https://bridge.example.test")
        );
        assert_eq!(
            config
                .weixin
                .bridge_access_token
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("token-root")
        );
        assert_eq!(config.weixin.account_id.as_deref(), Some("bot-root"));
        assert_eq!(
            config.weixin.allowed_contact_ids,
            vec!["wxid-root".to_owned()]
        );
        assert!(result.owner_contact_bootstrap_applied);
        assert_eq!(result.runtime_account_id, "bot-root");
    }

    #[test]
    fn apply_registration_updates_named_account_and_sets_first_default_account() {
        let mut channel = mvp::config::WeixinChannelConfig::default();
        let configured_account_id =
            ensure_selected_account_exists(&mut channel, Some("ops")).expect("account selection");
        assert_eq!(configured_account_id, "ops");
        assert_eq!(channel.default_account.as_deref(), Some("ops"));

        let applied = apply_registration_to_selected_account(
            &mut channel,
            "ops",
            &WeixinQrRegistrationResult {
                bridge_url: "https://bridge.example.test".to_owned(),
                bot_token: "token-ops".to_owned(),
                bot_id: Some("bot-ops".to_owned()),
                user_id: Some("wxid-ops".to_owned()),
                qr_url: "https://scan.example/qr".to_owned(),
                qr_rendered: false,
            },
        );

        let account = channel.accounts.get("ops").expect("named account");
        assert_eq!(account.enabled, Some(true));
        assert_eq!(
            account.bridge_url.as_deref(),
            Some("https://bridge.example.test")
        );
        assert_eq!(
            account
                .bridge_access_token
                .as_ref()
                .and_then(SecretRef::inline_literal_value),
            Some("token-ops")
        );
        assert_eq!(account.account_id.as_deref(), Some("bot-ops"));
        assert_eq!(
            account.allowed_contact_ids.clone().unwrap_or_default(),
            vec!["wxid-ops".to_owned()]
        );
        assert!(applied);
    }

    #[test]
    fn ensure_selected_account_exists_reuses_existing_display_label_account() {
        let mut channel = mvp::config::WeixinChannelConfig::default();
        channel.accounts.insert(
            "Ops Team".to_owned(),
            mvp::config::WeixinAccountConfig::default(),
        );

        let configured_account_id = ensure_selected_account_exists(&mut channel, Some("ops-team"))
            .expect("account selection");

        assert_eq!(configured_account_id, "ops-team");
        assert_eq!(channel.accounts.len(), 1);
        assert!(channel.accounts.contains_key("Ops Team"));
    }

    #[test]
    fn apply_registration_updates_display_label_named_account() {
        let mut channel = mvp::config::WeixinChannelConfig::default();
        channel.accounts.insert(
            "Ops Team".to_owned(),
            mvp::config::WeixinAccountConfig::default(),
        );

        let applied = apply_registration_to_selected_account(
            &mut channel,
            "ops-team",
            &WeixinQrRegistrationResult {
                bridge_url: "https://bridge.example.test".to_owned(),
                bot_token: "token-ops".to_owned(),
                bot_id: Some("bot-ops".to_owned()),
                user_id: Some("wxid-ops".to_owned()),
                qr_url: "https://scan.example/qr".to_owned(),
                qr_rendered: false,
            },
        );

        let account = channel.accounts.get("Ops Team").expect("named account");
        assert_eq!(account.enabled, Some(true));
        assert_eq!(account.account_id.as_deref(), Some("bot-ops"));
        assert_eq!(
            account.allowed_contact_ids.clone().unwrap_or_default(),
            vec!["wxid-ops".to_owned()]
        );
        assert!(applied);
    }

    #[test]
    fn render_qr_instructions_mentions_scan_url_when_qr_is_rendered() {
        let rendered = render_qr_instructions("https://scan.example/qr", true).join("\n");
        assert!(rendered.contains("Scan the QR code above"));
        assert!(rendered.contains("https://scan.example/qr"));
    }

    #[test]
    fn encode_terminal_qr_returns_non_empty_unicode_art() {
        let rendered =
            encode_terminal_qr("https://scan.example/qr?device=1").expect("qr should render");
        assert!(!rendered.is_empty());
        assert!(rendered.lines().count() >= 10);
    }

    #[test]
    fn build_client_version_encodes_semver_components() {
        assert_eq!(build_client_version("2.1.1"), 0x0002_0101);
        assert_eq!(build_client_version("2.1.0"), 0x0002_0100);
    }

    async fn spawn_qr_server(state: QrServerState) -> String {
        async fn handle_qr(
            State(state): State<QrServerState>,
            Query(_query): Query<HashMap<String, String>>,
        ) -> Json<Value> {
            let mut qr_requests = state.qr_requests.lock().await;
            *qr_requests += 1;
            let qr_id = format!("qr-{}", *qr_requests);
            Json(json!({
                "qrcode": qr_id,
                "qrcode_img_content": "https://scan.example/activate"
            }))
        }

        async fn handle_status(
            State(state): State<QrServerState>,
            Query(_query): Query<HashMap<String, String>>,
            request: axum::extract::Request,
        ) -> Json<Value> {
            let host = request
                .headers()
                .get("host")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            state.status_hosts.lock().await.push(host);
            let mut responses = state.status_responses.lock().await;
            let response = if responses.is_empty() {
                json!({"status": "wait"})
            } else {
                responses.remove(0)
            };
            Json(response)
        }

        let app = Router::new()
            .route("/ilink/bot/get_bot_qrcode", get(handle_qr))
            .route("/ilink/bot/get_qrcode_status", get(handle_status))
            .with_state(state);
        spawn_router(app).await
    }

    async fn spawn_static_json_server(payload: Value) -> String {
        let app = Router::new().route(
            "/ilink/bot/get_qrcode_status",
            get(move || {
                let payload = payload.clone();
                async move { Json(payload) }
            }),
        );
        spawn_router(app).await
    }

    async fn spawn_router(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        format!("http://{}", addr)
    }
}
