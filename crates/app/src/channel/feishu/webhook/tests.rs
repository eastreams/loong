use super::*;
use crate::channel::ChannelPlatform;
use crate::channel::runtime::state::start_channel_operation_runtime_tracker_for_test;
use crate::config::{LoongConfig, ProviderConfig};
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_test_kernel_context};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    response::IntoResponse,
    routing::{post, put},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const MOCK_PROVIDER_MARKDOWN_REPLY: &str = "## structured inbound ack\n\n- rendered";
const FEISHU_WEBHOOK_TEST_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockRequest {
    path: String,
    query: Option<String>,
    authorization: Option<String>,
    body: String,
}

#[derive(Clone, Default)]
struct MockServerState {
    requests: Arc<Mutex<Vec<MockRequest>>>,
}

fn temp_webhook_test_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "loong-feishu-webhook-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

fn run_feishu_webhook_test_on_large_stack<F, Fut>(thread_name: &str, operation: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let join_handle = std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(FEISHU_WEBHOOK_TEST_STACK_SIZE_BYTES)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build feishu webhook test runtime");
            runtime.block_on(operation());
        })
        .expect("spawn feishu webhook large-stack test thread");
    match join_handle.join() {
        Ok(()) => {}
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn spawn_mock_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let address = listener.local_addr().expect("mock server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve mock server");
    });
    (format!("http://{address}"), handle)
}

async fn record_request(State(state): State<MockServerState>, request: Request) -> String {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX)
        .await
        .expect("read mock request body");
    let body_text = String::from_utf8(body.to_vec()).expect("mock request body utf8");
    state.requests.lock().await.push(MockRequest {
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().map(ToOwned::to_owned),
        authorization: parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        body: body_text.clone(),
    });
    body_text
}

fn mock_provider_stream_enabled(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|payload| payload.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn mock_provider_stream_response_body(response_text: &str) -> String {
    format!(
        "data: {}\n\n\
data: {}\n\n\
data: [DONE]\n\n",
        json!({
            "choices": [{
                "delta": {
                    "content": response_text
                }
            }]
        }),
        json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }]
        }),
    )
}

fn mock_provider_success_response(
    request_body: &str,
    response_text: &str,
) -> axum::response::Response {
    if mock_provider_stream_enabled(request_body) {
        return (
            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
            mock_provider_stream_response_body(response_text),
        )
            .into_response();
    }

    Json(json!({
        "choices": [{
            "message": {
                "content": response_text
            }
        }]
    }))
    .into_response()
}

async fn wait_for_request_count(
    requests: &Arc<Mutex<Vec<MockRequest>>>,
    expected_len: usize,
) -> Vec<MockRequest> {
    for _ in 0..50 {
        let snapshot = requests.lock().await.clone();
        if snapshot.len() >= expected_len {
            return snapshot;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    requests.lock().await.clone()
}

async fn wait_for_request_match(
    requests: &Arc<Mutex<Vec<MockRequest>>>,
    predicate: impl Fn(&MockRequest) -> bool,
) -> Vec<MockRequest> {
    for _ in 0..50 {
        let snapshot = requests.lock().await.clone();
        if snapshot.iter().any(&predicate) {
            return snapshot;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    requests.lock().await.clone()
}

async fn spawn_mock_provider_server(
    requests: Arc<Mutex<Vec<MockRequest>>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = MockServerState { requests };
    let router = Router::new().route(
        "/v1/chat/completions",
        post({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    let request_body = record_request(State(state), request).await;
                    mock_provider_success_response(
                        request_body.as_str(),
                        MOCK_PROVIDER_MARKDOWN_REPLY,
                    )
                }
            }
        }),
    );
    spawn_mock_server(router).await
}

async fn spawn_mock_provider_callback_toast_server(
    requests: Arc<Mutex<Vec<MockRequest>>>,
    response_text: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = MockServerState { requests };
    let router = Router::new().route(
        "/v1/chat/completions",
        post({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    let request_body = record_request(State(state), request).await;
                    mock_provider_success_response(request_body.as_str(), response_text)
                }
            }
        }),
    );
    spawn_mock_server(router).await
}

async fn spawn_mock_provider_failure_server(
    requests: Arc<Mutex<Vec<MockRequest>>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = MockServerState { requests };
    let router = Router::new().route(
        "/v1/chat/completions",
        post({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "error": {
                                "message": "provider offline"
                            }
                        })),
                    )
                }
            }
        }),
    );
    spawn_mock_server(router).await
}

async fn spawn_mock_provider_delayed_success_server(
    requests: Arc<Mutex<Vec<MockRequest>>>,
    delay: std::time::Duration,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = MockServerState { requests };
    let router = Router::new().route(
        "/v1/chat/completions",
        post({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    let request_body = record_request(State(state), request).await;
                    tokio::time::sleep(delay).await;
                    mock_provider_success_response(
                        request_body.as_str(),
                        MOCK_PROVIDER_MARKDOWN_REPLY,
                    )
                }
            }
        }),
    );
    spawn_mock_server(router).await
}

