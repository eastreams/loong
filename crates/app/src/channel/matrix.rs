use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use reqwest::Url;
use serde_json::{Value, json};

use crate::CliResult;
use crate::config::{self, ResolvedMatrixChannelConfig};

use super::{
    ChannelAdapter, ChannelDelivery, ChannelInboundMessage, ChannelOutboundMessage,
    ChannelOutboundTarget, ChannelOutboundTargetKind, ChannelPlatform, ChannelSession,
    access_policy::ChannelInboundAccessPolicy,
};

pub(super) struct MatrixAdapter {
    account_id: String,
    user_id: Option<String>,
    access_token: String,
    base_url: String,
    timeout_ms: u64,
    access_policy: ChannelInboundAccessPolicy<String>,
    require_mention: bool,
    ignore_self_messages: bool,
    cursor_tracker: MatrixSyncCursorTracker,
}

struct MatrixSyncCursorTracker {
    cursor_path: PathBuf,
    current_cursor: Option<String>,
    pending_cursor: Option<String>,
}

const MATRIX_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MATRIX_SEND_TIMEOUT: Duration = Duration::from_secs(30);

impl MatrixSyncCursorTracker {
    fn new(cursor_path: PathBuf, current_cursor: Option<String>) -> Self {
        Self {
            cursor_path,
            current_cursor,
            pending_cursor: None,
        }
    }

    fn current_cursor(&self) -> Option<&str> {
        self.current_cursor.as_deref()
    }

    fn remember_polled_cursor(&mut self, next_cursor: Option<String>) {
        self.pending_cursor = next_cursor
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| Some(*value) != self.current_cursor())
            .map(str::to_owned);
    }

    fn complete_batch(&mut self) -> CliResult<()> {
        let Some(next_cursor) = self.pending_cursor.take() else {
            return Ok(());
        };
        save_cursor(&self.cursor_path, next_cursor.as_str())?;
        self.current_cursor = Some(next_cursor);
        Ok(())
    }
}

impl MatrixAdapter {
    pub(super) fn new(config: &ResolvedMatrixChannelConfig, access_token: String) -> Self {
        let cursor_home = config::default_loongclaw_home();
        let cursor_path =
            matrix_sync_cursor_path_for_account(cursor_home.as_path(), config.account.id.as_str());
        let current_cursor =
            load_cursor_for_account(cursor_home.as_path(), config.account.id.as_str());
        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            config.allowed_room_ids.as_slice(),
            config.allowed_sender_ids.as_slice(),
            false,
        );

        Self {
            account_id: config.account.id.clone(),
            user_id: config.user_id.clone(),
            access_token,
            base_url: config.resolved_base_url().unwrap_or_default(),
            timeout_ms: config.sync_timeout_s.clamp(1, 300).saturating_mul(1000),
            access_policy,
            require_mention: config.require_mention,
            ignore_self_messages: config.ignore_self_messages,
            cursor_tracker: MatrixSyncCursorTracker::new(cursor_path, current_cursor),
        }
    }

    fn sync_url(&self) -> CliResult<Url> {
        let mut url = build_matrix_client_url(self.base_url.as_str())?;
        url.path_segments_mut()
            .map_err(|_cannot_be_a_base| {
                "matrix base_url cannot be used as a homeserver URL".to_owned()
            })?
            .extend(["_matrix", "client", "v3", "sync"]);
        Ok(url)
    }

    fn send_event_url(&self, room_id: &str, txn_id: &str) -> CliResult<Url> {
        let mut url = build_matrix_client_url(self.base_url.as_str())?;
        url.path_segments_mut()
            .map_err(|_cannot_be_a_base| {
                "matrix base_url cannot be used as a homeserver URL".to_owned()
            })?
            .extend([
                "_matrix",
                "client",
                "v3",
                "rooms",
                room_id,
                "send",
                "m.room.message",
                txn_id,
            ]);
        Ok(url)
    }
}

