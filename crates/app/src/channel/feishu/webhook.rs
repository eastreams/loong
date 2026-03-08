use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::channel::{process_inbound_with_provider, ChannelInboundMessage};
use crate::config::LoongClawConfig;
use crate::KernelContext;

use super::adapter::FeishuAdapter;
use super::payload::FeishuWebhookAction;

#[derive(Clone)]
pub(super) struct FeishuWebhookState {
    config: LoongClawConfig,
    adapter: Arc<Mutex<FeishuAdapter>>,
    verification_token: Option<String>,
    encrypt_key: Option<String>,
    allowed_chat_ids: BTreeSet<String>,
    ignore_bot_messages: bool,
    seen_events: Arc<Mutex<RecentIdCache>>,
    kernel_ctx: Arc<KernelContext>,
}

impl FeishuWebhookState {
    pub(super) fn new(
        config: LoongClawConfig,
        adapter: FeishuAdapter,
        kernel_ctx: KernelContext,
    ) -> Self {
        Self {
            verification_token: config.feishu.verification_token(),
            encrypt_key: config.feishu.encrypt_key(),
            allowed_chat_ids: config
                .feishu
                .allowed_chat_ids
                .iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect(),
            ignore_bot_messages: config.feishu.ignore_bot_messages,
            config,
            adapter: Arc::new(Mutex::new(adapter)),
            seen_events: Arc::new(Mutex::new(RecentIdCache::new(2_048))),
            kernel_ctx: Arc::new(kernel_ctx),
        }
    }
}

struct RecentIdCache {
    max_len: usize,
    queue: VecDeque<String>,
    set: BTreeSet<String>,
}

impl RecentIdCache {
    fn new(max_len: usize) -> Self {
        Self {
            max_len: max_len.max(1),
            queue: VecDeque::new(),
            set: BTreeSet::new(),
        }
    }

    fn insert_if_new(&mut self, id: &str) -> bool {
        let id = id.trim();
        if id.is_empty() {
            return false;
        }
        if self.set.contains(id) {
            return false;
        }

        self.queue.push_back(id.to_owned());
        self.set.insert(id.to_owned());
        while self.queue.len() > self.max_len {
            if let Some(removed) = self.queue.pop_front() {
                self.set.remove(&removed);
            }
        }
        true
    }
}

pub(super) async fn feishu_webhook_handler(
    State(state): State<FeishuWebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let body_text = match std::str::from_utf8(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "code": StatusCode::BAD_REQUEST.as_u16(),
                    "msg": format!("invalid utf-8 request body: {error}"),
                })),
            );
        }
    };
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "code": StatusCode::BAD_REQUEST.as_u16(),
                    "msg": format!("invalid JSON request body: {error}"),
                })),
            );
        }
    };

    match handle_feishu_webhook_payload(state, &headers, body_text, payload).await {
        Ok(reply) => (StatusCode::OK, Json(reply)),
        Err((status, message)) => (
            status,
            Json(json!({
                "code": status.as_u16(),
                "msg": message,
            })),
        ),
    }
}

async fn handle_feishu_webhook_payload(
    state: FeishuWebhookState,
    headers: &HeaderMap,
    raw_body: &str,
    payload: Value,
) -> Result<Value, (StatusCode, String)> {
    verify_feishu_signature(headers, raw_body, &payload, state.encrypt_key.as_deref())?;

    let parsed = super::payload::parse_feishu_webhook_payload(
        &payload,
        state.verification_token.as_deref(),
        state.encrypt_key.as_deref(),
        &state.allowed_chat_ids,
        state.ignore_bot_messages,
    )
    .map_err(map_feishu_parse_error)?;

    match parsed {
        FeishuWebhookAction::UrlVerification { challenge } => Ok(json!({ "challenge": challenge })),
        FeishuWebhookAction::Ignore => Ok(json!({"code": 0, "msg": "ignored"})),
        FeishuWebhookAction::Inbound(event) => {
            {
                let mut dedupe = state.seen_events.lock().await;
                if !dedupe.insert_if_new(&event.event_id) {
                    return Ok(json!({"code": 0, "msg": "duplicate_event"}));
                }
            }

            let channel_message = ChannelInboundMessage {
                session_id: event.session_id,
                reply_target: event.message_id.clone(),
                text: event.text,
            };
            let reply = process_inbound_with_provider(
                &state.config,
                &channel_message,
                Some(&state.kernel_ctx),
            )
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("provider processing failed: {error}"),
                )
            })?;

            let mut adapter = state.adapter.lock().await;
            if let Err(first_error) = adapter.send_reply(&event.message_id, &reply).await {
                adapter.refresh_tenant_token().await.map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "feishu token refresh failed after send error `{first_error}`: {error}"
                        ),
                    )
                })?;
                adapter
                    .send_reply(&event.message_id, &reply)
                    .await
                    .map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("feishu reply failed after token refresh: {error}"),
                        )
                    })?;
            }

            Ok(json!({"code": 0, "msg": "ok"}))
        }
    }
}

