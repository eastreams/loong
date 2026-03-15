use std::time::Duration;

use chrono::Utc;
use reqwest::Url;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::multipart::Form;
use reqwest::{RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::sleep;

use crate::CliResult;
use crate::config::{FeishuIntegrationConfig, ResolvedFeishuChannelConfig};

use super::error::FeishuApiError;
use super::principal::FeishuGrantScopeSet;

#[derive(Debug, Clone)]
pub struct FeishuClient {
    app_id: String,
    app_secret: String,
    base_url: String,
    http: reqwest::Client,
    retry_policy: FeishuRetryPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuBinaryResponse {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuRetryPolicy {
    max_attempts: usize,
    initial_backoff_ms: usize,
    max_backoff_ms: usize,
}

impl Default for FeishuRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            initial_backoff_ms: 200,
            max_backoff_ms: 2_000,
        }
    }
}

impl FeishuRetryPolicy {
    fn backoff_for_retry(&self, retry_index: usize) -> Duration {
        let multiplier = 1usize
            .checked_shl(retry_index.saturating_sub(1) as u32)
            .unwrap_or(usize::MAX);
        let base_ms = self.initial_backoff_ms.saturating_mul(multiplier);
        Duration::from_millis(base_ms.min(self.max_backoff_ms) as u64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuUserInfo {
    pub name: Option<String>,
    pub en_name: Option<String>,
    pub avatar_url: Option<String>,
    pub open_id: Option<String>,
    pub union_id: Option<String>,
    pub user_id: Option<String>,
    pub email: Option<String>,
    pub enterprise_email: Option<String>,
    pub mobile: Option<String>,
    pub tenant_key: Option<String>,
}

impl FeishuClient {
    pub fn new(
        base_url: impl Into<String>,
        app_id: impl Into<String>,
        app_secret: impl Into<String>,
        request_timeout_s: usize,
    ) -> CliResult<Self> {
        Self::new_with_retry_policy(
            base_url,
            app_id,
            app_secret,
            request_timeout_s,
            FeishuRetryPolicy::default(),
        )
    }

    fn new_with_retry_policy(
        base_url: impl Into<String>,
        app_id: impl Into<String>,
        app_secret: impl Into<String>,
        request_timeout_s: usize,
        retry_policy: FeishuRetryPolicy,
    ) -> CliResult<Self> {
        let base_url = base_url.into();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(request_timeout_s.max(1) as u64))
            .build()
            .map_err(|error| format!("build feishu http client failed: {error}"))?;
        Ok(Self {
            app_id: app_id.into(),
            app_secret: app_secret.into(),
            base_url,
            http,
            retry_policy,
        })
    }

    pub fn from_configs(
        channel: &ResolvedFeishuChannelConfig,
        integration: &FeishuIntegrationConfig,
    ) -> CliResult<Self> {
        let app_id = channel
            .app_id()
            .ok_or_else(|| "missing Feishu app id (feishu.app_id or env)".to_owned())?;
        let app_secret = channel
            .app_secret()
            .ok_or_else(|| "missing Feishu app secret (feishu.app_secret or env)".to_owned())?;
        Self::new_with_retry_policy(
            channel.resolved_base_url(),
            app_id,
            app_secret,
            integration.request_timeout_s,
            FeishuRetryPolicy {
                max_attempts: integration.retry_max_attempts.max(1),
                initial_backoff_ms: integration.retry_initial_backoff_ms,
                max_backoff_ms: integration.retry_max_backoff_ms,
            },
        )
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn app_secret(&self) -> &str {
        &self.app_secret
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn build_open_api_url(&self, path: &str) -> CliResult<Url> {
        let normalized_path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        Url::parse(&format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            normalized_path
        ))
        .map_err(|error| format!("build feishu api url failed: {error}"))
    }

    pub fn build_open_api_url_with_query(
        &self,
        path: &str,
        query_pairs: &[(String, String)],
    ) -> CliResult<Url> {
        let mut url = self.build_open_api_url(path)?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query_pairs {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    continue;
                }
                pairs.append_pair(key, trimmed);
            }
        }
        Ok(url)
    }

    pub async fn exchange_authorization_code(
        &self,
        code: &str,
        redirect_uri: Option<&str>,
        scopes: &FeishuGrantScopeSet,
        code_verifier: Option<&str>,
    ) -> CliResult<Value> {
        let mut body = serde_json::Map::new();
        body.insert(
            "grant_type".to_owned(),
            Value::String("authorization_code".to_owned()),
        );
        body.insert(
            "client_id".to_owned(),
            Value::String(self.app_id().to_owned()),
        );
        body.insert(
            "client_secret".to_owned(),
            Value::String(self.app_secret().to_owned()),
        );
        body.insert("code".to_owned(), Value::String(code.trim().to_owned()));
        if let Some(redirect_uri) = redirect_uri
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body.insert(
                "redirect_uri".to_owned(),
                Value::String(redirect_uri.to_owned()),
            );
        }
        if !scopes.is_empty() {
            body.insert("scope".to_owned(), Value::String(scopes.to_scope_csv()));
        }
        if let Some(code_verifier) = code_verifier
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body.insert(
                "code_verifier".to_owned(),
                Value::String(code_verifier.to_owned()),
            );
        }
        self.post_json(
            "/open-apis/authen/v2/oauth/token",
            None,
            &[],
            &Value::Object(body),
        )
        .await
    }

    pub async fn refresh_user_access_token(&self, refresh_token: &str) -> CliResult<Value> {
        let body = json!({
            "grant_type": "refresh_token",
            "client_id": self.app_id(),
            "client_secret": self.app_secret(),
            "refresh_token": refresh_token.trim(),
        });
        self.post_json("/open-apis/authen/v2/oauth/token", None, &[], &body)
            .await
    }

    pub async fn get_tenant_access_token(&self) -> CliResult<String> {
        let body = json!({
            "app_id": self.app_id(),
            "app_secret": self.app_secret(),
        });
        let payload = self
            .post_json(
                "/open-apis/auth/v3/tenant_access_token/internal",
                None,
                &[],
                &body,
            )
            .await?;
        parse_tenant_access_token_response(&payload)
    }

    pub async fn get_user_info(&self, user_access_token: &str) -> CliResult<FeishuUserInfo> {
        let payload = self
            .get_json(
                "/open-apis/authen/v1/user_info",
                Some(user_access_token),
                &[],
            )
            .await?;
        parse_user_info_response(&payload)
    }

    pub async fn get_json(
        &self,
        path: &str,
        bearer_token: Option<&str>,
        query_pairs: &[(String, String)],
    ) -> CliResult<Value> {
        let url = self.build_open_api_url_with_query(path, query_pairs)?;
        let request = self.authorized(self.http.get(url), bearer_token).header(
            reqwest::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        );
        self.send_json_request_with_retry(request).await
    }

    pub async fn post_json(
        &self,
        path: &str,
        bearer_token: Option<&str>,
        query_pairs: &[(String, String)],
        body: &Value,
    ) -> CliResult<Value> {
        let url = self.build_open_api_url_with_query(path, query_pairs)?;
        let request = self
            .authorized(self.http.post(url), bearer_token)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )
            .json(body);
        self.send_json_request_with_retry(request).await
    }

    pub async fn post_multipart(
        &self,
        path: &str,
        bearer_token: Option<&str>,
        query_pairs: &[(String, String)],
        build_form: impl Fn() -> Form,
    ) -> CliResult<Value> {
        let url = self.build_open_api_url_with_query(path, query_pairs)?;
        self.send_json_request_with_retry_factory(|| {
            self.authorized(self.http.post(url.clone()), bearer_token)
                .multipart(build_form())
        })
        .await
    }

    pub async fn get_binary(
        &self,
        path: &str,
        bearer_token: Option<&str>,
        query_pairs: &[(String, String)],
    ) -> CliResult<FeishuBinaryResponse> {
        let url = self.build_open_api_url_with_query(path, query_pairs)?;
        let request = self.authorized(self.http.get(url), bearer_token).header(
            reqwest::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        );
        self.send_binary_request_with_retry(request).await
    }

    fn authorized(
        &self,
        builder: reqwest::RequestBuilder,
        bearer_token: Option<&str>,
    ) -> reqwest::RequestBuilder {
        if let Some(token) = bearer_token
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return builder.bearer_auth(token);
        }
        builder
    }

    async fn send_json_request_with_retry(&self, request: RequestBuilder) -> CliResult<Value> {
        let max_attempts = self.retry_policy.max_attempts.max(1);
        let mut current_request = Some(request);

        for attempt in 1..=max_attempts {
            let request = current_request
                .take()
                .ok_or_else(|| "feishu retry pipeline lost pending request".to_owned())?;
            let retry_request = if attempt < max_attempts {
                request.try_clone()
            } else {
                None
            };

            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let headers = response.headers().clone();
                    let bytes = response
                        .bytes()
                        .await
                        .map_err(|error| format!("feishu http response decode failed: {error}"))?;
                    match evaluate_json_response(status, &bytes) {
                        Ok(payload) => return Ok(payload),
                        Err((payload, error)) => {
                            if attempt < max_attempts
                                && is_retryable_json_failure(status, payload.as_ref())
                                && let Some(next_request) = retry_request
                            {
                                sleep(retry_delay_for_attempt(
                                    &self.retry_policy,
                                    &headers,
                                    attempt,
                                ))
                                .await;
                                current_request = Some(next_request);
                                continue;
                            }
                            return Err(error);
                        }
                    }
                }
                Err(error) => {
                    if attempt < max_attempts
                        && (error.is_timeout() || error.is_connect())
                        && let Some(next_request) = retry_request
                    {
                        sleep(self.retry_policy.backoff_for_retry(attempt)).await;
                        current_request = Some(next_request);
                        continue;
                    }
                    return Err(format!("feishu http request failed: {error}"));
                }
            }
        }

        Err("feishu request retry loop exhausted without returning a result".to_owned())
    }

    async fn send_json_request_with_retry_factory(
        &self,
        build_request: impl Fn() -> RequestBuilder,
    ) -> CliResult<Value> {
        let max_attempts = self.retry_policy.max_attempts.max(1);

        for attempt in 1..=max_attempts {
            match build_request().send().await {
                Ok(response) => {
                    let status = response.status();
                    let headers = response.headers().clone();
                    let bytes = response
                        .bytes()
                        .await
                        .map_err(|error| format!("feishu http response decode failed: {error}"))?;
                    match evaluate_json_response(status, &bytes) {
                        Ok(payload) => return Ok(payload),
                        Err((payload, error)) => {
                            if attempt < max_attempts
                                && is_retryable_json_failure(status, payload.as_ref())
                            {
                                sleep(retry_delay_for_attempt(
                                    &self.retry_policy,
                                    &headers,
                                    attempt,
                                ))
                                .await;
                                continue;
                            }
                            return Err(error);
                        }
                    }
                }
                Err(error) => {
                    if attempt < max_attempts && (error.is_timeout() || error.is_connect()) {
                        sleep(self.retry_policy.backoff_for_retry(attempt)).await;
                        continue;
                    }
                    return Err(format!("feishu http request failed: {error}"));
                }
            }
        }

        Err("feishu request retry loop exhausted without returning a result".to_owned())
    }

    async fn send_binary_request_with_retry(
        &self,
        request: RequestBuilder,
    ) -> CliResult<FeishuBinaryResponse> {
        let max_attempts = self.retry_policy.max_attempts.max(1);
        let mut current_request = Some(request);

        for attempt in 1..=max_attempts {
            let request = current_request
                .take()
                .ok_or_else(|| "feishu retry pipeline lost pending request".to_owned())?;
            let retry_request = if attempt < max_attempts {
                request.try_clone()
            } else {
                None
            };

            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let headers = response.headers().clone();
                    let bytes = response
                        .bytes()
                        .await
                        .map_err(|error| format!("feishu http response decode failed: {error}"))?;
                    if status.is_success() {
                        return Ok(FeishuBinaryResponse {
                            bytes: bytes.to_vec(),
                            content_type: header_value(headers.get(reqwest::header::CONTENT_TYPE)),
                            content_disposition: header_value(
                                headers.get(reqwest::header::CONTENT_DISPOSITION),
                            ),
                        });
                    }

                    let payload = serde_json::from_slice::<Value>(&bytes).ok();
                    if attempt < max_attempts
                        && is_retryable_json_failure(status, payload.as_ref())
                        && let Some(next_request) = retry_request
                    {
                        sleep(retry_delay_for_attempt(
                            &self.retry_policy,
                            &headers,
                            attempt,
                        ))
                        .await;
                        current_request = Some(next_request);
                        continue;
                    }

                    if let Some(payload) = payload
                        && let Err(error) = ensure_success_payload(&payload)
                    {
                        return Err(error);
                    }

                    let body = String::from_utf8_lossy(&bytes);
                    let detail = body.trim();
                    if detail.is_empty() {
                        return Err(format!("feishu http request failed with status {status}"));
                    }
                    return Err(format!(
                        "feishu http request failed with status {status}: {detail}"
                    ));
                }
                Err(error) => {
                    if attempt < max_attempts
                        && (error.is_timeout() || error.is_connect())
                        && let Some(next_request) = retry_request
                    {
                        sleep(self.retry_policy.backoff_for_retry(attempt)).await;
                        current_request = Some(next_request);
                        continue;
                    }
                    return Err(format!("feishu http request failed: {error}"));
                }
            }
        }

        Err("feishu request retry loop exhausted without returning a result".to_owned())
    }
}