async fn spawn_mock_feishu_api_server(
    requests: Arc<Mutex<Vec<MockRequest>>>,
    reply_message_id: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = MockServerState { requests };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-webhook"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/interactive/v1/card/update",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "msg": "ok"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/{message_id}/reply",
            post({
                let state = state.clone();
                move |axum::extract::Path(message_id): axum::extract::Path<String>, request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": reply_message_id,
                                "root_id": message_id
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/{message_id}/reactions",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "reaction_id": "reaction_webhook_1"
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/{message_id}",
            put({
                let state = state.clone();
                move |axum::extract::Path(message_id): axum::extract::Path<String>, request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": message_id
                            }
                        }))
                    }
                }
            }),
        );
    spawn_mock_server(router).await
}

async fn spawn_mock_feishu_api_server_with_failing_reactions(
    requests: Arc<Mutex<Vec<MockRequest>>>,
    reply_message_id: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let state = MockServerState { requests };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-webhook"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/interactive/v1/card/update",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "msg": "ok"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/{message_id}/reply",
            post({
                let state = state.clone();
                move |axum::extract::Path(message_id): axum::extract::Path<String>, request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": reply_message_id,
                                "root_id": message_id
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/{message_id}/reactions",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 99991663,
                            "msg": "reaction failed"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/{message_id}",
            put({
                let state = state.clone();
                move |axum::extract::Path(message_id): axum::extract::Path<String>, request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": message_id
                            }
                        }))
                    }
                }
            }),
        );
    spawn_mock_server(router).await
}

fn test_webhook_config(provider_base_url: &str, feishu_base_url: &str) -> LoongConfig {
    let temp_dir = temp_webhook_test_dir("runtime");
    std::fs::create_dir_all(&temp_dir).expect("create webhook temp dir");

    let mut config = LoongConfig {
        provider: ProviderConfig {
            base_url: provider_base_url.to_owned(),
            api_key: Some(loong_contracts::SecretRef::Inline(
                "test-provider-key".to_owned(),
            )),
            model: "test-model".to_owned(),
            ..ProviderConfig::default()
        },
        ..LoongConfig::default()
    };
    config.memory.sqlite_path = temp_dir.join("memory.sqlite3").display().to_string();
    config.tools.file_root = Some(temp_dir.join("tool-root").display().to_string());
    config.feishu.enabled = true;
    config.feishu.account_id = Some("feishu_main".to_owned());
    config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned()));
    config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("secret-123".to_owned()));
    config.feishu.base_url = Some(feishu_base_url.to_owned());
    config.feishu.receive_id_type = "chat_id".to_owned();
    config.feishu.allowed_chat_ids = vec!["oc_demo".to_owned()];
    config.feishu.verification_token = Some(loong_contracts::SecretRef::Inline(
        "verify-token".to_owned(),
    ));
    config.feishu.encrypt_key = Some(loong_contracts::SecretRef::Inline("encrypt-key".to_owned()));
    config
}

fn signed_headers(body: &str, encrypt_key: &str) -> HeaderMap {
    let timestamp = "1736480000";
    let nonce = "nonce-1";
    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(encrypt_key.as_bytes());
    hasher.update(body.as_bytes());
    let signature = hex::encode(hasher.finalize());

    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Lark-Request-Timestamp",
        timestamp.parse().expect("timestamp header"),
    );
    headers.insert("X-Lark-Request-Nonce", nonce.parse().expect("nonce header"));
    headers.insert(
        "X-Lark-Signature",
        signature.parse().expect("signature header"),
    );
    headers
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
    let nonce = "nonce-1";

    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(encrypt_key.as_bytes());
    hasher.update(body.as_bytes());
    let signature = hex::encode(hasher.finalize());

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
fn feishu_webhook_file_event_reaches_provider_as_structured_text_and_replies() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-file-event", || async move {
        feishu_webhook_file_event_reaches_provider_as_structured_text_and_replies_impl().await;
    });
}

