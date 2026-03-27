use serde::Deserialize;
use serde_json::Value;

use crate::{CliResult, config::ResolvedTwitchChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{build_outbound_http_client, read_json_or_text_response, response_body_detail},
};

const TWITCH_USER_WRITE_CHAT_SCOPE: &str = "user:write:chat";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedTwitchToken {
    client_id: String,
    user_id: String,
    login: Option<String>,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTwitchBroadcaster {
    broadcaster_id: String,
}

#[derive(Debug, Deserialize)]
struct TwitchValidateTokenResponse {
    client_id: Option<String>,
    login: Option<String>,
    scopes: Option<Vec<String>>,
    user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TwitchUsersResponse {
    data: Vec<TwitchUserEntry>,
}

#[derive(Debug, Deserialize)]
struct TwitchUserEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct TwitchSendChatResponse {
    data: Vec<TwitchSendChatEntry>,
}

#[derive(Debug, Deserialize)]
struct TwitchSendChatEntry {
    is_sent: bool,
    drop_reason: Option<TwitchDropReason>,
}

#[derive(Debug, Deserialize)]
struct TwitchDropReason {
    code: Option<String>,
    message: Option<String>,
}

pub(super) async fn run_twitch_send(
    resolved: &ResolvedTwitchChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Conversation {
        return Err(format!(
            "twitch send requires conversation target kind, got {}",
            target_kind.as_str()
        ));
    }

    let access_token = resolved
        .access_token()
        .ok_or_else(|| "twitch access token missing (set twitch.access_token or env)".to_owned())?;

    let client = build_outbound_http_client("twitch send")?;
    let validated_token =
        validate_twitch_access_token(&client, resolved, access_token.as_str()).await?;
    ensure_twitch_send_scope(&validated_token)?;

    let broadcaster = resolve_twitch_broadcaster(
        &client,
        resolved,
        access_token.as_str(),
        validated_token.client_id.as_str(),
        target_id,
    )
    .await?;

    let api_base_url = resolved.resolved_api_base_url();
    let trimmed_api_base_url = api_base_url.trim_end_matches('/');
    let request_url = format!("{trimmed_api_base_url}/chat/messages");
    let request_body = serde_json::json!({
        "broadcaster_id": broadcaster.broadcaster_id,
        "sender_id": validated_token.user_id,
        "message": text,
    });

    let request = client
        .post(request_url.as_str())
        .bearer_auth(access_token.as_str())
        .header("Client-Id", validated_token.client_id.as_str())
        .json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("twitch send failed: {error}"))?;
    ensure_twitch_send_success(response).await?;

    Ok(())
}

async fn validate_twitch_access_token(
    client: &reqwest::Client,
    resolved: &ResolvedTwitchChannelConfig,
    access_token: &str,
) -> CliResult<ValidatedTwitchToken> {
    let oauth_base_url = resolved.resolved_oauth_base_url();
    let trimmed_oauth_base_url = oauth_base_url.trim_end_matches('/');
    let request_url = format!("{trimmed_oauth_base_url}/validate");
    let authorization_value = format!("OAuth {access_token}");
    let request = client
        .get(request_url.as_str())
        .header("Authorization", authorization_value);
    let response = request
        .send()
        .await
        .map_err(|error| format!("twitch token validation failed: {error}"))?;
    let (status, body, payload) =
        read_json_or_text_response(response, "twitch token validation").await?;
    if !status.is_success() {
        let detail = twitch_response_error_detail(&payload, body.as_str());
        return Err(format!(
            "twitch token validation failed with status {}: {detail}",
            status.as_u16()
        ));
    }

    let decoded = serde_json::from_value::<TwitchValidateTokenResponse>(payload)
        .map_err(|error| format!("decode twitch token validation response failed: {error}"))?;

    let client_id = decoded
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "twitch token validation did not return client_id".to_owned())?;
    let user_id = decoded
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            "twitch send requires a user access token; token validation did not return user_id"
                .to_owned()
        })?;
    let login = decoded
        .login
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let scopes = decoded.scopes.unwrap_or_default();

    Ok(ValidatedTwitchToken {
        client_id,
        user_id,
        login,
        scopes,
    })
}

