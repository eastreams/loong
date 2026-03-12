use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::KernelContext;
use crate::channel::{
    ChannelAdapter, ChannelInboundMessage, process_inbound_with_provider,
    runtime_state::{ChannelOperationRuntimeTracker, default_channel_runtime_state_dir},
};
use crate::config::{LoongClawConfig, ResolvedFeishuChannelConfig};

use super::adapter::FeishuAdapter;
use super::payload::FeishuWebhookAction;

const FEISHU_REPLAY_CACHE_MAX_LEN: usize = 2_048;
const FEISHU_REPLAY_TTL_SECONDS: i64 = 60 * 60;
const FEISHU_SIGNATURE_MAX_SKEW_SECONDS: i64 = 5 * 60;

#[derive(Clone)]
pub(super) struct FeishuWebhookState {
    config: LoongClawConfig,
    adapter: Arc<Mutex<FeishuAdapter>>,
    account_id: String,
    verification_token: Option<String>,
    encrypt_key: Option<String>,
    allowed_chat_ids: BTreeSet<String>,
    ignore_bot_messages: bool,
    seen_events: Arc<Mutex<RecentIdCache>>,
    kernel_ctx: Arc<KernelContext>,
    runtime: Arc<ChannelOperationRuntimeTracker>,
}

impl FeishuWebhookState {
    pub(super) fn new(
        config: LoongClawConfig,
        resolved: &ResolvedFeishuChannelConfig,
        adapter: FeishuAdapter,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> Result<Self, String> {
        let replay_cache_path = feishu_replay_cache_path(resolved.account.id.as_str());
        let seen_events = RecentIdCache::new_persisted(
            FEISHU_REPLAY_CACHE_MAX_LEN,
            FEISHU_REPLAY_TTL_SECONDS,
            replay_cache_path,
        )?;
        Ok(Self {
            account_id: resolved.account.id.clone(),
            verification_token: resolved.verification_token(),
            encrypt_key: resolved.encrypt_key(),
            allowed_chat_ids: resolved
                .allowed_chat_ids
                .iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect(),
            ignore_bot_messages: resolved.ignore_bot_messages,
            config,
            adapter: Arc::new(Mutex::new(adapter)),
            seen_events: Arc::new(Mutex::new(seen_events)),
            kernel_ctx: Arc::new(kernel_ctx),
            runtime,
        })
    }
}

struct RecentIdCache {
    max_len: usize,
    ttl_seconds: i64,
    persist_path: Option<PathBuf>,
    queue: VecDeque<String>,
    states: BTreeMap<String, RecentIdEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecentIdEntry {
    state: RecentIdState,
    updated_at: i64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedReplayCache {
    completed: Vec<PersistedReplayEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedReplayEntry {
    id: String,
    updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecentIdState {
    Processing,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecentIdReservation {
    Accepted,
    InProgressDuplicate,
    CompletedDuplicate,
}

impl RecentIdCache {
    #[cfg(test)]
    fn new(max_len: usize) -> Self {
        Self {
            max_len: max_len.max(1),
            ttl_seconds: FEISHU_REPLAY_TTL_SECONDS,
            persist_path: None,
            queue: VecDeque::new(),
            states: BTreeMap::new(),
        }
    }

    fn new_persisted(max_len: usize, ttl_seconds: i64, path: PathBuf) -> Result<Self, String> {
        let mut cache = Self {
            max_len: max_len.max(1),
            ttl_seconds: ttl_seconds.max(1),
            persist_path: Some(path.clone()),
            queue: VecDeque::new(),
            states: BTreeMap::new(),
        };

        let now = now_unix_timestamp_s()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create feishu replay cache directory failed: {error}"))?;
        }

        if path.exists() {
            let encoded = fs::read_to_string(&path)
                .map_err(|error| format!("read feishu replay cache failed: {error}"))?;
            if !encoded.trim().is_empty() {
                let persisted: PersistedReplayCache = serde_json::from_str(&encoded)
                    .map_err(|error| format!("parse feishu replay cache failed: {error}"))?;
                for entry in persisted.completed {
                    let id = entry.id.trim();
                    if id.is_empty() {
                        continue;
                    }
                    if now.saturating_sub(entry.updated_at) > cache.ttl_seconds {
                        continue;
                    }
                    cache.queue.push_back(id.to_owned());
                    cache.states.insert(
                        id.to_owned(),
                        RecentIdEntry {
                            state: RecentIdState::Completed,
                            updated_at: entry.updated_at,
                        },
                    );
                }
                cache.trim_to_max();
            }
        }

        Ok(cache)
    }

    fn begin_processing(&mut self, id: &str) -> RecentIdReservation {
        let now = match now_unix_timestamp_s() {
            Ok(value) => value,
            Err(_) => return RecentIdReservation::CompletedDuplicate,
        };
        self.prune_expired(now);

        let id = id.trim();
        if id.is_empty() {
            return RecentIdReservation::CompletedDuplicate;
        }
        if let Some(state) = self.states.get(id) {
            return match state.state {
                RecentIdState::Processing => RecentIdReservation::InProgressDuplicate,
                RecentIdState::Completed => RecentIdReservation::CompletedDuplicate,
            };
        }

        self.queue.push_back(id.to_owned());
        self.states.insert(
            id.to_owned(),
            RecentIdEntry {
                state: RecentIdState::Processing,
                updated_at: now,
            },
        );
        self.trim_to_max();
        RecentIdReservation::Accepted
    }

    fn mark_completed(&mut self, id: &str) {
        let now = match now_unix_timestamp_s() {
            Ok(value) => value,
            Err(_) => return,
        };
        self.prune_expired(now);
        let id = id.trim();
        if let Some(state) = self.states.get_mut(id) {
            state.state = RecentIdState::Completed;
            state.updated_at = now;
            let _ = self.persist_completed_entries();
        }
    }

    fn release(&mut self, id: &str) {
        let now = match now_unix_timestamp_s() {
            Ok(value) => value,
            Err(_) => return,
        };
        self.prune_expired(now);
        let id = id.trim();
        if self.states.remove(id).is_some() {
            self.queue.retain(|entry| entry != id);
            let _ = self.persist_completed_entries();
        }
    }

    fn trim_to_max(&mut self) {
        let mut removed_any = false;
        while self.queue.len() > self.max_len {
            if let Some(removed_id) = self.queue.pop_front() {
                self.states.remove(&removed_id);
                removed_any = true;
            }
        }
        if removed_any {
            let _ = self.persist_completed_entries();
        }
    }

    fn prune_expired(&mut self, now: i64) {
        let ttl = self.ttl_seconds;
        self.queue.retain(|id| {
            self.states
                .get(id)
                .map(|entry| now.saturating_sub(entry.updated_at) <= ttl)
                .unwrap_or(false)
        });

        self.states
            .retain(|_, entry| now.saturating_sub(entry.updated_at) <= ttl);
    }

    fn persist_completed_entries(&self) -> Result<(), String> {
        let Some(path) = self.persist_path.as_ref() else {
            return Ok(());
        };
        let mut completed = Vec::new();
        for id in &self.queue {
            let Some(entry) = self.states.get(id) else {
                continue;
            };
            if entry.state != RecentIdState::Completed {
                continue;
            }
            completed.push(PersistedReplayEntry {
                id: id.clone(),
                updated_at: entry.updated_at,
            });
        }
        let encoded = serde_json::to_string(&PersistedReplayCache { completed })
            .map_err(|error| format!("serialize feishu replay cache failed: {error}"))?;
        fs::write(path, encoded).map_err(|error| format!("write feishu replay cache failed: {error}"))
    }
}

fn feishu_replay_cache_path(account_id: &str) -> PathBuf {
    let runtime_dir = default_channel_runtime_state_dir();
    let sanitized_account = sanitize_path_component(account_id);
    runtime_dir.join(format!("feishu-webhook-replay-{sanitized_account}.json"))
}

fn sanitize_path_component(input: &str) -> String {
    let mut result = String::with_capacity(input.len().max(8));
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    let trimmed = result.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn now_unix_timestamp_s() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before unix epoch: {error}"))?;
    i64::try_from(duration.as_secs())
        .map_err(|error| format!("unix timestamp overflow: {error}"))
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
        state.account_id.as_str(),
    )
    .map_err(map_feishu_parse_error)?;

    match parsed {
        FeishuWebhookAction::UrlVerification { challenge } => Ok(json!({ "challenge": challenge })),
        FeishuWebhookAction::Ignore => Ok(json!({"code": 0, "msg": "ignored"})),
        FeishuWebhookAction::Inbound(event) => {
            {
                let mut dedupe = state.seen_events.lock().await;
                if !matches!(
                    dedupe.begin_processing(&event.event_id),
                    RecentIdReservation::Accepted
                ) {
                    return Ok(json!({"code": 0, "msg": "duplicate_event"}));
                }
            }

            let event_id = event.event_id.clone();
            let result = async {
                state.runtime.mark_run_start().await.map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("channel runtime start failed: {error}"),
                    )
                })?;
                let channel_message = ChannelInboundMessage {
                    session: event.session,
                    reply_target: event.reply_target,
                    text: event.text,
                    delivery: Default::default(),
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
                let reply_target = channel_message.reply_target.clone();

                let mut adapter = state.adapter.lock().await;
                if let Err(first_error) = adapter.send_text(&reply_target, &reply).await {
                    adapter.refresh_tenant_token().await.map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!(
                                "feishu token refresh failed after send error `{first_error}`: {error}"
                            ),
                        )
                    })?;
                    adapter.send_text(&reply_target, &reply).await.map_err(|error| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("feishu reply failed after token refresh: {error}"),
                        )
                    })?;
                }