async fn feishu_webhook_file_event_reaches_provider_as_structured_text_and_replies_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) =
        spawn_mock_provider_server(provider_requests.clone()).await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_1").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    adapter
        .refresh_tenant_token()
        .await
        .expect("refresh tenant token before webhook test");
    let kernel_ctx = bootstrap_test_kernel_context("feishu-webhook-test", DEFAULT_TOKEN_TTL_S)
        .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "token": "verify-token",
        "header": {
            "event_id": "evt_file_end_to_end",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_1"
                }
            },
            "message": {
                "chat_id": "oc_demo",
                "message_id": "om_inbound_file_1",
                "message_type": "file",
                "content": "{\"file_key\":\"file_v2_demo\",\"file_name\":\"report.pdf\"}"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("webhook should succeed");

    assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

    let provider_requests = provider_requests.lock().await.clone();
    assert_eq!(provider_requests.len(), 1);
    assert_eq!(provider_requests[0].path, "/v1/chat/completions");
    let provider_body =
        serde_json::from_str::<Value>(&provider_requests[0].body).expect("provider body json");
    assert_eq!(provider_body["stream"], json!(true));
    let provider_user_content = provider_body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("provider user content");
    assert!(
        provider_user_content.contains("[feishu_inbound_message]"),
        "provider should receive the structured feishu marker"
    );
    assert!(
        provider_user_content.contains("\"message_type\":\"file\""),
        "provider should receive the structured file message type"
    );
    assert!(
        provider_user_content.contains("\"file_key\":\"file_v2_demo\""),
        "provider should receive the feishu file key"
    );
    assert!(
        provider_user_content.contains("Binary file content is not fetched automatically."),
        "provider should receive the binary fetch note"
    );

    let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
    assert_eq!(feishu_requests.len(), 3);
    let reaction_request = feishu_requests
        .iter()
        .find(|request| request.path == "/open-apis/im/v1/messages/om_inbound_file_1/reactions")
        .expect("webhook flow should add ack reaction");
    assert_eq!(
        reaction_request.authorization.as_deref(),
        Some("Bearer t-token-webhook")
    );
    assert!(
        reaction_request.body.contains("\"emoji_type\""),
        "reaction request should include a Feishu emoji type"
    );
    let reply_request = feishu_requests
        .iter()
        .find(|request| request.path == "/open-apis/im/v1/messages/om_inbound_file_1/reply")
        .expect("webhook flow should still send a reply");
    assert!(
        reply_request.body.contains("\"msg_type\":\"interactive\""),
        "webhook reply should send markdown-capable interactive cards"
    );
    assert!(
        reply_request.body.contains("\\\"tag\\\":\\\"markdown\\\""),
        "reply body should wrap the provider reply in a markdown card"
    );
    assert!(
        reply_request
            .body
            .contains("\\\"content\\\":\\\"## structured inbound ack\\\\n\\\\n- rendered\\\""),
        "reply body should preserve provider markdown content"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_skips_ack_reaction_when_disabled() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-no-ack", || async move {
        feishu_webhook_skips_ack_reaction_when_disabled_impl().await;
    });
}

async fn feishu_webhook_skips_ack_reaction_when_disabled_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) =
        spawn_mock_provider_server(provider_requests.clone()).await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_disabled_1").await;

    let mut config = test_webhook_config(&provider_base_url, &feishu_base_url);
    config.feishu.ack_reactions = false;
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    adapter
        .refresh_tenant_token()
        .await
        .expect("refresh tenant token before webhook test");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-no-ack-test", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "token": "verify-token",
        "header": {
            "event_id": "evt_ack_disabled",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_disabled"
                }
            },
            "message": {
                "chat_id": "oc_demo",
                "message_id": "om_inbound_no_ack_1",
                "message_type": "text",
                "content": "{\"text\":\"hello without ack\"}"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("webhook should succeed");

    assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

    let feishu_requests = wait_for_request_count(&feishu_requests, 1).await;
    assert!(
        feishu_requests.iter().all(
            |request| request.path != "/open-apis/im/v1/messages/om_inbound_no_ack_1/reactions"
        ),
        "disabled ack_reactions should skip the reaction API call"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_provider_failure_retry_does_not_duplicate_ack_reaction() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-ack-retry", || async move {
        feishu_webhook_provider_failure_retry_does_not_duplicate_ack_reaction_impl().await;
    });
}

async fn feishu_webhook_provider_failure_retry_does_not_duplicate_ack_reaction_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) =
        spawn_mock_provider_failure_server(provider_requests.clone()).await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    adapter
        .refresh_tenant_token()
        .await
        .expect("refresh tenant token before webhook test");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-provider-failure", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "token": "verify-token",
        "header": {
            "event_id": "evt_provider_failure_no_ack",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_provider_failure"
                }
            },
            "message": {
                "chat_id": "oc_demo",
                "message_id": "om_inbound_failure_no_ack_1",
                "message_type": "text",
                "content": "{\"text\":\"provider failure should not ack\"}"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state.clone(),
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect(
        "webhook should return a safe success body while the provider failure is handled inline",
    );
    assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

    let response_retry = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("webhook retry should still return a safe success body");
    assert_eq!(
        response_retry.body(),
        &json!({"code": 0, "msg": "duplicate_event"})
    );

    let feishu_requests = wait_for_request_count(&feishu_requests, 1).await;
    assert_eq!(
        feishu_requests
            .iter()
            .filter(|request| request.path
                == "/open-apis/im/v1/messages/om_inbound_failure_no_ack_1/reactions")
            .count(),
        1,
        "retrying an inline-failed inbound turn must not duplicate ack reactions"
    );
    assert!(
        feishu_requests
            .iter()
            .filter(|request| request.path
                == "/open-apis/im/v1/messages/om_inbound_failure_no_ack_1/reply")
            .count()
            <= 1,
        "inline provider failure handling should send at most one user-facing reply across retries"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_reaction_failure_stays_best_effort_after_reply() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-ack-best-effort", || async move {
        feishu_webhook_reaction_failure_stays_best_effort_after_reply_impl().await;
    });
}