fn ensure_twitch_send_scope(validated_token: &ValidatedTwitchToken) -> CliResult<()> {
    let has_required_scope = validated_token
        .scopes
        .iter()
        .any(|scope| scope == TWITCH_USER_WRITE_CHAT_SCOPE);
    if has_required_scope {
        return Ok(());
    }

    let scope_list = if validated_token.scopes.is_empty() {
        "none".to_owned()
    } else {
        validated_token.scopes.join(", ")
    };
    let sender_login = validated_token
        .login
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| validated_token.user_id.clone());
    Err(format!(
        "twitch access token for `{sender_login}` is missing required scope `{TWITCH_USER_WRITE_CHAT_SCOPE}` (granted scopes: {scope_list})"
    ))
}

async fn resolve_twitch_broadcaster(
    client: &reqwest::Client,
    resolved: &ResolvedTwitchChannelConfig,
    access_token: &str,
    client_id: &str,
    target_id: &str,
) -> CliResult<ResolvedTwitchBroadcaster> {
    let trimmed_target = target_id.trim();
    if trimmed_target.is_empty() {
        return Err("twitch outbound target id is empty".to_owned());
    }

    let api_base_url = resolved.resolved_api_base_url();
    let trimmed_api_base_url = api_base_url.trim_end_matches('/');
    let request_url = format!("{trimmed_api_base_url}/users");
    let query_key = if trimmed_target.chars().all(|value| value.is_ascii_digit()) {
        "id"
    } else {
        "login"
    };
    let query_pairs = [(query_key, trimmed_target)];
    let request = client
        .get(request_url.as_str())
        .bearer_auth(access_token)
        .header("Client-Id", client_id)
        .query(&query_pairs);
    let response = request
        .send()
        .await
        .map_err(|error| format!("twitch broadcaster lookup failed: {error}"))?;
    let (status, body, payload) =
        read_json_or_text_response(response, "twitch broadcaster lookup").await?;
    if !status.is_success() {
        let detail = twitch_response_error_detail(&payload, body.as_str());
        return Err(format!(
            "twitch broadcaster lookup failed with status {}: {detail}",
            status.as_u16()
        ));
    }

    let decoded = serde_json::from_value::<TwitchUsersResponse>(payload)
        .map_err(|error| format!("decode twitch broadcaster lookup response failed: {error}"))?;
    let broadcaster = decoded
        .data
        .into_iter()
        .next()
        .ok_or_else(|| format!("twitch broadcaster `{trimmed_target}` was not found"))?;

    Ok(ResolvedTwitchBroadcaster {
        broadcaster_id: broadcaster.id,
    })
}

async fn ensure_twitch_send_success(response: reqwest::Response) -> CliResult<()> {
    let (status, body, payload) = read_json_or_text_response(response, "twitch send").await?;
    if !status.is_success() {
        let detail = twitch_response_error_detail(&payload, body.as_str());
        return Err(format!(
            "twitch send failed with status {}: {detail}",
            status.as_u16()
        ));
    }

    let decoded = serde_json::from_value::<TwitchSendChatResponse>(payload)
        .map_err(|error| format!("decode twitch send response failed: {error}"))?;
    let send_result = decoded
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "twitch send response did not contain a result entry".to_owned())?;
    if send_result.is_sent {
        return Ok(());
    }

    let drop_reason = send_result.drop_reason;
    let detail = format_twitch_drop_reason(drop_reason.as_ref());
    Err(format!("twitch send was rejected: {detail}"))
}

fn format_twitch_drop_reason(drop_reason: Option<&TwitchDropReason>) -> String {
    let Some(drop_reason) = drop_reason else {
        return "message was not sent and Twitch did not provide a drop reason".to_owned();
    };

    let message = drop_reason
        .message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let code = drop_reason
        .code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    match (code, message) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code,
        (None, Some(message)) => message,
        (None, None) => "message was not sent and Twitch did not provide a drop reason".to_owned(),
    }
}