#[async_trait]
impl ChannelAdapter for MatrixAdapter {
    fn name(&self) -> &str {
        "matrix"
    }

    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
        let client = build_matrix_http_client()?;
        let mut request = client.get(self.sync_url()?);
        let mut query = vec![
            ("timeout", self.timeout_ms.to_string()),
            ("full_state", "false".to_owned()),
        ];
        if let Some(since) = self.cursor_tracker.current_cursor() {
            query.push(("since", since.to_owned()));
        }
        request = request
            .query(&query)
            .bearer_auth(self.access_token.as_str());

        let response = request
            .timeout(matrix_sync_request_timeout(self.timeout_ms))
            .send()
            .await
            .map_err(|error| format!("matrix sync failed: {error}"))?;
        let payload = read_matrix_json_response(response, "matrix sync").await?;

        let (inbox, next_cursor) = parse_matrix_sync_response(
            &payload,
            &self.access_policy,
            self.account_id.as_str(),
            self.user_id.as_deref(),
            self.ignore_self_messages,
            self.require_mention,
        )?;
        self.cursor_tracker.remember_polled_cursor(next_cursor);
        Ok(inbox)
    }

    async fn send_message(
        &self,
        target: &ChannelOutboundTarget,
        message: &ChannelOutboundMessage,
    ) -> CliResult<()> {
        if target.platform != ChannelPlatform::Matrix {
            return Err(format!(
                "matrix adapter cannot send to {} target",
                target.platform.as_str()
            ));
        }
        if target.kind != ChannelOutboundTargetKind::Conversation {
            return Err(format!(
                "matrix adapter requires conversation target, got {}",
                target.kind.as_str()
            ));
        }
        let text = match message {
            ChannelOutboundMessage::Text(text) => text,
            other @ ChannelOutboundMessage::MarkdownCard(_)
            | other @ ChannelOutboundMessage::Post(_)
            | other @ ChannelOutboundMessage::Image { .. }
            | other @ ChannelOutboundMessage::File { .. } => {
                return Err(format!(
                    "matrix adapter only supports plain text outbound messages, got {other:?}"
                ));
            }
        };

        let room_id = target.trimmed_id()?;
        let txn_id = target
            .idempotency_key()
            .map(str::to_owned)
            .unwrap_or_else(next_matrix_transaction_id);
        let body = json!({
            "msgtype": "m.text",
            "body": text,
        });

        let client = build_matrix_http_client()?;
        let response = client
            .put(self.send_event_url(room_id, txn_id.as_str())?)
            .bearer_auth(self.access_token.as_str())
            .json(&body)
            .timeout(matrix_send_request_timeout())
            .send()
            .await
            .map_err(|error| format!("matrix send failed: {error}"))?;
        let payload = read_matrix_json_response(response, "matrix send").await?;
        if payload
            .get("event_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(format!("matrix send did not return an event_id: {payload}"));
        }
        Ok(())
    }

    async fn complete_batch(&mut self) -> CliResult<()> {
        self.cursor_tracker.complete_batch()
    }
}

pub(super) async fn run_matrix_send(
    config: &ResolvedMatrixChannelConfig,
    access_token: String,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    let adapter = MatrixAdapter::new(config, access_token);
    let target = build_matrix_send_target(target_kind, target_id)?;
    adapter.send_text(&target, text).await
}

fn build_matrix_send_target(
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
) -> CliResult<ChannelOutboundTarget> {
    if target_kind != ChannelOutboundTargetKind::Conversation {
        return Err(format!(
            "matrix send requires conversation target kind, got {}",
            target_kind.as_str()
        ));
    }

    let trimmed_target_id = target_id.trim();
    if trimmed_target_id.is_empty() {
        return Err("matrix outbound target id is empty".to_owned());
    }

    Ok(ChannelOutboundTarget::new(
        ChannelPlatform::Matrix,
        target_kind,
        trimmed_target_id.to_owned(),
    ))
}

