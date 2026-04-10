use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde::Serialize;
use serde_json::Value;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::CliResult;
use crate::KernelContext;
use crate::channel::feishu::api::{FeishuClient, FeishuWsEndpointClientConfig};
use crate::channel::{ChannelServeStopHandle, runtime::state::ChannelOperationRuntimeTracker};
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedFeishuChannelConfig,
};

use super::adapter::FeishuAdapter;
use super::webhook::{FeishuParsedActionResponse, FeishuWebhookState, handle_feishu_parsed_action};

const HEADER_BIZ_RT: &str = "biz_rt";
const HEADER_MESSAGE_ID: &str = "message_id";
const HEADER_SEQ: &str = "seq";
const HEADER_SUM: &str = "sum";
const HEADER_TYPE: &str = "type";
const MESSAGE_TYPE_PING: &str = "ping";
const MESSAGE_TYPE_PONG: &str = "pong";
const FRAME_TYPE_CONTROL: i32 = 0;
const FRAME_TYPE_DATA: i32 = 1;
const DEFAULT_WS_RECONNECT_INTERVAL_S: u64 = 120;
const DEFAULT_WS_PING_INTERVAL_S: u64 = 120;

fn ensure_feishu_websocket_rustls_provider() {
    static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();

    RUSTLS_PROVIDER_INIT.get_or_init(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
}

fn redact_feishu_websocket_log_url(raw_url: &str) -> String {
    let trimmed_url = raw_url.trim();
    let parsed_url = reqwest::Url::parse(trimmed_url);

    let Ok(mut parsed_url) = parsed_url else {
        let without_query = trimmed_url.split('?').next().unwrap_or(trimmed_url);
        let without_fragment = without_query.split('#').next().unwrap_or(without_query);
        return without_fragment.to_owned();
    };

    let _ = parsed_url.set_username("");
    let _ = parsed_url.set_password(None);
    parsed_url.set_query(None);
    parsed_url.set_fragment(None);
    parsed_url.to_string()
}

#[derive(Clone, PartialEq, prost::Message)]
struct FeishuWsHeader {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(string, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct FeishuWsFrame {
    #[prost(uint64, tag = "1")]
    seq_id: u64,
    #[prost(uint64, tag = "2")]
    log_id: u64,
    #[prost(int32, tag = "3")]
    service: i32,
    #[prost(int32, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<FeishuWsHeader>,
    #[prost(string, tag = "6")]
    payload_encoding: String,
    #[prost(string, tag = "7")]
    payload_type: String,
    #[prost(bytes, tag = "8")]
    payload: Vec<u8>,
    #[prost(string, tag = "9")]
    log_id_new: String,
}

#[derive(Debug, Default)]
struct FeishuWsFragments {
    messages: BTreeMap<String, FeishuWsFragmentSet>,
}

#[derive(Debug)]
struct FeishuWsFragmentSet {
    created_at: Instant,
    total: usize,
    parts: Vec<Option<Vec<u8>>>,
}

#[derive(Serialize)]
struct FeishuWsResponseEnvelope {
    code: u16,
    headers: BTreeMap<String, String>,
    data: Option<String>,
}

impl FeishuWsFrame {
    fn header_value(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find_map(|header| (header.key == key).then_some(header.value.as_str()))
    }

    fn header_value_usize(&self, key: &str) -> Option<usize> {
        self.header_value(key)?.parse::<usize>().ok()
    }

    fn set_header(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some(header) = self.headers.iter_mut().find(|header| header.key == key) {
            header.value = value;
            return;
        }
        self.headers.push(FeishuWsHeader {
            key: key.to_owned(),
            value,
        });
    }
}

impl FeishuWsFragments {
    fn combine(
        &mut self,
        message_id: &str,
        total: usize,
        seq: usize,
        payload: Vec<u8>,
    ) -> Option<Vec<u8>> {
        self.retain_recent();
        if total <= 1 {
            return Some(payload);
        }
        if message_id.trim().is_empty() || seq >= total {
            return Some(payload);
        }

        let entry = self
            .messages
            .entry(message_id.to_owned())
            .or_insert_with(|| FeishuWsFragmentSet {
                created_at: Instant::now(),
                total,
                parts: vec![None; total],
            });
        if entry.total != total {
            *entry = FeishuWsFragmentSet {
                created_at: Instant::now(),
                total,
                parts: vec![None; total],
            };
        }
        if let Some(part) = entry.parts.get_mut(seq) {
            *part = Some(payload);
            entry.created_at = Instant::now();
        } else {
            return Some(payload);
        }
        if entry.parts.iter().any(Option::is_none) {
            return None;
        }

        let mut combined = Vec::new();
        for part in entry.parts.iter().flatten() {
            combined.extend_from_slice(part);
        }
        self.messages.remove(message_id);
        Some(combined)
    }

    fn retain_recent(&mut self) {
        let ttl = Duration::from_secs(10);
        self.messages
            .retain(|_, set| set.created_at.elapsed() <= ttl);
    }
}

pub(super) async fn run_feishu_websocket_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedFeishuChannelConfig,
    resolved_path: &Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    let mut adapter = FeishuAdapter::new(resolved)?;
    adapter.refresh_tenant_token().await?;
    let state = FeishuWebhookState::new_with_resolved_path(
        config.clone(),
        resolved_path.to_path_buf(),
        resolved,
        adapter,
        kernel_ctx,
        runtime,
    );
    let client = FeishuClient::from_configs(resolved, &config.feishu_integration)?;

    #[allow(clippy::print_stdout)]
    {
        println!(
            "feishu channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, mode=websocket)",
            resolved_path.display(),
            resolved.configured_account_id,
            resolved.account.label,
            selected_by_default,
            default_account_source.as_str()
        );
    }

    tracing::info!(
        target: "loongclaw.channel.feishu",
        transport = "websocket",
        config_path = %resolved_path.display(),
        configured_account_id = %resolved.configured_account_id,
        account_id = %resolved.account.id,
        selected_by_default,
        default_source = default_account_source.as_str(),
        "feishu runtime started"
    );

    loop {
        let endpoint = match tokio::select! {
            _ = stop.wait() => return Ok(()),
            endpoint = client.get_websocket_endpoint() => endpoint,
        } {
            Ok(endpoint) => endpoint,
            Err(error) => {
                #[allow(clippy::print_stderr)]
                {
                    eprintln!("warning: feishu websocket endpoint discovery failed: {error}");
                }
                tokio::select! {
                    _ = stop.wait() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(DEFAULT_WS_RECONNECT_INTERVAL_S)) => {}
                }
                continue;
            }
        };
        let ws_config = endpoint.client_config.unwrap_or_default();
        let reconnect_interval = Duration::from_secs(
            ws_config
                .reconnect_interval_s
                .unwrap_or(DEFAULT_WS_RECONNECT_INTERVAL_S)
                .max(1),
        );

        if let Err(error) = Box::pin(run_feishu_websocket_session(
            &state,
            &endpoint.url,
            &ws_config,
            stop.clone(),
        ))
        .await
        {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("warning: feishu websocket session ended: {error}");
            }
        }

        tokio::select! {
            _ = stop.wait() => return Ok(()),
            _ = tokio::time::sleep(reconnect_interval) => {}
        }
    }
}