fn twitch_response_error_detail(payload: &Value, body: &str) -> String {
    let payload_message = payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(payload_message) = payload_message {
        return payload_message;
    }

    let error_message = payload
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(error_message) = error_message {
        return error_message;
    }

    response_body_detail(body)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, HeaderValue, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use loongclaw_contracts::SecretRef;
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct MockTwitchState {
        validate_headers: Arc<Mutex<Vec<String>>>,
        users_queries: Arc<Mutex<Vec<String>>>,
        users_headers: Arc<Mutex<Vec<String>>>,
        send_headers: Arc<Mutex<Vec<String>>>,
        send_bodies: Arc<Mutex<Vec<Value>>>,
        validate_scopes: Arc<Mutex<Vec<String>>>,
    }

    #[tokio::test]
    async fn run_twitch_send_validates_token_resolves_target_and_sends_message() {
        let state = MockTwitchState::default();
        let router = build_mock_twitch_router(state.clone());
        let (base_url, server) = spawn_mock_twitch_server(router).await;

        let resolved = ResolvedTwitchChannelConfig {
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            account: crate::config::ChannelAccountIdentity {
                id: "default".to_owned(),
                label: "default".to_owned(),
                source: crate::config::ChannelAccountIdentitySource::Default,
            },
            enabled: true,
            access_token: Some(SecretRef::Inline("twitch-access-token".to_owned())),
            access_token_env: None,
            api_base_url: Some(format!("{base_url}/helix")),
            oauth_base_url: Some(format!("{base_url}/oauth2")),
            channel_names: vec!["streamer".to_owned()],
        };

        let send_result = run_twitch_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "streamer",
            "hello from loongclaw",
        )
        .await;

        send_result.expect("twitch send should succeed");

        let validate_headers = state
            .validate_headers
            .lock()
            .expect("validate headers lock");
        assert_eq!(
            validate_headers.as_slice(),
            &[String::from("OAuth twitch-access-token")]
        );

        let users_queries = state.users_queries.lock().expect("users queries lock");
        assert_eq!(users_queries.as_slice(), &[String::from("login=streamer")]);

        let users_headers = state.users_headers.lock().expect("users headers lock");
        assert_eq!(
            users_headers.as_slice(),
            &[String::from("Bearer twitch-access-token | client-123")]
        );

        let send_headers = state.send_headers.lock().expect("send headers lock");
        assert_eq!(
            send_headers.as_slice(),
            &[String::from("Bearer twitch-access-token | client-123")]
        );

        let send_bodies = state.send_bodies.lock().expect("send bodies lock");
        assert_eq!(send_bodies.len(), 1);
        assert_eq!(
            send_bodies[0],
            serde_json::json!({
                "broadcaster_id": "broadcaster-456",
                "sender_id": "sender-123",
                "message": "hello from loongclaw",
            })
        );

        server.abort();
    }

    #[tokio::test]
    async fn run_twitch_send_rejects_tokens_without_chat_scope() {
        let state = MockTwitchState::default();
        {
            let mut validate_scopes = state.validate_scopes.lock().expect("scopes lock");
            validate_scopes.push("moderator:read:followers".to_owned());
        }
        let router = build_mock_twitch_router(state);
        let (base_url, server) = spawn_mock_twitch_server(router).await;

        let resolved = ResolvedTwitchChannelConfig {
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            account: crate::config::ChannelAccountIdentity {
                id: "default".to_owned(),
                label: "default".to_owned(),
                source: crate::config::ChannelAccountIdentitySource::Default,
            },
            enabled: true,
            access_token: Some(SecretRef::Inline("twitch-access-token".to_owned())),
            access_token_env: None,
            api_base_url: Some(format!("{base_url}/helix")),
            oauth_base_url: Some(format!("{base_url}/oauth2")),
            channel_names: Vec::new(),
        };

        let error = run_twitch_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "streamer",
            "hello from loongclaw",
        )
        .await
        .expect_err("missing scope should fail");

        assert!(error.contains(TWITCH_USER_WRITE_CHAT_SCOPE));

        server.abort();
    }

    #[tokio::test]
    async fn run_twitch_send_requires_conversation_target_kind() {
        let resolved = ResolvedTwitchChannelConfig {
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            account: crate::config::ChannelAccountIdentity {
                id: "default".to_owned(),
                label: "default".to_owned(),
                source: crate::config::ChannelAccountIdentitySource::Default,
            },
            enabled: true,
            access_token: Some(SecretRef::Inline("twitch-access-token".to_owned())),
            access_token_env: None,
            api_base_url: None,
            oauth_base_url: None,
            channel_names: Vec::new(),
        };

        let error = run_twitch_send(
            &resolved,
            ChannelOutboundTargetKind::Address,
            "streamer",
            "hello from loongclaw",
        )
        .await
        .expect_err("address targets should fail");

        assert_eq!(
            error,
            "twitch send requires conversation target kind, got address"
        );
    }

    fn build_mock_twitch_router(state: MockTwitchState) -> Router {
        Router::new()
            .route("/oauth2/validate", get(mock_validate_token))
            .route("/helix/users", get(mock_get_users))
            .route("/helix/chat/messages", post(mock_send_chat_message))
            .with_state(state)
    }

    async fn spawn_mock_twitch_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock twitch server");
        let address = listener.local_addr().expect("mock twitch server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock twitch server");
        });
        let base_url = format!("http://{}", address);
        (base_url, handle)
    }

    async fn mock_validate_token(
        State(state): State<MockTwitchState>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let authorization = header_text(headers.get("authorization"));
        {
            let mut validate_headers = state
                .validate_headers
                .lock()
                .expect("validate headers lock");
            validate_headers.push(authorization);
        }

        let scopes = {
            let validate_scopes = state.validate_scopes.lock().expect("scopes lock");
            if validate_scopes.is_empty() {
                vec![TWITCH_USER_WRITE_CHAT_SCOPE.to_owned()]
            } else {
                validate_scopes.clone()
            }
        };

        let body = serde_json::json!({
            "client_id": "client-123",
            "login": "loongclaw-bot",
            "scopes": scopes,
            "user_id": "sender-123",
        });
        Json(body)
    }

    async fn mock_get_users(
        State(state): State<MockTwitchState>,
        headers: HeaderMap,
        uri: axum::http::Uri,
    ) -> impl IntoResponse {
        let raw_query = uri.query().unwrap_or_default();
        let login_entry = raw_query
            .split('&')
            .find_map(|entry| entry.strip_prefix("login="));
        let id_entry = raw_query
            .split('&')
            .find_map(|entry| entry.strip_prefix("id="));
        let (query_key, query_value) = if let Some(login_entry) = login_entry {
            ("login", login_entry.to_owned())
        } else if let Some(id_entry) = id_entry {
            ("id", id_entry.to_owned())
        } else {
            ("", String::new())
        };
        {
            let mut users_queries = state.users_queries.lock().expect("users queries lock");
            users_queries.push(format!("{query_key}={query_value}"));
        }

        let authorization = header_text(headers.get("authorization"));
        let client_id = header_text(headers.get("client-id"));
        {
            let mut users_headers = state.users_headers.lock().expect("users headers lock");
            users_headers.push(format!("{authorization} | {client_id}"));
        }

        let body = serde_json::json!({
            "data": [{
                "id": "broadcaster-456",
            }],
        });
        Json(body)
    }

    async fn mock_send_chat_message(
        State(state): State<MockTwitchState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        let authorization = header_text(headers.get("authorization"));
        let client_id = header_text(headers.get("client-id"));
        {
            let mut send_headers = state.send_headers.lock().expect("send headers lock");
            send_headers.push(format!("{authorization} | {client_id}"));
        }
        {
            let mut send_bodies = state.send_bodies.lock().expect("send bodies lock");
            send_bodies.push(body);
        }

        let response_body = serde_json::json!({
            "data": [{
                "is_sent": true,
                "drop_reason": null,
            }],
        });
        (StatusCode::OK, Json(response_body))
    }

    fn header_text(raw: Option<&HeaderValue>) -> String {
        raw.and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_default()
    }
}