async fn feishu_webhook_reaction_failure_stays_best_effort_after_reply_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) =
        spawn_mock_provider_server(provider_requests.clone()).await;
    let (feishu_base_url, feishu_server) = spawn_mock_feishu_api_server_with_failing_reactions(
        feishu_requests.clone(),
        "om_reply_reaction_failure_1",
    )
    .await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    adapter
        .refresh_tenant_token()
        .await
        .expect("refresh tenant token before webhook test");
    let kernel_ctx = bootstrap_test_kernel_context(
        "feishu-webhook-reaction-failure-best-effort",
        DEFAULT_TOKEN_TTL_S,
    )
    .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "token": "verify-token",
        "header": {
            "event_id": "evt_reaction_failure_best_effort",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_reaction_failure"
                }
            },
            "message": {
                "chat_id": "oc_demo",
                "message_id": "om_inbound_reaction_failure_1",
                "message_type": "text",
                "content": "{\"text\":\"reaction failure should not fail webhook\"}"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("reaction failure should stay best-effort");

    assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

    let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
    assert_eq!(feishu_requests.len(), 3);
    assert!(
        feishu_requests.iter().any(|request| request.path
            == "/open-apis/im/v1/messages/om_inbound_reaction_failure_1/reply"),
        "reply should still be sent even when reaction fails"
    );
    assert!(
        feishu_requests.iter().any(|request| request.path
            == "/open-apis/im/v1/messages/om_inbound_reaction_failure_1/reactions"),
        "reaction attempt should still be issued"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_provider_timeout_acknowledges_after_retry_budget_exhaustion() {
    run_feishu_webhook_test_on_large_stack(
        "feishu-webhook-provider-timeout-terminal",
        || async move {
            feishu_webhook_provider_timeout_acknowledges_after_retry_budget_exhaustion_impl().await;
        },
    );
}

async fn feishu_webhook_provider_timeout_acknowledges_after_retry_budget_exhaustion_impl() {
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let mut config = test_webhook_config("http://127.0.0.1:9", &feishu_base_url);
    config.provider.request_timeout_ms = 50;
    config.provider.retry_max_attempts = 2;
    config.provider.retry_initial_backoff_ms = 50;
    config.provider.retry_max_backoff_ms = 50;
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    adapter
        .refresh_tenant_token()
        .await
        .expect("refresh tenant token before webhook test");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-provider-timeout", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "token": "verify-token",
        "header": {
            "event_id": "evt_provider_timeout_terminal",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_provider_timeout"
                }
            },
            "message": {
                "chat_id": "oc_demo",
                "message_id": "om_inbound_timeout_terminal_1",
                "message_type": "text",
                "content": "{\"text\":\"provider timeout should stop feishu redelivery\"}"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state.clone(),
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("webhook timeout should reply inline after provider retries");

    assert_eq!(response.body()["code"], json!(0));
    assert_eq!(response.body()["msg"], json!("ok"));

    let response_retry = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("completed timeout event should stay acknowledged on duplicate delivery");
    assert_eq!(
        response_retry.body(),
        &json!({"code": 0, "msg": "duplicate_event"})
    );

    let feishu_requests = wait_for_request_match(&feishu_requests, |request| {
        (request.path == "/open-apis/im/v1/messages/om_reply_unused"
            || request.path == "/open-apis/im/v1/messages/om_inbound_timeout_terminal_1/reply")
            && request
                .body
                .contains("Sorry, I couldn't finish this request")
            && !request.body.contains("[provider_error]")
    })
    .await;
    assert_eq!(
        feishu_requests
            .iter()
            .filter(|request| request.path
                == "/open-apis/im/v1/messages/om_inbound_timeout_terminal_1/reactions")
            .count(),
        1,
        "inline timeout reply must not duplicate ack reactions"
    );
    assert!(
        feishu_requests.iter().any(|request| {
            (request.path == "/open-apis/im/v1/messages/om_reply_unused"
                || request.path == "/open-apis/im/v1/messages/om_inbound_timeout_terminal_1/reply")
                && request
                    .body
                    .contains("Sorry, I couldn't finish this request")
                && !request.body.contains("[provider_error]")
        }),
        "final failure should deliver a user-facing error back to the Feishu conversation"
    );

    feishu_server.abort();
}

#[test]
fn feishu_retry_status_handle_creates_and_updates_single_status_message() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-retry-status-handle", || async move {
        feishu_retry_status_handle_creates_and_updates_single_status_message_impl().await;
    });
}

