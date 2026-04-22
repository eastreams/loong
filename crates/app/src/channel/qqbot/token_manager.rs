use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::CliResult;
use crate::channel::core::http::{read_json_or_text_response, validate_outbound_http_target};
use crate::channel::http::ChannelOutboundHttpPolicy;
use serde_json::json;
use tokio::sync::RwLock;

/// Seconds before token expiry to proactively refresh.
const TOKEN_REFRESH_LEEWAY_S: u64 = 50;

const QQ_AUTH_URL: &str = "https://bots.qq.com/app/getAppAccessToken";

/// QQBot token response from the official API.
#[derive(Debug)]
struct QqbotAccessToken {
    access_token: String,
    expires_at: Instant,
}

/// Manages QQBot Access Token lifecycle.
pub(super) struct QqbotTokenManager {
    app_id: String,
    client_secret: String,
    current_token: Arc<RwLock<Option<QqbotAccessToken>>>,
    http_client: reqwest::Client,
    policy: ChannelOutboundHttpPolicy,
}

impl QqbotTokenManager {
    pub(super) fn new(
        app_id: String,
        client_secret: String,
        http_client: reqwest::Client,
        policy: ChannelOutboundHttpPolicy,
    ) -> Self {
        Self {
            app_id,
            client_secret,
            current_token: Arc::new(RwLock::new(None)),
            http_client,
            policy,
        }
    }

    /// Returns a valid token, refreshing proactively if expiry is within 50s.
    pub(super) async fn get_valid_access_token(&mut self) -> CliResult<String> {
        if self.is_token_valid().await {
            let current_token = self.current_token.read().await;
            if let Some(token) = &*current_token {
                return Ok(token.access_token.clone());
            }
        }
        self.refresh_token().await
    }

    /// Force-refresh the token from the QQBot API.
    pub(super) async fn refresh_token(&mut self) -> CliResult<String> {
        let request_url =
            validate_outbound_http_target("qqbot auth_url", QQ_AUTH_URL, self.policy)?;
        let body = json!({
            "appId": self.app_id,
            "clientSecret": self.client_secret,
        });
        let resp = self
            .http_client
            .post(request_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("qqbot token request failed: {e}"));
        let resp = match resp {
            Ok(res) => res,
            Err(e) => {
                return Err(e);
            }
        };
        let (status, body, payload) = read_json_or_text_response(resp, "qqbot token").await?;
        if status.is_success() {
            let token_resp = parse_qqbot_token_response(&payload)?;
            let mut cache = self.current_token.write().await;
            let token_val = token_resp.access_token.clone();
            *cache = Some(token_resp);
            Ok(token_val)
        } else {
            Err(format!("QQ token request failed ({}): {}", status, body))
        }
    }

    /// Checks whether the current token is still valid (with 50s leeway).
    pub(super) async fn is_token_valid(&self) -> bool {
        let acs_token = self.current_token.read().await;
        match &*acs_token {
            Some(token) => {
                if token.expires_at.saturating_duration_since(Instant::now())
                    > Duration::from_secs(TOKEN_REFRESH_LEEWAY_S)
                {
                    return false;
                } else {
                    return true;
                }
            }
            None => {
                return false;
            }
        }
    }
}

fn parse_qqbot_token_response(payload: &serde_json::Value) -> CliResult<QqbotAccessToken> {
    let access_token = payload
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("qqbot token response missing access_token: {payload}"))?;
    let expires_in = payload
        .get("expires_in")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("qqbot token response missing expires_in: {payload}"))?;
    let expires_at = Instant::now()
        + Duration::from_secs(
            expires_in
                .parse::<u64>()
                .map_err(|e| format!("qqbot token expires_in convert err: {e}"))?,
        );
    Ok(QqbotAccessToken {
        access_token: access_token.to_owned(),
        expires_at,
    })
}