fn map_feishu_parse_error(error: String) -> (StatusCode, String) {
    if let Some(message) = error.strip_prefix("unauthorized:") {
        return (StatusCode::UNAUTHORIZED, message.trim().to_owned());
    }
    (StatusCode::BAD_REQUEST, error)
}

fn verify_feishu_signature(
    headers: &HeaderMap,
    raw_body: &str,
    payload: &Value,
    encrypt_key: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        return Ok(());
    }

    let Some(encrypt_key) = encrypt_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };

    let timestamp = read_header_required(headers, "X-Lark-Request-Timestamp")?;
    let nonce = read_header_required(headers, "X-Lark-Request-Nonce")?;
    let signature = read_header_required(headers, "X-Lark-Signature")?;

    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(encrypt_key.as_bytes());
    hasher.update(raw_body.as_bytes());
    let expected = format!("{:x}", hasher.finalize());

    if expected != signature {
        return Err((
            StatusCode::UNAUTHORIZED,
            "unauthorized: feishu signature mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn read_header_required<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, (StatusCode, String)> {
    let value = headers
        .get(name)
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                format!("unauthorized: missing required header `{name}`"),
            )
        })?
        .to_str()
        .map_err(|error| {
            (
                StatusCode::UNAUTHORIZED,
                format!("unauthorized: invalid header `{name}`: {error}"),
            )
        })?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!("unauthorized: empty required header `{name}`"),
        ));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn recent_cache_deduplicates_and_rolls_window() {
        let mut cache = RecentIdCache::new(2);
        assert!(cache.insert_if_new("a"));
        assert!(!cache.insert_if_new("a"));
        assert!(cache.insert_if_new("b"));
        assert!(cache.insert_if_new("c"));
        assert!(cache.insert_if_new("a"));
    }

    #[test]
    fn signature_verification_passes_with_valid_headers() {
        let body = r#"{"encrypt":"opaque"}"#;
        let encrypt_key = "test-encrypt-key";
        let timestamp = "1736480000";
        let nonce = "nonce-1";

        let mut hasher = Sha256::new();
        hasher.update(timestamp.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(encrypt_key.as_bytes());
        hasher.update(body.as_bytes());
        let signature = format!("{:x}", hasher.finalize());

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Lark-Request-Timestamp",
            timestamp.parse().expect("header"),
        );
        headers.insert("X-Lark-Request-Nonce", nonce.parse().expect("header"));
        headers.insert("X-Lark-Signature", signature.parse().expect("header"));

        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let result = verify_feishu_signature(&headers, body, &payload, Some(encrypt_key));
        assert!(result.is_ok());
    }

    #[test]
    fn signature_verification_rejects_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Lark-Request-Timestamp", "1".parse().expect("header"));
        headers.insert("X-Lark-Request-Nonce", "n".parse().expect("header"));
        headers.insert("X-Lark-Signature", "deadbeef".parse().expect("header"));

        let body = "{\"encrypt\":\"x\"}";
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let error =
            verify_feishu_signature(&headers, body, &payload, Some("key")).expect_err("mismatch");
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn signature_verification_skips_url_verification_payload() {
        let headers = HeaderMap::new();
        let body = r#"{"type":"url_verification","token":"token","challenge":"ok"}"#;
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let result = verify_feishu_signature(&headers, body, &payload, Some("encrypt-key"));
        assert!(result.is_ok());
    }
}