async fn feishu_retry_status_handle_creates_and_updates_single_status_message_impl() {
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_retry_status").await;

    let config = test_webhook_config("http://127.0.0.1:9", &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    adapter
        .refresh_tenant_token()
        .await
        .expect("refresh tenant token before retry status test");
    let retry_target = ChannelOutboundTarget::feishu_message_reply("om_source_retry_status")
        .with_feishu_reply_chat_id("oc_demo")
        .with_feishu_reply_in_thread(true);
    let handle = FeishuRetryStatusHandle::new(Arc::new(Mutex::new(adapter)), retry_target);

    let callback = handle.callback().expect("retry callback should exist");
    callback(crate::provider::ProviderRetryProgress {
        model: "glm-5".to_owned(),
        next_attempt: 2,
        max_attempts: 3,
        delay_ms: 1_000,
        status_code: None,
        timeout: true,
        connect: false,
    });

    let handled = handle
            .finalize_failure(
                "Sorry, I couldn't finish this request because the model timed out before a full reply was produced. Please try again in a moment."
                    .to_owned(),
            )
            .await;
    assert!(
        handled,
        "final failure should update the existing status message"
    );

    let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
    assert!(
        feishu_requests.iter().any(|request| {
            request.path == "/open-apis/im/v1/messages/om_source_retry_status/reply"
                && request.body.contains("Retrying attempt 2/3 in 1s")
        }),
        "retry progress should create a dedicated Feishu status reply"
    );
    assert!(
            feishu_requests.iter().any(|request| {
                request.path == "/open-apis/im/v1/messages/om_retry_status"
                    && request.body.contains(
                        "Sorry, I couldn't finish this request because the model timed out before a full reply was produced. Please try again in a moment.",
                    )
            }),
            "final failure should update the single status message instead of sending a second reply"
        );

    feishu_server.abort();
}

#[test]
fn feishu_webhook_inbound_reply_stays_successful_when_runtime_end_write_fails() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-runtime-end", || async move {
        feishu_webhook_inbound_reply_stays_successful_when_runtime_end_write_fails_impl().await;
    });
}

async fn feishu_webhook_inbound_reply_stays_successful_when_runtime_end_write_fails_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_delayed_success_server(
        provider_requests.clone(),
        std::time::Duration::from_millis(50),
    )
    .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_runtime_end").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    adapter
        .refresh_tenant_token()
        .await
        .expect("refresh tenant token before webhook test");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-runtime-end-failure", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime_dir = temp_webhook_test_dir("runtime-end-failure");
    std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
    let runtime = Arc::new(
        start_channel_operation_runtime_tracker_for_test(
            &runtime_dir,
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
            424242,
        )
        .await
        .expect("start test runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let runtime_dir_for_delete = runtime_dir.clone();
    let runtime_delete = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        std::fs::remove_dir_all(&runtime_dir_for_delete).expect("remove runtime dir");
        std::fs::write(&runtime_dir_for_delete, "blocked").expect("replace runtime dir with file");
    });

    let payload = json!({
        "token": "verify-token",
        "header": {
            "event_id": "evt_runtime_end_failure",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_runtime_end"
                }
            },
            "message": {
                "chat_id": "oc_demo",
                "message_id": "om_runtime_end_1",
                "message_type": "text",
                "content": "{\"text\":\"runtime end failure should stay acknowledged\"}"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("reply should stay successful even if runtime end bookkeeping fails");

    runtime_delete.await.expect("join runtime file deletion");

    assert_eq!(response.body(), &json!({"code": 0, "msg": "ok"}));

    let provider_requests = provider_requests.lock().await.clone();
    assert_eq!(provider_requests.len(), 1);
    let provider_body =
        serde_json::from_str::<Value>(&provider_requests[0].body).expect("provider body json");
    assert_eq!(provider_body["stream"], json!(true));

    let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
    assert_eq!(feishu_requests.len(), 3);
    assert!(
        feishu_requests
            .iter()
            .any(|request| request.path == "/open-apis/im/v1/messages/om_runtime_end_1/reply"),
        "reply should still be sent when runtime end bookkeeping fails"
    );
    assert!(
        feishu_requests
            .iter()
            .any(|request| request.path == "/open-apis/im/v1/messages/om_runtime_end_1/reactions"),
        "ack reaction should still be attempted when runtime end bookkeeping fails"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_reaches_provider_and_returns_safe_noop_body() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-noop", || async move {
        feishu_webhook_card_callback_reaches_provider_and_returns_safe_noop_body_impl().await;
    });
}