                Ok(json!({"code": 0, "msg": "ok"}))
            }
            .await;
            let runtime_end_result = state.runtime.mark_run_end().await;
            let result = match (result, runtime_end_result) {
                (Ok(reply), Ok(())) => Ok(reply),
                (Err(error), _) => Err(error),
                (Ok(_), Err(error)) => Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("channel runtime end failed: {error}"),
                )),
            };

            {
                let mut dedupe = state.seen_events.lock().await;
                if result.is_ok() {
                    dedupe.mark_completed(&event_id);
                } else {
                    dedupe.release(&event_id);
                }
            }

            result
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
    let now = now_unix_timestamp_s().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read system clock for signature validation: {error}"),
        )
    })?;
    verify_feishu_signature_with_now(headers, raw_body, payload, encrypt_key, now)
}

fn verify_feishu_signature_with_now(
    headers: &HeaderMap,
    raw_body: &str,
    payload: &Value,
    encrypt_key: Option<&str>,
    now_timestamp_s: i64,
) -> Result<(), (StatusCode, String)> {
    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        return Ok(());
    }

    let Some(encrypt_key) = encrypt_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "unauthorized: feishu encrypt key is not configured".to_owned(),
        ));
    };

    let timestamp = read_header_required(headers, "X-Lark-Request-Timestamp")?;
    let nonce = read_header_required(headers, "X-Lark-Request-Nonce")?;
    let signature = read_header_required(headers, "X-Lark-Signature")?;
    validate_timestamp_freshness(timestamp, now_timestamp_s)?;

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