pub fn parse_user_info_response(payload: &Value) -> CliResult<FeishuUserInfo> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu user info payload missing data object".to_owned())?;
    Ok(FeishuUserInfo {
        name: string_field(data, "name"),
        en_name: string_field(data, "en_name"),
        avatar_url: string_field(data, "avatar_url"),
        open_id: string_field(data, "open_id"),
        union_id: string_field(data, "union_id"),
        user_id: string_field(data, "user_id"),
        email: string_field(data, "email"),
        enterprise_email: string_field(data, "enterprise_email"),
        mobile: string_field(data, "mobile"),
        tenant_key: string_field(data, "tenant_key"),
    })
}

pub fn parse_tenant_access_token_response(payload: &Value) -> CliResult<String> {
    payload
        .get("tenant_access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "feishu tenant token payload missing tenant_access_token".to_owned())
}

fn evaluate_json_response(
    status: StatusCode,
    bytes: &[u8],
) -> Result<Value, (Option<Value>, String)> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(payload) => match ensure_success_payload(&payload) {
            Ok(payload) => Ok(payload),
            Err(error) => Err((Some(payload), error)),
        },
        Err(error) => {
            if !status.is_success() {
                let body = String::from_utf8_lossy(bytes);
                let detail = body.trim();
                if detail.is_empty() {
                    return Err((
                        None,
                        format!("feishu http request failed with status {status}"),
                    ));
                }
                return Err((
                    None,
                    format!("feishu http request failed with status {status}: {detail}"),
                ));
            }
            Err((None, format!("feishu http response decode failed: {error}")))
        }
    }
}