async fn feishu_webhook_card_callback_reaches_provider_and_returns_safe_noop_body_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) =
        spawn_mock_provider_server(provider_requests.clone()).await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-card-callback", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1",
                    "user_id": "u_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request",
                "value": {
                    "ticket_id": "T-500"
                }
            },
            "context": {
                "open_message_id": "om_card_source_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(response.body(), &json!({}));

    let provider_requests = provider_requests.lock().await.clone();
    assert_eq!(provider_requests.len(), 1);
    let provider_body =
        serde_json::from_str::<Value>(&provider_requests[0].body).expect("provider body json");
    let provider_user_content = provider_body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("provider user content");
    assert!(provider_user_content.contains("[feishu_card_callback]"));
    assert!(provider_user_content.contains("\"name\":\"approve_request\""));
    assert!(
        !provider_requests[0].body.contains("callback-token-1"),
        "callback token must stay out of provider-visible prompt state"
    );

    let feishu_requests = feishu_requests.lock().await.clone();
    assert_eq!(
        feishu_requests.len(),
        0,
        "callback flow should not send a normal Feishu reply"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_structured_toast_response_is_returned() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-toast", || async move {
        feishu_webhook_card_callback_structured_toast_response_is_returned_impl().await;
    });
}

async fn feishu_webhook_card_callback_structured_toast_response_is_returned_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"toast\",\"kind\":\"success\",\"content\":\"Approved\"}",
        )
        .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-card-callback-toast", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_toast_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-toast-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1",
                    "user_id": "u_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_toast_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(
        response.body(),
        &json!({
            "toast": {
                "type": "success",
                "content": "Approved"
            }
        })
    );
    assert_eq!(provider_requests.lock().await.len(), 1);
    assert_eq!(
        feishu_requests.lock().await.len(),
        0,
        "toast callback flow should not send a normal Feishu reply"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_structured_card_response_is_returned() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-card", || async move {
        feishu_webhook_card_callback_structured_card_response_is_returned_impl().await;
    });
}

async fn feishu_webhook_card_callback_structured_card_response_is_returned_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"card\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"Approved inline\"}]}}",
        )
        .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-card-callback-card", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_card_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-card-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1",
                    "user_id": "u_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_card_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(
        response.body(),
        &json!({
            "card": {
                "elements": [
                    {
                        "tag": "markdown",
                        "content": "Approved inline"
                    }
                ]
            }
        })
    );
    assert_eq!(provider_requests.lock().await.len(), 1);
    assert_eq!(
        feishu_requests.lock().await.len(),
        0,
        "card callback response should not send a normal Feishu reply"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_structured_card_markdown_response_is_returned() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-card-md", || async move {
        feishu_webhook_card_callback_structured_card_markdown_response_is_returned_impl().await;
    });
}

async fn feishu_webhook_card_callback_structured_card_markdown_response_is_returned_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
        provider_requests.clone(),
        "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"Approved inline\"}",
    )
    .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx = bootstrap_test_kernel_context(
        "feishu-webhook-card-callback-card-markdown",
        DEFAULT_TOKEN_TTL_S,
    )
    .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_card_markdown_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-card-markdown-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1",
                    "user_id": "u_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_card_markdown_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(
        response.body(),
        &json!({
            "card": {
                "schema": "2.0",
                "config": {
                    "wide_screen_mode": true
                },
                "body": {
                    "elements": [
                        {
                            "tag": "markdown",
                            "content": "Approved inline"
                        }
                    ]
                }
            }
        })
    );
    assert_eq!(provider_requests.lock().await.len(), 1);
    assert_eq!(
        feishu_requests.lock().await.len(),
        0,
        "card callback response should not send a normal Feishu reply"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_structured_card_response_with_toast_is_returned() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-card-toast", || async move {
        feishu_webhook_card_callback_structured_card_response_with_toast_is_returned_impl().await;
    });
}