fn validate_timestamp_freshness(
    timestamp: &str,
    now_timestamp_s: i64,
) -> Result<(), (StatusCode, String)> {
    let parsed_timestamp = timestamp.parse::<i64>().map_err(|error| {
        (
            StatusCode::UNAUTHORIZED,
            format!("unauthorized: invalid feishu timestamp header: {error}"),
        )
    })?;
    let skew = now_timestamp_s.abs_diff(parsed_timestamp);
    if skew > FEISHU_SIGNATURE_MAX_SKEW_SECONDS as u64 {
        return Err((
            StatusCode::UNAUTHORIZED,
            format!(
                "unauthorized: feishu request timestamp outside allowed skew window (>{}s)",
                FEISHU_SIGNATURE_MAX_SKEW_SECONDS
            ),
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
    use std::path::PathBuf;
    use sha2::{Digest, Sha256};

    fn temp_replay_cache_path(test_name: &str) -> PathBuf {
        let unique = format!(
            "{test_name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(format!("loongclaw-feishu-replay-{unique}.json"))
    }

    #[test]
    fn recent_cache_deduplicates_and_rolls_window() {
        let mut cache = RecentIdCache::new(2);
        assert!(matches!(
            cache.begin_processing("a"),
            RecentIdReservation::Accepted
        ));
        cache.mark_completed("a");
        assert!(matches!(
            cache.begin_processing("a"),
            RecentIdReservation::CompletedDuplicate
        ));
        assert!(matches!(
            cache.begin_processing("b"),
            RecentIdReservation::Accepted
        ));
        cache.mark_completed("b");
        assert!(matches!(
            cache.begin_processing("c"),
            RecentIdReservation::Accepted
        ));
        cache.mark_completed("c");
        assert!(matches!(
            cache.begin_processing("a"),
            RecentIdReservation::Accepted
        ));
    }

    #[test]
    fn recent_cache_releases_failed_events_for_retry() {
        let mut cache = RecentIdCache::new(4);

        assert!(matches!(
            cache.begin_processing("evt-1"),
            RecentIdReservation::Accepted
        ));
        assert!(matches!(
            cache.begin_processing("evt-1"),
            RecentIdReservation::InProgressDuplicate
        ));

        cache.release("evt-1");

        assert!(matches!(
            cache.begin_processing("evt-1"),
            RecentIdReservation::Accepted
        ));
    }

    #[test]
    fn signature_verification_passes_with_valid_headers() {
        let body = r#"{"encrypt":"opaque"}"#;
        let encrypt_key = "test-encrypt-key";
        let timestamp = "1736480000";
        let now = 1_736_480_010_i64;
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
        let result =
            verify_feishu_signature_with_now(&headers, body, &payload, Some(encrypt_key), now);
        assert!(result.is_ok());
    }

    #[test]
    fn signature_verification_rejects_mismatch() {
        let now = 1_i64;
        let mut headers = HeaderMap::new();
        headers.insert("X-Lark-Request-Timestamp", "1".parse().expect("header"));
        headers.insert("X-Lark-Request-Nonce", "n".parse().expect("header"));
        headers.insert("X-Lark-Signature", "deadbeef".parse().expect("header"));

        let body = "{\"encrypt\":\"x\"}";
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let error = verify_feishu_signature_with_now(&headers, body, &payload, Some("key"), now)
            .expect_err("mismatch");
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn signature_verification_rejects_stale_timestamp() {
        let body = r#"{"encrypt":"opaque"}"#;
        let encrypt_key = "test-encrypt-key";
        let timestamp = "100";
        let now = 1_000_i64;
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
        let error = verify_feishu_signature_with_now(
            &headers,
            body,
            &payload,
            Some(encrypt_key),
            now,
        )
        .expect_err("stale timestamp should fail");
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
        assert!(error.1.contains("outside allowed skew window"));
    }

    #[test]
    fn signature_verification_requires_encrypt_key_for_event_payloads() {
        let headers = HeaderMap::new();
        let body = "{\"header\":{\"event_type\":\"im.message.receive_v1\"}}";
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let error = verify_feishu_signature(&headers, body, &payload, None)
            .expect_err("missing encrypt key should fail");
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
        assert!(error.1.contains("encrypt key is not configured"));
    }

    #[test]
    fn signature_verification_skips_url_verification_payload() {
        let headers = HeaderMap::new();
        let body = r#"{"type":"url_verification","token":"token","challenge":"ok"}"#;
        let payload = serde_json::from_str::<Value>(body).expect("payload");
        let result = verify_feishu_signature(&headers, body, &payload, Some("encrypt-key"));
        assert!(result.is_ok());
    }

    #[test]
    fn persisted_replay_cache_survives_restart_for_completed_events() {
        let path = temp_replay_cache_path("persisted-replay");
        let ttl = 3_600_i64;

        {
            let mut cache =
                RecentIdCache::new_persisted(16, ttl, path.clone()).expect("create cache");
            assert!(matches!(
                cache.begin_processing("evt-42"),
                RecentIdReservation::Accepted
            ));
            cache.mark_completed("evt-42");
        }

        {
            let mut cache =
                RecentIdCache::new_persisted(16, ttl, path.clone()).expect("reopen cache");
            assert!(matches!(
                cache.begin_processing("evt-42"),
                RecentIdReservation::CompletedDuplicate
            ));
        }

        let _ = std::fs::remove_file(path);
    }
}