fn is_retryable_json_failure(status: StatusCode, payload: Option<&Value>) -> bool {
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return true;
    }

    let code = payload
        .and_then(|payload| payload.get("code"))
        .and_then(Value::as_i64);
    matches!(
        code,
        Some(2200)
            | Some(11232)
            | Some(11233)
            | Some(11247)
            | Some(1771001)
            | Some(1771002)
            | Some(1771003)
            | Some(1771004)
            | Some(1771005)
            | Some(99991400)
    )
}

fn retry_delay_for_attempt(
    retry_policy: &FeishuRetryPolicy,
    headers: &HeaderMap,
    attempt: usize,
) -> Duration {
    parse_retry_after_delay(headers).unwrap_or_else(|| retry_policy.backoff_for_retry(attempt))
}

fn parse_retry_after_delay(headers: &HeaderMap) -> Option<Duration> {
    let value = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let retry_at = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    let now = Utc::now();
    Some(
        retry_at
            .signed_duration_since(now)
            .to_std()
            .unwrap_or(Duration::ZERO),
    )
}

fn ensure_success_payload(payload: &Value) -> CliResult<Value> {
    let code = payload.get("code").and_then(Value::as_i64).unwrap_or(0);
    if code == 0
        && payload
            .get("error")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return Ok(payload.clone());
    }

    let message = payload
        .get("msg")
        .and_then(Value::as_str)
        .or_else(|| payload.get("error_description").and_then(Value::as_str))
        .or_else(|| payload.get("error").and_then(Value::as_str))
        .unwrap_or("unknown error");
    Err(FeishuApiError::new(code, message).to_string())
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn header_value(value: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    value
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Json, Router,
        body::Body,
        http::{HeaderMap, HeaderValue, Response, StatusCode},
        routing::{get, post},
    };
    use reqwest::multipart::{Form, Part};

    use super::*;

    async fn spawn_mock_feishu_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock feishu listener");
        let addr = listener.local_addr().expect("mock feishu listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock feishu api");
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn build_open_api_url_preserves_trimmed_base_url() {
        let client = FeishuClient::new("https://open.feishu.cn/", "cli_xxx", "secret_xxx", 20)
            .expect("client");
        let url = client
            .build_open_api_url("/open-apis/authen/v1/user_info")
            .expect("build url");

        assert_eq!(
            url.as_str(),
            "https://open.feishu.cn/open-apis/authen/v1/user_info"
        );
    }

    #[test]
    fn parse_user_info_response_reads_primary_identifiers() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "name": "Alice",
                "open_id": "ou_123",
                "union_id": "on_456",
                "user_id": "u_789",
                "tenant_key": "tenant_x"
            }
        });

        let user = parse_user_info_response(&payload).expect("parse user info");

        assert_eq!(user.name.as_deref(), Some("Alice"));
        assert_eq!(user.open_id.as_deref(), Some("ou_123"));
        assert_eq!(user.union_id.as_deref(), Some("on_456"));
        assert_eq!(user.user_id.as_deref(), Some("u_789"));
        assert_eq!(user.tenant_key.as_deref(), Some("tenant_x"));
    }

    #[test]
    fn parse_tenant_access_token_response_reads_token_value() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "ok",
            "tenant_access_token": "t-123"
        });

        let token = parse_tenant_access_token_response(&payload).expect("parse tenant token");

        assert_eq!(token, "t-123");
    }

    #[tokio::test]
    async fn get_json_retries_rate_limited_retry_after_response_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/test",
            get({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            let mut headers = HeaderMap::new();
                            headers.insert("retry-after", HeaderValue::from_static("0"));
                            return (
                                StatusCode::TOO_MANY_REQUESTS,
                                headers,
                                Json(serde_json::json!({
                                    "code": 99991400,
                                    "msg": "request trigger frequency limit"
                                })),
                            );
                        }
                        (
                            StatusCode::OK,
                            HeaderMap::new(),
                            Json(serde_json::json!({
                                "code": 0,
                                "msg": "ok",
                                "data": {
                                    "value": "ok"
                                }
                            })),
                        )
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");
        client.retry_policy.max_attempts = 2;
        client.retry_policy.initial_backoff_ms = 0;
        client.retry_policy.max_backoff_ms = 0;

        let payload = client
            .get_json("/open-apis/test", Some("u-token"), &[])
            .await
            .expect("rate-limited request should retry and succeed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(payload["data"]["value"], "ok");

        server.abort();
    }

    #[tokio::test]
    async fn post_json_retries_retryable_feishu_payload_error_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/test",
            post({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            return Json(serde_json::json!({
                                "code": 2200,
                                "msg": "internal error"
                            }));
                        }
                        Json(serde_json::json!({
                            "code": 0,
                            "msg": "ok",
                            "data": {
                                "value": "ok"
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");
        client.retry_policy.max_attempts = 2;
        client.retry_policy.initial_backoff_ms = 0;
        client.retry_policy.max_backoff_ms = 0;

        let payload = client
            .post_json(
                "/open-apis/test",
                Some("u-token"),
                &[],
                &serde_json::json!({"hello": "world"}),
            )
            .await
            .expect("retryable payload error should retry and succeed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(payload["data"]["value"], "ok");

        server.abort();
    }

    #[tokio::test]
    async fn post_multipart_retries_retryable_feishu_payload_error_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/test",
            post({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            return Json(serde_json::json!({
                                "code": 2200,
                                "msg": "internal error"
                            }));
                        }
                        Json(serde_json::json!({
                            "code": 0,
                            "msg": "ok",
                            "data": {
                                "file_key": "file_v2_demo"
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");
        client.retry_policy.max_attempts = 2;
        client.retry_policy.initial_backoff_ms = 0;
        client.retry_policy.max_backoff_ms = 0;

        let payload = client
            .post_multipart("/open-apis/test", Some("tenant-token"), &[], || {
                Form::new()
                    .text("file_type", "stream")
                    .part("file", Part::bytes(b"demo".to_vec()).file_name("demo.txt"))
            })
            .await
            .expect("retryable multipart error should retry and succeed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(payload["data"]["file_key"], "file_v2_demo");

        server.abort();
    }

    #[tokio::test]
    async fn get_binary_retries_rate_limited_response_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/test",
            get({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            return Response::builder()
                                .status(StatusCode::TOO_MANY_REQUESTS)
                                .header("content-type", "application/json")
                                .header("retry-after", "0")
                                .body(Body::from(
                                    serde_json::json!({
                                        "code": 99991400,
                                        "msg": "request trigger frequency limit"
                                    })
                                    .to_string(),
                                ))
                                .expect("build retry response");
                        }

                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/pdf")
                            .header(
                                "content-disposition",
                                "attachment; filename=\"spec-sheet.pdf\"",
                            )
                            .body(Body::from(b"demo-pdf".to_vec()))
                            .expect("build binary response")
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");
        client.retry_policy.max_attempts = 2;
        client.retry_policy.initial_backoff_ms = 0;
        client.retry_policy.max_backoff_ms = 0;

        let payload = client
            .get_binary("/open-apis/test", Some("tenant-token"), &[])
            .await
            .expect("binary request should retry and succeed");

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(payload.bytes, b"demo-pdf".to_vec());
        assert_eq!(payload.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(
            payload.content_disposition.as_deref(),
            Some("attachment; filename=\"spec-sheet.pdf\"")
        );

        server.abort();
    }

    #[test]
    fn parse_retry_after_delay_supports_http_date_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Thu, 01 Jan 1970 00:00:00 GMT"),
        );

        assert_eq!(parse_retry_after_delay(&headers), Some(Duration::ZERO));
    }

    #[tokio::test]
    async fn get_json_does_not_retry_non_retryable_feishu_error() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route(
            "/open-apis/test",
            get({
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!({
                            "code": 10029,
                            "msg": "chat_id not exist"
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let mut client = FeishuClient::new(base_url, "cli_xxx", "secret_xxx", 20).expect("client");
        client.retry_policy.max_attempts = 2;
        client.retry_policy.initial_backoff_ms = 0;
        client.retry_policy.max_backoff_ms = 0;

        let error = client
            .get_json("/open-apis/test", Some("u-token"), &[])
            .await
            .expect_err("non-retryable error should be returned immediately");

        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(error, "feishu api error 10029: chat_id not exist");

        server.abort();
    }
}