pub(super) fn parse_matrix_sync_response(
    payload: &Value,
    access_policy: &ChannelInboundAccessPolicy<String>,
    account_id: &str,
    user_id: Option<&str>,
    ignore_self_messages: bool,
    require_mention: bool,
) -> CliResult<(Vec<ChannelInboundMessage>, Option<String>)> {
    let next_cursor = payload
        .get("next_batch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            "matrix sync response is missing a non-empty next_batch cursor".to_owned()
        })?;
    let joined_rooms = payload
        .get("rooms")
        .and_then(|rooms| rooms.get("join"))
        .and_then(Value::as_object);

    let mut inbox = Vec::new();
    for (room_id, room) in joined_rooms.into_iter().flat_map(|rooms| rooms.iter()) {
        let room_id = room_id.trim();
        if room_id.is_empty() {
            continue;
        }

        let events = room
            .get("timeline")
            .and_then(|timeline| timeline.get("events"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for event in events {
            if event.get("type").and_then(Value::as_str) != Some("m.room.message") {
                continue;
            }

            let content = event.get("content").cloned().unwrap_or(Value::Null);
            if is_matrix_replacement_event(&content) {
                continue;
            }
            if content.get("msgtype").and_then(Value::as_str) != Some("m.text") {
                continue;
            }
            let Some(text) = content
                .get("body")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
            else {
                continue;
            };

            let sender = event
                .get("sender")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if ignore_self_messages
                && sender
                    .as_deref()
                    .zip(user_id.map(str::trim))
                    .is_some_and(|(sender, user_id)| sender == user_id)
            {
                continue;
            }
            let event_id = event
                .get("event_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            let sender_ref = sender.as_deref();
            let allowed = access_policy.allows_str(room_id, sender_ref);
            if !allowed {
                continue;
            }
            if require_mention && !matrix_message_mentions_user(&content, text.as_str(), user_id) {
                continue;
            }

            let mut session =
                ChannelSession::with_account(ChannelPlatform::Matrix, account_id, room_id);
            if let Some(sender) = sender.as_deref() {
                session = session.with_participant_id(sender.to_owned());
            }
            let mut reply_target = ChannelOutboundTarget::new(
                ChannelPlatform::Matrix,
                ChannelOutboundTargetKind::Conversation,
                room_id.to_owned(),
            );
            if let Some(event_id) = event_id.as_deref() {
                reply_target = reply_target.with_idempotency_key(event_id.to_owned());
            }

            inbox.push(ChannelInboundMessage {
                session,
                reply_target,
                text,
                delivery: ChannelDelivery {
                    ack_cursor: None,
                    source_message_id: event_id,
                    sender_principal_key: sender.map(|sender| format!("matrix:user:{sender}")),
                    thread_root_id: None,
                    parent_message_id: None,
                    resources: Vec::new(),
                    feishu_callback: None,
                },
            });
        }
    }

    Ok((inbox, Some(next_cursor)))
}

fn matrix_message_mentions_user(content: &Value, body: &str, user_id: Option<&str>) -> bool {
    let Some(user_id) = user_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };

    let mentions = content.get("m.mentions");
    let room_mention = mentions
        .and_then(|mentions| mentions.get("room"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if room_mention {
        return true;
    }

    let user_mentions = mentions
        .and_then(|mentions| mentions.get("user_ids"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for mentioned_user in user_mentions {
        let mentioned_user = mentioned_user.as_str().map(str::trim).unwrap_or_default();
        if mentioned_user == user_id {
            return true;
        }
    }

    if body.contains(user_id) {
        return true;
    }

    content
        .get("formatted_body")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|formatted_body| formatted_body.contains(user_id))
}

pub(super) fn build_matrix_client_url(base_url: &str) -> CliResult<Url> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("matrix.base_url is missing; configure a homeserver base url".to_owned());
    }
    Url::parse(trimmed).map_err(|error| format!("invalid matrix base_url `{trimmed}`: {error}"))
}

fn build_matrix_http_client() -> CliResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(MATRIX_HTTP_CONNECT_TIMEOUT)
        .build()
        .map_err(|error| format!("build matrix http client failed: {error}"))
}

fn matrix_sync_request_timeout(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms.saturating_add(5_000))
}

