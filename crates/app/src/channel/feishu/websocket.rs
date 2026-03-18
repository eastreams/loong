use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
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
use crate::channel::runtime_state::ChannelOperationRuntimeTracker;
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedFeishuChannelConfig,
};
use crate::feishu::{FeishuClient, FeishuWsEndpointClientConfig};

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

    loop {
        let endpoint = client.get_websocket_endpoint().await?;
        let ws_config = endpoint.client_config.unwrap_or_default();
        let reconnect_interval = Duration::from_secs(
            ws_config
                .reconnect_interval_s
                .unwrap_or(DEFAULT_WS_RECONNECT_INTERVAL_S)
                .max(1),
        );

        if let Err(error) = run_feishu_websocket_session(&state, &endpoint.url, &ws_config).await {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("warning: feishu websocket session ended: {error}");
            }
        }

        tokio::time::sleep(reconnect_interval).await;
    }
}

async fn run_feishu_websocket_session(
    state: &FeishuWebhookState,
    url: &str,
    ws_config: &FeishuWsEndpointClientConfig,
) -> CliResult<()> {
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

    let (mut stream, _) = connect_async(parsed_url.as_str())
        .await
        .map_err(|error| format!("connect Feishu websocket failed: {error}"))?;
    let mut ping_interval = tokio::time::interval(Duration::from_secs(ping_interval_s));
    ping_interval.tick().await;
    let mut fragments = FeishuWsFragments::default();

    loop {
        tokio::select! {
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
                    .send(Message::Binary(bytes))
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
                        let response: FeishuWsOutboundResponse = match state.parse_websocket_payload(&payload) {
                            Ok(parsed) => match handle_feishu_parsed_action(state, parsed).await {
                                Ok(response) => build_ws_success_response(response, started_at.elapsed()),
                                Err((status, message)) => build_ws_error_response(status, started_at.elapsed(), message),
                            },
                            Err(error) => build_ws_error_response(map_parse_error_status(&error), started_at.elapsed(), error),
                        };
                        let response_bytes = encode_ws_response_frame(&mut frame, &response)?;
                        let deferred_updates = response.deferred_updates;
                        stream
                            .send(Message::Binary(response_bytes))
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
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::{
        Json, Router,
        body::to_bytes,
        extract::{Request, State},
        routing::post,
    };
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tokio_tungstenite::accept_async;

    use super::*;
    use crate::channel::ChannelPlatform;
    use crate::config::{FeishuChannelServeMode, LoongClawConfig, ProviderConfig};
    use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_test_kernel_context};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockRequest {
        path: String,
        authorization: Option<String>,
        body: String,
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

    async fn record_request(State(state): State<MockServerState>, request: Request) {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, usize::MAX)
            .await
            .expect("read mock request body");
        state.requests.lock().await.push(MockRequest {
            path: parts.uri.path().to_owned(),
            authorization: parts
                .headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body: String::from_utf8(body.to_vec()).expect("mock request body utf8"),
        });
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
                        record_request(State(state), request).await;
                        Json(json!({
                            "choices": [{
                                "message": {
                                    "content": "structured inbound ack"
                                }
                            }]
                        }))
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
            );
        spawn_mock_http_server(router).await
    }

    fn test_websocket_config(provider_base_url: &str, feishu_base_url: &str) -> LoongClawConfig {
        let temp_dir = temp_websocket_test_dir("runtime");
        std::fs::create_dir_all(&temp_dir).expect("create websocket temp dir");

        let mut config = LoongClawConfig {
            provider: ProviderConfig {
                base_url: provider_base_url.to_owned(),
                api_key: Some("test-provider-key".to_owned()),
                model: "test-model".to_owned(),
                ..ProviderConfig::default()
            },
            ..LoongClawConfig::default()
        };
        config.memory.sqlite_path = temp_dir.join("memory.sqlite3").display().to_string();
        config.feishu.enabled = true;
        config.feishu.account_id = Some("feishu_main".to_owned());
        config.feishu.app_id = Some("cli_a1b2c3".to_owned());
        config.feishu.app_secret = Some("secret-123".to_owned());
        config.feishu.base_url = Some(feishu_base_url.to_owned());
        config.feishu.mode = FeishuChannelServeMode::Websocket;
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
                .send(Message::Binary(bytes))
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

    #[tokio::test]
    async fn feishu_websocket_session_reaches_provider_and_replies() {
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
        assert!(
            provider_requests[0].body.contains("hello over websocket"),
            "provider request should include the websocket inbound message"
        );

        let feishu_requests = feishu_requests.lock().await.clone();
        assert_eq!(feishu_requests.len(), 2);
        assert_eq!(
            feishu_requests[1].path,
            "/open-apis/im/v1/messages/om_inbound_ws_1/reply"
        );
        assert_eq!(
            feishu_requests[1].authorization.as_deref(),
            Some("Bearer t-token-websocket")
        );
        assert!(
            feishu_requests[1]
                .body
                .contains("\\\"text\\\":\\\"structured inbound ack\\\""),
            "websocket flow should still send the provider reply back through Feishu"
        );

        provider_server.abort();
        feishu_server.abort();
    }
}