async fn feishu_webhook_card_callback_structured_card_response_with_toast_is_returned_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"toast\":{\"kind\":\"success\",\"content\":\"Approved\"},\"card\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"Approved inline\"}]}}",
        )
        .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx = bootstrap_test_kernel_context(
        "feishu-webhook-card-callback-card-with-toast",
        DEFAULT_TOKEN_TTL_S,
    )
    .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_card_toast_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-card-toast-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1",
                    "user_id": "u_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_card_toast_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(
        response.body(),
        &json!({
            "toast": {
                "type": "success",
                "content": "Approved"
            },
            "card": {
                "elements": [
                    {
                        "tag": "markdown",
                        "content": "Approved inline"
                    }
                ]
            }
        })
    );
    assert_eq!(provider_requests.lock().await.len(), 1);
    assert_eq!(
        feishu_requests.lock().await.len(),
        0,
        "card callback response should not send a normal Feishu reply"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_structured_card_markdown_response_with_toast_is_returned() {
    run_feishu_webhook_test_on_large_stack(
        "feishu-webhook-callback-card-md-toast",
        || async move {
            feishu_webhook_card_callback_structured_card_markdown_response_with_toast_is_returned_impl().await;
        },
    );
}

async fn feishu_webhook_card_callback_structured_card_markdown_response_with_toast_is_returned_impl()
 {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"Approved inline\",\"toast\":{\"kind\":\"success\",\"content\":\"Approved\"}}",
        )
        .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx = bootstrap_test_kernel_context(
        "feishu-webhook-card-callback-card-markdown-with-toast",
        DEFAULT_TOKEN_TTL_S,
    )
    .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_card_markdown_toast_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-card-markdown-toast-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1",
                    "user_id": "u_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_card_markdown_toast_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(
        response.body(),
        &json!({
            "toast": {
                "type": "success",
                "content": "Approved"
            },
            "card": {
                "schema": "2.0",
                "config": {
                    "wide_screen_mode": true
                },
                "body": {
                    "elements": [
                        {
                            "tag": "markdown",
                            "content": "Approved inline"
                        }
                    ]
                }
            }
        })
    );
    assert_eq!(provider_requests.lock().await.len(), 1);
    assert_eq!(
        feishu_requests.lock().await.len(),
        0,
        "card callback response should not send a normal Feishu reply"
    );

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn parse_feishu_structured_callback_response_rejects_card_markdown_conflict() {
    let response = parse_feishu_structured_callback_response(
        "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"Approved inline\",\"card\":{\"elements\":[{\"tag\":\"markdown\",\"content\":\"raw\"}]}}",
    );

    assert!(response.is_none());
}

#[test]
fn parse_feishu_structured_callback_response_rejects_empty_card_markdown() {
    let response = parse_feishu_structured_callback_response(
        "[feishu_callback_response]\n{\"mode\":\"card\",\"markdown\":\"   \"}",
    );

    assert!(response.is_none());
}

#[test]
fn feishu_webhook_card_callback_invalid_structured_response_falls_back_to_safe_noop_body() {
    run_feishu_webhook_test_on_large_stack(
        "feishu-webhook-callback-invalid-toast",
        || async move {
            feishu_webhook_card_callback_invalid_structured_response_falls_back_to_safe_noop_body_impl().await;
        },
    );
}

async fn feishu_webhook_card_callback_invalid_structured_response_falls_back_to_safe_noop_body_impl()
 {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
        provider_requests.clone(),
        "[feishu_callback_response]\n{\"mode\":\"toast\",\"kind\":\"danger\",\"content\":\"nope\"}",
    )
    .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx = bootstrap_test_kernel_context(
        "feishu-webhook-card-callback-invalid-toast",
        DEFAULT_TOKEN_TTL_S,
    )
    .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_invalid_toast_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-invalid-toast-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_invalid_toast_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(response.body(), &json!({}));
    assert_eq!(provider_requests.lock().await.len(), 1);
    assert_eq!(feishu_requests.lock().await.len(), 0);

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_invalid_structured_card_response_falls_back_to_safe_noop_body() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-invalid-card", || async move {
        feishu_webhook_card_callback_invalid_structured_card_response_falls_back_to_safe_noop_body_impl().await;
    });
}

async fn feishu_webhook_card_callback_invalid_structured_card_response_falls_back_to_safe_noop_body_impl()
 {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) = spawn_mock_provider_callback_toast_server(
            provider_requests.clone(),
            "[feishu_callback_response]\n{\"mode\":\"card\",\"toast\":{\"kind\":\"danger\",\"content\":\"nope\"},\"card\":true}",
        )
        .await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx = bootstrap_test_kernel_context(
        "feishu-webhook-card-callback-invalid-card",
        DEFAULT_TOKEN_TTL_S,
    )
    .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_invalid_card_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-invalid-card-1",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_invalid_card_1",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");
    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback webhook should succeed");

    assert_eq!(response.body(), &json!({}));
    assert_eq!(provider_requests.lock().await.len(), 1);
    assert_eq!(feishu_requests.lock().await.len(), 0);

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_duplicate_is_deduped_safely() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-callback-dedupe", || async move {
        feishu_webhook_card_callback_duplicate_is_deduped_safely_impl().await;
    });
}