fn matrix_send_request_timeout() -> Duration {
    MATRIX_SEND_TIMEOUT
}

async fn read_matrix_json_response(response: reqwest::Response, context: &str) -> CliResult<Value> {
    let status = response.status();
    let raw = response
        .text()
        .await
        .map_err(|error| format!("{context} decode failed: {error}"))?;
    let payload =
        serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| Value::String(raw.clone()));
    if !status.is_success() {
        return Err(format!("{context} returned {}: {payload}", status.as_u16()));
    }
    Ok(payload)
}

fn is_matrix_replacement_event(content: &Value) -> bool {
    content
        .get("m.relates_to")
        .and_then(|relation| relation.get("rel_type"))
        .and_then(Value::as_str)
        == Some("m.replace")
}

fn matrix_sync_cursor_path_for_account(loongclaw_home: &Path, account_id: &str) -> PathBuf {
    loongclaw_home
        .join("matrix-sync-cursors")
        .join(format!("{}.cursor", account_id.trim()))
}

fn load_cursor_for_account(loongclaw_home: &Path, account_id: &str) -> Option<String> {
    let account_path = matrix_sync_cursor_path_for_account(loongclaw_home, account_id);
    load_cursor(&account_path)
}

fn load_cursor(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn save_cursor(path: &Path, cursor: &str) -> CliResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create matrix sync cursor directory failed: {error}"))?;
    }
    fs::write(path, cursor)
        .map_err(|error| format!("write matrix sync cursor file failed: {error}"))?;
    Ok(())
}