async fn run_feishu_websocket_session(
    state: &FeishuWebhookState,
    url: &str,
    ws_config: &FeishuWsEndpointClientConfig,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    let redacted_url = redact_feishu_websocket_log_url(url);

    tracing::info!(
        target: "loongclaw.channel.feishu",
        transport = "websocket",
        configured_account_id = %state.configured_account_id(),
        account_id = %state.account_id(),
        url = %redacted_url,
        "connecting feishu websocket session"
    );

    let parsed_url = reqwest::Url::parse(url)
        .map_err(|error| format!("parse Feishu websocket URL failed: {error}"))?;
    let service_id = parsed_url
        .query_pairs()
        .find_map(|(key, value)| (key == "service_id").then_some(value))
        .ok_or_else(|| "Feishu websocket URL missing service_id".to_owned())?
        .parse::<i32>()
        .map_err(|error| format!("parse Feishu websocket service_id failed: {error}"))?;
    let ping_interval_s = ws_config
        .ping_interval_s
        .unwrap_or(DEFAULT_WS_PING_INTERVAL_S)
        .max(1);

    // Feishu already uses reqwest's ring-backed rustls path for HTTP calls in the same flow.
    // Install the same process default once so websocket TLS does not panic when other crates
    // also link rustls with aws-lc-rs enabled.
    ensure_feishu_websocket_rustls_provider();
    let connect_result = tokio::select! {
        _ = stop.wait() => return Ok(()),
        result = connect_async(parsed_url.as_str()) => result,
    };
    let (mut stream, _) =
        connect_result.map_err(|error| format!("connect Feishu websocket failed: {error}"))?;
    let mut ping_interval = tokio::time::interval(Duration::from_secs(ping_interval_s));
    ping_interval.tick().await;
    let mut fragments = FeishuWsFragments::default();

    loop {
        tokio::select! {
            _ = stop.wait() => return Ok(()),
            _ = ping_interval.tick() => {
                let ping_frame = FeishuWsFrame {
                    seq_id: 0,
                    log_id: 0,
                    service: service_id,
                    method: FRAME_TYPE_CONTROL,
                    headers: vec![FeishuWsHeader {
                        key: HEADER_TYPE.to_owned(),
                        value: MESSAGE_TYPE_PING.to_owned(),
                    }],
                    payload_encoding: String::new(),
                    payload_type: String::new(),
                    payload: Vec::new(),
                    log_id_new: String::new(),
                };
                let mut bytes = Vec::new();
                ping_frame
                    .encode(&mut bytes)
                    .map_err(|error| format!("encode Feishu websocket ping frame failed: {error}"))?;
                stream
                    .send(Message::Binary(bytes.into()))
                    .await
                    .map_err(|error| format!("send Feishu websocket ping failed: {error}"))?;
            }
            maybe_message = stream.next() => {
                let message = match maybe_message {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        return Err(format!("read Feishu websocket frame failed: {error}"));
                    }
                    None => return Err("Feishu websocket closed by remote peer".to_owned()),
                };

                match message {
                    Message::Binary(bytes) => {
                        let mut frame = FeishuWsFrame::decode(bytes.as_ref())
                            .map_err(|error| format!("decode Feishu websocket frame failed: {error}"))?;
                        if frame.method == FRAME_TYPE_CONTROL {
                            if frame.header_value(HEADER_TYPE) == Some(MESSAGE_TYPE_PONG)
                                && !frame.payload.is_empty()
                                && let Ok(config) = serde_json::from_slice::<FeishuWsEndpointClientConfig>(&frame.payload)
                                && let Some(next_ping_interval_s) = config.ping_interval_s
                            {
                                let interval = next_ping_interval_s.max(1);
                                ping_interval = tokio::time::interval(Duration::from_secs(interval));
                                ping_interval.tick().await;
                            }
                            continue;
                        }
                        if frame.method != FRAME_TYPE_DATA {
                            continue;
                        }

                        let total = frame.header_value_usize(HEADER_SUM).unwrap_or(1);
                        let seq = frame.header_value_usize(HEADER_SEQ).unwrap_or(0);
                        let message_id = frame.header_value(HEADER_MESSAGE_ID).unwrap_or_default().to_owned();
                        let Some(payload_bytes) = fragments.combine(&message_id, total, seq, frame.payload.clone()) else {
                            continue;
                        };

                        let started_at = Instant::now();
                        let payload = serde_json::from_slice::<Value>(&payload_bytes).map_err(|error| {
                            format!("decode Feishu websocket event payload failed: {error}")
                        })?;
                        tracing::info!(
                            target: "loongclaw.channel.feishu",
                            transport = "websocket",
                            configured_account_id = %state.configured_account_id(),
                            message_id = %message_id,
                            seq,
                            total,
                            "received feishu websocket event payload"
                        );
                        let response: FeishuWsOutboundResponse = match state.parse_websocket_payload(&payload) {
                            Ok(parsed) => match Box::pin(handle_feishu_parsed_action(state, parsed)).await {
                                Ok(response) => build_ws_success_response(response, started_at.elapsed()),
                                Err((status, message)) => build_ws_error_response(status, started_at.elapsed(), message),
                            },
                            Err(error) => build_ws_error_response(map_parse_error_status(&error), started_at.elapsed(), error),
                        };
                        let response_bytes = encode_ws_response_frame(&mut frame, &response)?;
                        let deferred_updates = response.deferred_updates;
                        stream
                            .send(Message::Binary(response_bytes.into()))
                            .await
                            .map_err(|error| format!("send Feishu websocket response failed: {error}"))?;
                        state.dispatch_deferred_updates(deferred_updates);
                    }
                    Message::Close(_) => return Err("Feishu websocket closed by remote peer".to_owned()),
                    Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

fn build_ws_success_response(
    response: FeishuParsedActionResponse,
    elapsed: Duration,
) -> FeishuWsOutboundResponse {
    FeishuWsOutboundResponse {
        status: StatusCode::OK,
        body: response.websocket_body,
        deferred_updates: response.deferred_updates,
        biz_rt_ms: elapsed.as_millis() as u64,
    }
}

fn build_ws_error_response(
    status: StatusCode,
    elapsed: Duration,
    _message: String,
) -> FeishuWsOutboundResponse {
    FeishuWsOutboundResponse {
        status,
        body: None,
        deferred_updates: Vec::new(),
        biz_rt_ms: elapsed.as_millis() as u64,
    }
}

struct FeishuWsOutboundResponse {
    status: StatusCode,
    body: Option<Value>,
    deferred_updates: Vec<crate::tools::DeferredFeishuCardUpdate>,
    biz_rt_ms: u64,
}

fn encode_ws_response_frame(
    frame: &mut FeishuWsFrame,
    response: &FeishuWsOutboundResponse,
) -> CliResult<Vec<u8>> {
    let payload = FeishuWsResponseEnvelope {
        code: response.status.as_u16(),
        headers: BTreeMap::new(),
        data: response
            .body
            .as_ref()
            .map(|body| {
                serde_json::to_vec(&body)
                    .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            })
            .transpose()
            .map_err(|error| format!("serialize Feishu websocket callback body failed: {error}"))?,
    };
    frame.set_header(HEADER_BIZ_RT, response.biz_rt_ms.to_string());
    frame.payload = serde_json::to_vec(&payload)
        .map_err(|error| format!("serialize Feishu websocket response payload failed: {error}"))?;
    let mut bytes = Vec::new();
    frame
        .encode(&mut bytes)
        .map_err(|error| format!("encode Feishu websocket response frame failed: {error}"))?;
    Ok(bytes)
}

fn map_parse_error_status(error: &str) -> StatusCode {
    if error.starts_with("unauthorized:") {
        return StatusCode::UNAUTHORIZED;
    }
    StatusCode::BAD_REQUEST
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        response::IntoResponse,
        routing::post,
    };
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex, Notify};
    use tokio_tungstenite::accept_async;

    use super::*;
    use crate::channel::ChannelPlatform;

    const MOCK_PROVIDER_MARKDOWN_REPLY: &str = "## structured inbound ack\n\n- rendered";
    const FEISHU_WEBSOCKET_TEST_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
    use crate::config::{FeishuChannelServeMode, LoongClawConfig, ProviderConfig};
    use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_test_kernel_context};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockRequest {
        path: String,
        authorization: Option<String>,
        body: String,
    }

    #[test]
    fn redact_feishu_websocket_log_url_strips_query_fragment_and_userinfo() {
        let raw_url =
            "wss://user:secret@example.com/ws/connect?ticket=secret-token&service_id=7#fragment";
        let redacted_url = redact_feishu_websocket_log_url(raw_url);

        assert_eq!(redacted_url, "wss://example.com/ws/connect");
    }

    #[test]
    fn redact_feishu_websocket_log_url_falls_back_for_invalid_urls() {
        let raw_url = "not a url?ticket=secret-token#fragment";
        let redacted_url = redact_feishu_websocket_log_url(raw_url);

        assert_eq!(redacted_url, "not a url");
    }

    #[derive(Clone, Default)]
    struct MockServerState {
        requests: Arc<Mutex<Vec<MockRequest>>>,
    }

    fn temp_websocket_test_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-feishu-websocket-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn run_feishu_websocket_test_on_large_stack<F, Fut>(thread_name: &str, operation: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let join_handle = std::thread::Builder::new()
            .name(thread_name.to_owned())
            .stack_size(FEISHU_WEBSOCKET_TEST_STACK_SIZE_BYTES)
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build feishu websocket test runtime");
                runtime.block_on(operation());
            })
            .expect("spawn feishu websocket large-stack test thread");
        match join_handle.join() {
            Ok(()) => {}
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    async fn spawn_mock_http_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock http server");
        let address = listener.local_addr().expect("mock http server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock http server");
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
        spawn_mock_http_server(router).await
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
                                "tenant_access_token": "t-token-websocket"
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
                                    "reaction_id": "reaction_websocket_1"
                                }
                            }))
                        }
                    }
                }),
            );
        spawn_mock_http_server(router).await
    }

    fn test_websocket_config(provider_base_url: &str, feishu_base_url: &str) -> LoongClawConfig {
        let temp_dir = temp_websocket_test_dir("runtime");
        std::fs::create_dir_all(&temp_dir).expect("create websocket temp dir");

        let mut config = LoongClawConfig {
            provider: ProviderConfig {
                base_url: provider_base_url.to_owned(),
                api_key: Some(loongclaw_contracts::SecretRef::Inline(
                    "test-provider-key".to_owned(),
                )),
                model: "test-model".to_owned(),
                ..ProviderConfig::default()
            },
            ..LoongClawConfig::default()
        };
        config.memory.sqlite_path = temp_dir.join("memory.sqlite3").display().to_string();
        config.feishu.enabled = true;
        config.feishu.account_id = Some("feishu_main".to_owned());
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
            "cli_a1b2c3".to_owned(),
        ));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "secret-123".to_owned(),
        ));
        config.feishu.base_url = Some(feishu_base_url.to_owned());
        config.feishu.mode = Some(FeishuChannelServeMode::Websocket);
        config.feishu.receive_id_type = "chat_id".to_owned();
        config.feishu.allowed_chat_ids = vec!["oc_demo".to_owned()];
        config.feishu.verification_token = None;
        config.feishu.encrypt_key = None;
        config
    }

    async fn spawn_mock_ws_server(
        payload: Value,
    ) -> (String, tokio::task::JoinHandle<CliResult<FeishuWsFrame>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock websocket server");
        let address = listener.local_addr().expect("mock websocket server addr");
        let handle = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut stream = accept_async(socket)
                .await
                .map_err(|error| format!("accept websocket failed: {error}"))?;
            let request_frame = FeishuWsFrame {
                seq_id: 1,
                log_id: 1,
                service: 42,
                method: FRAME_TYPE_DATA,
                headers: vec![
                    FeishuWsHeader {
                        key: HEADER_MESSAGE_ID.to_owned(),
                        value: "evt_ws_inbound_1".to_owned(),
                    },
                    FeishuWsHeader {
                        key: HEADER_SEQ.to_owned(),
                        value: "0".to_owned(),
                    },
                    FeishuWsHeader {
                        key: HEADER_SUM.to_owned(),
                        value: "1".to_owned(),
                    },
                ],
                payload_encoding: "json".to_owned(),
                payload_type: "event".to_owned(),
                payload: serde_json::to_vec(&payload)
                    .map_err(|error| format!("encode websocket payload failed: {error}"))?,
                log_id_new: String::new(),
            };
            let mut bytes = Vec::new();
            request_frame
                .encode(&mut bytes)
                .map_err(|error| format!("encode websocket frame failed: {error}"))?;
            stream
                .send(Message::Binary(bytes.into()))
                .await
                .map_err(|error| format!("send websocket frame failed: {error}"))?;

            loop {
                let message = stream
                    .next()
                    .await
                    .ok_or_else(|| "websocket client disconnected before replying".to_owned())?
                    .map_err(|error| format!("read websocket reply failed: {error}"))?;
                match message {
                    Message::Binary(bytes) => {
                        let response = FeishuWsFrame::decode(bytes.as_ref()).map_err(|error| {
                            format!("decode websocket reply frame failed: {error}")
                        })?;
                        stream
                            .close(None)
                            .await
                            .map_err(|error| format!("close websocket server failed: {error}"))?;
                        return Ok(response);
                    }
                    Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Frame(_) => {}
                    Message::Close(_) => {
                        return Err("websocket client closed before sending a reply".to_owned());
                    }
                }
            }
        });
        (format!("ws://{address}/events?service_id=42"), handle)
    }

    #[test]
    fn encode_ws_response_frame_base64_encodes_callback_body() {
        let mut frame = FeishuWsFrame {
            seq_id: 7,
            log_id: 11,
            service: 42,
            method: FRAME_TYPE_DATA,
            headers: vec![],
            payload_encoding: "json".to_owned(),
            payload_type: "event".to_owned(),
            payload: Vec::new(),
            log_id_new: String::new(),
        };
        let response = FeishuWsOutboundResponse {
            status: StatusCode::OK,
            body: Some(json!({
                "toast": {
                    "type": "success",
                    "content": "approved"
                }
            })),
            deferred_updates: Vec::new(),
            biz_rt_ms: 12,
        };

        let encoded = encode_ws_response_frame(&mut frame, &response).expect("encode response");
        let decoded = FeishuWsFrame::decode(encoded.as_slice()).expect("decode response frame");
        let envelope = serde_json::from_slice::<Value>(&decoded.payload).expect("response json");

        assert_eq!(envelope["code"], json!(200));
        assert_eq!(decoded.header_value(HEADER_BIZ_RT), Some("12"));
        let data = envelope["data"].as_str().expect("base64 callback body");
        let body = base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("decode base64 callback body");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("decoded callback body json"),
            json!({
                "toast": {
                    "type": "success",
                    "content": "approved"
                }
            })
        );
    }

    #[test]
    fn feishu_websocket_fragments_refresh_ttl_when_new_chunks_arrive() {
        run_feishu_websocket_test_on_large_stack("feishu-websocket-fragments", || async move {
            feishu_websocket_fragments_refresh_ttl_when_new_chunks_arrive_impl().await;
        });
    }

    async fn feishu_websocket_fragments_refresh_ttl_when_new_chunks_arrive_impl() {
        let mut fragments = FeishuWsFragments::default();
        assert_eq!(
            fragments.combine("evt_ws_fragments", 3, 0, b"hel".to_vec()),
            None
        );

        fragments
            .messages
            .get_mut("evt_ws_fragments")
            .expect("fragment entry after first chunk")
            .created_at = Instant::now() - Duration::from_millis(9_900);

        assert_eq!(
            fragments.combine("evt_ws_fragments", 3, 1, b"lo ".to_vec()),
            None
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            fragments.combine("evt_ws_fragments", 3, 2, b"ws".to_vec()),
            Some(b"hello ws".to_vec()),
            "recent fragment progress should keep the in-flight assembly alive"
        );
    }

    #[test]
    fn feishu_websocket_wss_urls_do_not_fail_due_to_missing_tls_support() {
        run_feishu_websocket_test_on_large_stack("feishu-websocket-wss-url", || async move {
            feishu_websocket_wss_urls_do_not_fail_due_to_missing_tls_support_impl().await;
        });
    }

    async fn feishu_websocket_wss_urls_do_not_fail_due_to_missing_tls_support_impl() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind disposable tcp listener");
        let address = listener.local_addr().expect("disposable listener addr");
        drop(listener);

        let error = connect_async(format!("wss://{address}/events?service_id=42"))
            .await
            .expect_err("closed port should reject the wss connection");

        assert!(
            !error.to_string().contains("TLS support not compiled in"),
            "wss support must be compiled in for Feishu websocket mode: {error}"
        );
    }

    #[test]
    fn feishu_websocket_wss_session_surfaces_tls_errors_without_panicking() {
        run_feishu_websocket_test_on_large_stack("feishu-websocket-wss-session", || async move {
            feishu_websocket_wss_session_surfaces_tls_errors_without_panicking_impl().await;
        });
    }

    async fn feishu_websocket_wss_session_surfaces_tls_errors_without_panicking_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_ws_tls_1").await;

        let config = test_websocket_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve websocket feishu account");
        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before websocket tls test");
        let kernel_ctx =
            bootstrap_test_kernel_context("feishu-websocket-wss-test", DEFAULT_TOKEN_TTL_S)
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

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock tls listener");
        let address = listener.local_addr().expect("mock tls listener addr");
        let accept_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept mock tls socket");
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(socket);
        });

        let session_url = format!("wss://{address}/events?service_id=42");
        let session_join = tokio::time::timeout(
            Duration::from_secs(15),
            tokio::spawn(async move {
                run_feishu_websocket_session(
                    &state,
                    session_url.as_str(),
                    &FeishuWsEndpointClientConfig::default(),
                    ChannelServeStopHandle::new(),
                )
                .await
            }),
        )
        .await
        .expect("wss session should not hang");
        let session_error = session_join
            .expect("wss session should return a recoverable error instead of panicking")
            .expect_err("plain tcp listener should not complete a tls websocket session");
        assert!(
            session_error.starts_with("connect Feishu websocket failed:"),
            "unexpected websocket tls session result: {session_error}"
        );
        assert!(
            !session_error
                .contains("Could not automatically determine the process-level CryptoProvider"),
            "wss session should not panic when rustls has multiple providers enabled: {session_error}"
        );

        accept_task.abort();
        let _ = accept_task.await;
        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_websocket_session_stop_interrupts_stalled_initial_connect() {
        run_feishu_websocket_test_on_large_stack("feishu-websocket-stop", || async move {
            feishu_websocket_session_stop_interrupts_stalled_initial_connect_impl().await;
        });
    }

    async fn feishu_websocket_session_stop_interrupts_stalled_initial_connect_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_ws_stop_1").await;

        let config = test_websocket_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve websocket feishu account");
        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before websocket stop test");
        let kernel_ctx =
            bootstrap_test_kernel_context("feishu-websocket-stop-test", DEFAULT_TOKEN_TTL_S)
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

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stalled websocket listener");
        let address = listener
            .local_addr()
            .expect("stalled websocket listener addr");
        let accepted = Arc::new(Notify::new());
        let release_socket = Arc::new(Notify::new());
        let accepted_for_server = accepted.clone();
        let release_for_server = release_socket.clone();
        let server = tokio::spawn(async move {
            let (socket, _) = listener
                .accept()
                .await
                .expect("accept stalled websocket socket");
            accepted_for_server.notify_waiters();
            release_for_server.notified().await;
            drop(socket);
        });

        let stop = ChannelServeStopHandle::new();
        let stop_for_session = stop.clone();
        let session_url = format!("ws://{address}/events?service_id=42");
        let mut session = tokio::spawn(async move {
            run_feishu_websocket_session(
                &state,
                session_url.as_str(),
                &FeishuWsEndpointClientConfig::default(),
                stop_for_session,
            )
            .await
        });

        accepted.notified().await;
        stop.request_stop();

        let session_result = tokio::select! {
            result = &mut session => result,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                session.abort();
                panic!("stop should interrupt the stalled websocket connect");
            }
        };
        session_result
            .expect("join stalled websocket session")
            .expect("stop should end stalled websocket connect cleanly");

        release_socket.notify_waiters();
        let _ = server.await;
        provider_server.abort();
        feishu_server.abort();
    }

    #[test]
    fn feishu_websocket_session_reaches_provider_and_replies() {
        run_feishu_websocket_test_on_large_stack("feishu-websocket-session", || async move {
            feishu_websocket_session_reaches_provider_and_replies_impl().await;
        });
    }

    async fn feishu_websocket_session_reaches_provider_and_replies_impl() {
        let provider_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let feishu_requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let (provider_base_url, provider_server) =
            spawn_mock_provider_server(provider_requests.clone()).await;
        let (feishu_base_url, feishu_server) =
            spawn_mock_feishu_api_server(feishu_requests.clone(), "om_reply_ws_1").await;

        let config = test_websocket_config(&provider_base_url, &feishu_base_url);
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve websocket feishu account");
        assert_eq!(resolved.mode, FeishuChannelServeMode::Websocket);

        let mut adapter = FeishuAdapter::new(&resolved).expect("build feishu adapter");
        adapter
            .refresh_tenant_token()
            .await
            .expect("refresh tenant token before websocket test");
        let kernel_ctx =
            bootstrap_test_kernel_context("feishu-websocket-test", DEFAULT_TOKEN_TTL_S)
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
                "event_id": "evt_ws_inbound_1",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": "ou_sender_ws_1"
                    }
                },
                "message": {
                    "chat_id": "oc_demo",
                    "message_id": "om_inbound_ws_1",
                    "message_type": "text",
                    "content": "{\"text\":\"hello over websocket\"}"
                }
            }
        });
        let (url, ws_server) = spawn_mock_ws_server(payload).await;

        let session_error = run_feishu_websocket_session(
            &state,
            url.as_str(),
            &FeishuWsEndpointClientConfig {
                ping_interval_s: Some(30),
                ..FeishuWsEndpointClientConfig::default()
            },
            ChannelServeStopHandle::new(),
        )
        .await
        .expect_err("session should end after the mock server closes");
        assert!(
            session_error.contains("closed by remote peer"),
            "unexpected websocket session result: {session_error}"
        );

        let response_frame = ws_server
            .await
            .expect("join websocket server")
            .expect("capture websocket response frame");
        let response_envelope =
            serde_json::from_slice::<Value>(&response_frame.payload).expect("response envelope");
        assert_eq!(response_envelope["code"], json!(200));
        assert!(response_envelope["data"].is_null());
        assert!(
            response_frame.header_value(HEADER_BIZ_RT).is_some(),
            "response should include Feishu biz_rt timing"
        );

        let provider_requests = provider_requests.lock().await.clone();
        assert_eq!(provider_requests.len(), 1);
        assert_eq!(provider_requests[0].path, "/v1/chat/completions");
        let provider_body =
            serde_json::from_str::<Value>(&provider_requests[0].body).expect("provider body json");
        assert_eq!(provider_body["stream"], json!(true));
        assert!(
            provider_requests[0].body.contains("hello over websocket"),
            "provider request should include the websocket inbound message"
        );

        let feishu_requests = wait_for_request_count(&feishu_requests, 3).await;
        assert_eq!(feishu_requests.len(), 3);
        let reaction_request = feishu_requests
            .iter()
            .find(|request| request.path == "/open-apis/im/v1/messages/om_inbound_ws_1/reactions")
            .expect("websocket flow should add ack reaction");
        assert_eq!(
            reaction_request.authorization.as_deref(),
            Some("Bearer t-token-websocket")
        );
        assert!(
            reaction_request.body.contains("\"emoji_type\""),
            "reaction request should include a Feishu emoji type"
        );
        let reply_request = feishu_requests
            .iter()
            .find(|request| request.path == "/open-apis/im/v1/messages/om_inbound_ws_1/reply")
            .expect("websocket flow should still send a reply");
        assert!(
            reply_request.body.contains("\"msg_type\":\"interactive\""),
            "websocket flow should send markdown-capable interactive cards"
        );
        assert!(
            reply_request.body.contains("\\\"tag\\\":\\\"markdown\\\""),
            "websocket flow should wrap the provider reply in a markdown card"
        );
        assert!(
            reply_request
                .body
                .contains("\\\"content\\\":\\\"## structured inbound ack\\\\n\\\\n- rendered\\\""),
            "websocket flow should preserve provider markdown content"
        );

        provider_server.abort();
        feishu_server.abort();
    }
}