async fn feishu_webhook_card_callback_duplicate_is_deduped_safely_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) =
        spawn_mock_provider_server(provider_requests.clone()).await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-card-callback-dedupe", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_dedupe_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-dedupe",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_dedupe",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");

    let first = handle_feishu_webhook_payload(
        state.clone(),
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("first callback should succeed");
    let second = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("second callback should succeed");

    assert_eq!(first.body(), &json!({}));
    assert_eq!(second.body(), &json!({}));
    assert!(
        !provider_requests.lock().await.is_empty(),
        "callback failure path should still attempt provider processing"
    );
    assert_eq!(feishu_requests.lock().await.len(), 0);

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn feishu_webhook_card_callback_provider_failure_still_returns_safe_noop_body() {
    run_feishu_webhook_test_on_large_stack(
        "feishu-webhook-callback-provider-failure",
        || async move {
            feishu_webhook_card_callback_provider_failure_still_returns_safe_noop_body_impl().await;
        },
    );
}

async fn feishu_webhook_card_callback_provider_failure_still_returns_safe_noop_body_impl() {
    let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let (provider_base_url, provider_server) =
        spawn_mock_provider_failure_server(provider_requests.clone()).await;
    let (feishu_base_url, feishu_server) =
        spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_unused").await;

    let config = test_webhook_config(&provider_base_url, &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");
    let adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
    let kernel_ctx =
        bootstrap_test_kernel_context("feishu-webhook-card-callback-failure", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap kernel context");
    let runtime = Arc::new(
        ChannelOperationRuntimeTracker::start(
            ChannelPlatform::Feishu,
            "serve",
            resolved.account.id.as_str(),
            resolved.account.label.as_str(),
        )
        .await
        .expect("start runtime tracker"),
    );
    let state = FeishuWebhookState::new(config, &resolved, adapter, kernel_ctx, runtime);

    let payload = json!({
        "header": {
            "event_id": "evt_card_webhook_failure_1",
            "event_type": "card.action.trigger",
            "token": "verify-token"
        },
        "event": {
            "token": "callback-token-failure",
            "operator": {
                "operator_id": {
                    "open_id": "ou_sender_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_card_source_failure",
                "open_chat_id": "oc_demo"
            }
        }
    });
    let raw_body = serde_json::to_string(&payload).expect("serialize payload");
    let headers = signed_headers(&raw_body, "encrypt-key");

    let response = handle_feishu_webhook_payload(
        state,
        &headers,
        raw_body.as_str(),
        serde_json::from_str(raw_body.as_str()).expect("payload value"),
    )
    .await
    .expect("callback failure should still produce a safe Feishu body");

    assert_eq!(response.body(), &json!({}));
    assert!(
        !provider_requests.lock().await.is_empty(),
        "callback failure path should still attempt provider processing"
    );
    assert_eq!(feishu_requests.lock().await.len(), 0);

    provider_server.abort();
    feishu_server.abort();
}

#[test]
fn execute_deferred_feishu_card_update_uses_delayed_update_api() {
    run_feishu_webhook_test_on_large_stack("feishu-webhook-deferred-update", || async move {
        execute_deferred_feishu_card_update_uses_delayed_update_api_impl().await;
    });
}

async fn execute_deferred_feishu_card_update_uses_delayed_update_api_impl() {
    let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: feishu_requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-deferred"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/interactive/v1/card/update",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "msg": "ok"
                        }))
                    }
                }
            }),
        );
    let (feishu_base_url, feishu_server) = spawn_mock_server(router).await;

    let config = test_webhook_config("http://127.0.0.1:9", &feishu_base_url);
    let resolved = config
        .feishu
        .resolve_account(None)
        .expect("resolve feishu account");

    execute_deferred_feishu_card_update(
        config,
        crate::tools::DeferredFeishuCardUpdate {
            configured_account_id: resolved.configured_account_id,
            token: "callback-token-deferred".to_owned(),
            card: json!({
                "elements": [{
                    "tag": "markdown",
                    "content": "deferred update"
                }]
            }),
            open_ids: vec!["ou_operator_1".to_owned()],
        },
    )
    .await
    .expect("deferred callback update should succeed");

    let feishu_requests = feishu_requests.lock().await.clone();
    assert_eq!(feishu_requests.len(), 2);
    assert_eq!(
        feishu_requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(
        feishu_requests[1].path,
        "/open-apis/interactive/v1/card/update"
    );
    assert_eq!(
        feishu_requests[1].authorization.as_deref(),
        Some("Bearer t-token-deferred")
    );
    assert!(
        feishu_requests[1]
            .body
            .contains("\"token\":\"callback-token-deferred\"")
    );
    assert!(
        feishu_requests[1]
            .body
            .contains("\"open_ids\":[\"ou_operator_1\"]")
    );
    assert!(
        feishu_requests[1]
            .body
            .contains("\"content\":\"deferred update\"")
    );

    feishu_server.abort();
}