fn next_matrix_transaction_id() -> String {
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("loongclaw-{now_ns}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        http::StatusCode,
        routing::{get, put},
    };
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockRequest {
        method: String,
        path: String,
        query: Option<String>,
        authorization: Option<String>,
        body: String,
    }

    #[derive(Clone, Default)]
    struct MockServerState {
        requests: Arc<Mutex<Vec<MockRequest>>>,
    }

    async fn spawn_mock_matrix_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock matrix server");
        let address = listener.local_addr().expect("mock matrix server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock matrix api");
        });
        (format!("http://{address}"), handle)
    }

    async fn record_request(
        State(state): State<MockServerState>,
        request: Request,
    ) -> (StatusCode, Json<Value>) {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, usize::MAX)
            .await
            .expect("read mock matrix request body");
        state.requests.lock().await.push(MockRequest {
            method: parts.method.to_string(),
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(ToOwned::to_owned),
            authorization: parts
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body: String::from_utf8(body.to_vec()).expect("request body utf-8"),
        });
        (StatusCode::OK, Json(json!({ "event_id": "$sent:event" })))
    }

    fn matrix_test_config(base_url: &str) -> ResolvedMatrixChannelConfig {
        let config: crate::config::LoongClawConfig = serde_json::from_value(json!({
            "matrix": {
                "enabled": true,
                "account_id": "Ops Bot",
                "user_id": "@ops-bot:example.org",
                "access_token": "matrix-token",
                "base_url": base_url,
                "allowed_room_ids": ["!ops:example.org"],
                "ignore_self_messages": true
            }
        }))
        .expect("deserialize matrix test config");

        config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix test config")
    }

    #[test]
    fn matrix_parser_filters_allowlisted_text_events_and_updates_cursor() {
        let payload = json!({
            "next_batch": "s72595_4483_1934",
            "rooms": {
                "join": {
                    "!ops:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$first:example.org",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "hello matrix"
                                    }
                                }
                            ]
                        }
                    },
                    "!blocked:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$blocked:example.org",
                                    "type": "m.room.message",
                                    "sender": "@bob:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "should be filtered"
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &[],
            false,
        );
        let (inbox, next_cursor) = parse_matrix_sync_response(
            &payload,
            &access_policy,
            "ops-bot",
            Some("@ops-bot:example.org"),
            true,
            false,
        )
        .expect("parse matrix sync response");

        assert_eq!(next_cursor.as_deref(), Some("s72595_4483_1934"));
        assert_eq!(inbox.len(), 1);
        assert_eq!(
            inbox[0].session.session_key(),
            "matrix:ops-bot:~b64~IW9wczpleGFtcGxlLm9yZw:~b64~QGFsaWNlOmV4YW1wbGUub3Jn"
        );
        assert_eq!(inbox[0].text, "hello matrix");
        assert_eq!(inbox[0].reply_target.id, "!ops:example.org");
        assert_eq!(
            inbox[0].delivery.sender_principal_key.as_deref(),
            Some("matrix:user:@alice:example.org")
        );
    }

    #[test]
    fn matrix_parser_requires_non_empty_next_batch_cursor() {
        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &[],
            false,
        );

        for payload in [
            json!({
                "rooms": {
                    "join": {}
                }
            }),
            json!({
                "next_batch": "   ",
                "rooms": {
                    "join": {}
                }
            }),
        ] {
            let error = parse_matrix_sync_response(
                &payload,
                &access_policy,
                "ops-bot",
                Some("@ops-bot:example.org"),
                true,
                false,
            )
            .expect_err("missing or empty next_batch should be rejected");
            assert!(
                error.contains("next_batch"),
                "parser error should mention next_batch, got: {error}"
            );
        }
    }

    #[test]
    fn matrix_parser_ignores_self_authored_events_when_enabled() {
        let payload = json!({
            "next_batch": "next",
            "rooms": {
                "join": {
                    "!ops:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$self:example.org",
                                    "type": "m.room.message",
                                    "sender": "@ops-bot:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "ignore me"
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &[],
            false,
        );
        let (inbox, next_cursor) = parse_matrix_sync_response(
            &payload,
            &access_policy,
            "ops-bot",
            Some("@ops-bot:example.org"),
            true,
            false,
        )
        .expect("parse matrix sync response");

        assert!(inbox.is_empty());
        assert_eq!(next_cursor.as_deref(), Some("next"));
    }

    #[test]
    fn matrix_parser_reuses_event_id_for_reply_idempotency() {
        let payload = json!({
            "next_batch": "next",
            "rooms": {
                "join": {
                    "!ops:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "  $reply:example.org  ",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "hello matrix"
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &[],
            false,
        );
        let (inbox, _) = parse_matrix_sync_response(
            &payload,
            &access_policy,
            "ops-bot",
            Some("@ops-bot:example.org"),
            true,
            false,
        )
        .expect("parse matrix sync response");

        assert_eq!(inbox.len(), 1);
        assert_eq!(
            inbox[0].delivery.source_message_id.as_deref(),
            Some("$reply:example.org")
        );
        assert_eq!(
            inbox[0].reply_target.idempotency_key(),
            Some("$reply:example.org")
        );
    }

    #[test]
    fn matrix_parser_filters_non_allowlisted_senders() {
        let payload = json!({
            "next_batch": "s72595_4483_1934",
            "rooms": {
                "join": {
                    "!ops:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$allowed:example.org",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "hello matrix"
                                    }
                                },
                                {
                                    "event_id": "$blocked:example.org",
                                    "type": "m.room.message",
                                    "sender": "@bob:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "blocked matrix"
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &["@alice:example.org".to_owned()],
            false,
        );
        let (inbox, next_cursor) = parse_matrix_sync_response(
            &payload,
            &access_policy,
            "ops-bot",
            Some("@ops-bot:example.org"),
            true,
            false,
        )
        .expect("parse matrix sync response");

        assert_eq!(next_cursor.as_deref(), Some("s72595_4483_1934"));
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].text, "hello matrix");
        assert_eq!(
            inbox[0].delivery.sender_principal_key.as_deref(),
            Some("matrix:user:@alice:example.org")
        );
    }

    #[test]
    fn matrix_parser_requires_explicit_mentions_when_enabled() {
        let payload = json!({
            "next_batch": "next",
            "rooms": {
                "join": {
                    "!ops:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$plain:example.org",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "hello team"
                                    }
                                },
                                {
                                    "event_id": "$mention:example.org",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "hello @ops-bot:example.org",
                                        "m.mentions": {
                                            "user_ids": ["@ops-bot:example.org"]
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &[],
            false,
        );
        let (inbox, next_cursor) = parse_matrix_sync_response(
            &payload,
            &access_policy,
            "ops-bot",
            Some("@ops-bot:example.org"),
            true,
            true,
        )
        .expect("parse matrix sync response");

        assert_eq!(next_cursor.as_deref(), Some("next"));
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].text, "hello @ops-bot:example.org");
    }

    #[test]
    fn matrix_parser_accepts_room_mentions_when_enabled() {
        let payload = json!({
            "next_batch": "next",
            "rooms": {
                "join": {
                    "!ops:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$room:example.org",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "attention room",
                                        "m.mentions": {
                                            "room": true
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &[],
            false,
        );
        let (inbox, _next_cursor) = parse_matrix_sync_response(
            &payload,
            &access_policy,
            "ops-bot",
            Some("@ops-bot:example.org"),
            true,
            true,
        )
        .expect("parse matrix sync response");

        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].text, "attention room");
    }

    #[test]
    fn matrix_parser_ignores_non_text_and_replacement_events() {
        let payload = json!({
            "next_batch": "next",
            "rooms": {
                "join": {
                    "!ops:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$image:example.org",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.image",
                                        "body": "image"
                                    }
                                },
                                {
                                    "event_id": "$edit:example.org",
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "edited",
                                        "m.relates_to": {
                                            "rel_type": "m.replace",
                                            "event_id": "$prior:example.org"
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            &["!ops:example.org".to_owned()],
            &[],
            false,
        );
        let (inbox, next_cursor) = parse_matrix_sync_response(
            &payload,
            &access_policy,
            "ops-bot",
            Some("@ops-bot:example.org"),
            true,
            false,
        )
        .expect("parse matrix sync response");

        assert!(inbox.is_empty());
        assert_eq!(next_cursor.as_deref(), Some("next"));
    }

    #[tokio::test]
    async fn matrix_send_writes_m_room_message_payload() {
        let state = MockServerState::default();
        let router = Router::new()
            .route(
                "/_matrix/client/v3/rooms/{room_id}/send/m.room.message/{txn_id}",
                put(record_request),
            )
            .with_state(state.clone());
        let (base_url, handle) = spawn_mock_matrix_server(router).await;

        let config = matrix_test_config(base_url.as_str());
        let adapter = MatrixAdapter::new(&config, "matrix-token".to_owned());
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Matrix,
            ChannelOutboundTargetKind::Conversation,
            "!ops:example.org",
        );

        adapter
            .send_text(&target, "hello room")
            .await
            .expect("matrix send should succeed");

        let requests = state.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "PUT");
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer matrix-token")
        );
        assert!(
            requests[0].path.starts_with("/_matrix/client/v3/rooms/"),
            "matrix send path should include the rooms collection: {:?}",
            requests[0].path
        );
        assert!(
            requests[0].path.contains("/send/m.room.message/"),
            "matrix send path should target m.room.message: {:?}",
            requests[0].path
        );
        assert!(
            requests[0].path.contains("%21ops%3Aexample.org")
                || requests[0].path.contains("!ops:example.org"),
            "matrix room id should survive URL path construction: {:?}",
            requests[0].path
        );
        let body: Value = serde_json::from_str(&requests[0].body).expect("request body json");
        assert_eq!(body["msgtype"], "m.text");
        assert_eq!(body["body"], "hello room");

        handle.abort();
    }

    #[test]
    fn matrix_sync_cursor_is_not_persisted_until_batch_completion() {
        let unique = format!(
            "loongclaw-matrix-sync-cursor-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique).join("cursor.txt");
        let mut tracker = MatrixSyncCursorTracker::new(path.clone(), None);

        tracker.remember_polled_cursor(Some("next-sync-cursor".to_owned()));

        assert_eq!(load_cursor(&path), None);
        assert_eq!(tracker.current_cursor(), None);

        tracker.complete_batch().expect("complete batch");

        assert_eq!(load_cursor(&path).as_deref(), Some("next-sync-cursor"));
        assert_eq!(tracker.current_cursor(), Some("next-sync-cursor"));
    }

    #[test]
    fn matrix_sync_cursor_path_is_account_scoped() {
        let home = std::env::temp_dir().join("loongclaw-matrix-account-cursor");
        let path = matrix_sync_cursor_path_for_account(home.as_path(), "ops-bot");

        assert!(path.ends_with("matrix-sync-cursors/ops-bot.cursor"));
    }

    #[tokio::test]
    async fn matrix_adapter_receive_batch_persists_sync_cursor_after_completion() {
        async fn serve_sync(
            State(state): State<MockServerState>,
            request: Request,
        ) -> (StatusCode, Json<Value>) {
            let (parts, _body) = request.into_parts();
            state.requests.lock().await.push(MockRequest {
                method: parts.method.to_string(),
                path: parts.uri.path().to_owned(),
                query: parts.uri.query().map(ToOwned::to_owned),
                authorization: parts
                    .headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(ToOwned::to_owned),
                body: String::new(),
            });
            (
                StatusCode::OK,
                Json(json!({
                    "next_batch": "s123",
                    "rooms": {
                        "join": {
                            "!ops:example.org": {
                                "timeline": {
                                    "events": [
                                        {
                                            "event_id": "$room:event",
                                            "type": "m.room.message",
                                            "sender": "@alice:example.org",
                                            "content": {
                                                "msgtype": "m.text",
                                                "body": "hello"
                                            }
                                        }
                                    ]
                                }
                            }
                        }
                    }
                })),
            )
        }

        let state = MockServerState::default();
        let router = Router::new()
            .route("/_matrix/client/v3/sync", get(serve_sync))
            .with_state(state);
        let (base_url, handle) = spawn_mock_matrix_server(router).await;

        let config = matrix_test_config(base_url.as_str());
        let cursor_home = config::default_loongclaw_home();
        let cursor_path =
            matrix_sync_cursor_path_for_account(cursor_home.as_path(), config.account.id.as_str());
        std::fs::remove_file(&cursor_path).ok();

        let mut adapter = MatrixAdapter::new(&config, "matrix-token".to_owned());
        let batch = adapter.receive_batch().await.expect("receive batch");
        assert_eq!(batch.len(), 1);
        assert_eq!(load_cursor(&cursor_path), None);

        adapter.complete_batch().await.expect("complete batch");
        assert_eq!(load_cursor(&cursor_path).as_deref(), Some("s123"));

        handle.abort();
    }

    #[test]
    fn matrix_sync_request_timeout_adds_network_slack() {
        assert_eq!(
            matrix_sync_request_timeout(1_000),
            std::time::Duration::from_millis(6_000)
        );
        assert_eq!(
            matrix_sync_request_timeout(300_000),
            std::time::Duration::from_millis(305_000)
        );
    }

    #[test]
    fn matrix_send_request_timeout_is_bounded() {
        assert_eq!(
            matrix_send_request_timeout(),
            std::time::Duration::from_secs(30)
        );
    }
}
