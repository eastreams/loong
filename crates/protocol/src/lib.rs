use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, stdin, stdout};
use tokio::sync::{Mutex, mpsc};

pub const PROTOCOL_VERSION: u32 = 1;

fn default_frame_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportInfo {
    pub name: String,
    pub version: String,
    pub secure: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboundFrame {
    pub method: String,
    pub id: Option<String>,
    pub payload: Value,
    #[serde(default = "default_frame_version")]
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboundFrame {
    pub method: String,
    pub id: Option<String>,
    pub payload: Value,
    #[serde(default = "default_frame_version")]
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolRoute {
    ToolsCall,
    Custom(String),
}

impl ProtocolRoute {
    pub fn from_method(method: &str) -> Self {
        match method {
            "tools/call" => Self::ToolsCall,
            other => Self::Custom(other.to_owned()),
        }
    }

    pub fn method(&self) -> &str {
        match self {
            Self::ToolsCall => "tools/call",
            Self::Custom(method) => method,
        }
    }

    pub fn is_standard(&self) -> bool {
        matches!(self, Self::ToolsCall)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePolicy {
    pub allow_anonymous: bool,
    pub required_capability: Option<String>,
}

impl Default for RoutePolicy {
    fn default() -> Self {
        Self {
            allow_anonymous: true,
            required_capability: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoute {
    pub route: ProtocolRoute,
    pub policy: RoutePolicy,
}

impl ResolvedRoute {
    pub fn method(&self) -> &str {
        self.route.method()
    }
}

#[derive(Debug, Clone)]
pub struct ProtocolRouter {
    strict: bool,
    custom_routes: BTreeMap<String, RoutePolicy>,
}

impl Default for ProtocolRouter {
    fn default() -> Self {
        Self::new(false)
    }
}

impl ProtocolRouter {
    pub fn new(strict: bool) -> Self {
        Self {
            strict,
            custom_routes: BTreeMap::new(),
        }
    }

    pub fn strict() -> Self {
        Self::new(true)
    }

    pub fn register_custom_route(
        &mut self,
        method: impl Into<String>,
        policy: RoutePolicy,
    ) -> Result<(), RouterError> {
        let method = method.into();
        validate_method_name(&method)?;
        if ProtocolRoute::from_method(&method).is_standard() {
            return Err(RouterError::InvalidMethod(format!(
                "standard route cannot be registered as custom: {method}"
            )));
        }
        self.custom_routes.insert(method, policy);
        Ok(())
    }

    pub fn resolve(&self, method: &str) -> Result<ResolvedRoute, RouterError> {
        validate_method_name(method)?;
        let route = ProtocolRoute::from_method(method);
        match route {
            ProtocolRoute::ToolsCall => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: false,
                    required_capability: Some("invoke".to_owned()),
                },
            }),
            ProtocolRoute::Custom(custom) => {
                if let Some(policy) = self.custom_routes.get(&custom) {
                    Ok(ResolvedRoute {
                        route: ProtocolRoute::Custom(custom),
                        policy: policy.clone(),
                    })
                } else if self.strict {
                    Err(RouterError::UnknownMethod(custom))
                } else {
                    Ok(ResolvedRoute {
                        route: ProtocolRoute::Custom(custom),
                        policy: RoutePolicy::default(),
                    })
                }
            }
        }
    }

    pub fn authorize(
        &self,
        resolved: &ResolvedRoute,
        request: &RouteAuthorizationRequest,
    ) -> Result<RouteAuthorizationDecision, RouteAuthorizationError> {
        if let Some(required) = &resolved.policy.required_capability {
            let normalized_required = normalize_capability(required);
            let has_required = request.capabilities.iter().any(|capability| {
                let normalized = normalize_capability(capability);
                normalized == normalized_required || normalized == "*"
            });
            if !has_required {
                return Err(RouteAuthorizationError::MissingCapability {
                    method: resolved.method().to_owned(),
                    required_capability: normalized_required,
                });
            }
        }

        Ok(RouteAuthorizationDecision::Allow)
    }
}

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("unknown protocol method: {0}")]
    UnknownMethod(String),
    #[error("invalid protocol method: {0}")]
    InvalidMethod(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAuthorizationRequest {
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteAuthorizationDecision {
    Allow,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RouteAuthorizationError {
    #[error("missing capability `{required_capability}` for method: {method}")]
    MissingCapability {
        method: String,
        required_capability: String,
    },
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport closed")]
    Closed,
    #[error("transport failure: {0}")]
    Failure(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub fn validate_method_name(method: &str) -> Result<(), RouterError> {
    if method.trim().is_empty() {
        return Err(RouterError::InvalidMethod(
            "method cannot be empty".to_owned(),
        ));
    }
    if method.trim() != method {
        return Err(RouterError::InvalidMethod(format!(
            "method cannot include surrounding whitespace: {method:?}"
        )));
    }
    if method.len() > 128 {
        return Err(RouterError::InvalidMethod(format!(
            "method exceeds max length (128): {}",
            method.len()
        )));
    }
    if method.starts_with('/') || method.ends_with('/') || method.contains("//") {
        return Err(RouterError::InvalidMethod(format!(
            "method contains invalid slash placement: {method}"
        )));
    }
    if !method.chars().all(|ch| {
        ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '/' | '.')
    }) {
        return Err(RouterError::InvalidMethod(format!(
            "method contains unsupported characters: {method}"
        )));
    }
    Ok(())
}

fn normalize_capability(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

#[async_trait]
pub trait Transport: Send + Sync + 'static {
    fn info(&self) -> TransportInfo;
    async fn send(&self, frame: OutboundFrame) -> Result<(), TransportError>;
    async fn recv(&self) -> Result<Option<InboundFrame>, TransportError>;
    async fn close(&self) -> Result<(), TransportError>;
}

#[derive(Debug)]
pub struct ChannelTransport {
    info: TransportInfo,
    outbound: Mutex<Option<mpsc::Sender<InboundFrame>>>,
    inbound: Mutex<mpsc::Receiver<InboundFrame>>,
}

impl ChannelTransport {
    pub fn linked(
        capacity: usize,
        left_info: TransportInfo,
        right_info: TransportInfo,
    ) -> Result<(Self, Self), TransportBuildError> {
        if capacity == 0 {
            return Err(TransportBuildError::InvalidCapacity(capacity));
        }

        let (left_to_right_tx, left_to_right_rx) = mpsc::channel::<InboundFrame>(capacity);
        let (right_to_left_tx, right_to_left_rx) = mpsc::channel::<InboundFrame>(capacity);

        let left = Self {
            info: left_info,
            outbound: Mutex::new(Some(left_to_right_tx)),
            inbound: Mutex::new(right_to_left_rx),
        };
        let right = Self {
            info: right_info,
            outbound: Mutex::new(Some(right_to_left_tx)),
            inbound: Mutex::new(left_to_right_rx),
        };
        Ok((left, right))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransportBuildError {
    #[error("channel transport capacity must be greater than 0, got {0}")]
    InvalidCapacity(usize),
}

#[async_trait]
impl Transport for ChannelTransport {
    fn info(&self) -> TransportInfo {
        self.info.clone()
    }

    async fn send(&self, frame: OutboundFrame) -> Result<(), TransportError> {
        let sender = self
            .outbound
            .lock()
            .await
            .as_ref()
            .cloned()
            .ok_or(TransportError::Closed)?;

        return sender
            .send(InboundFrame {
                method: frame.method,
                id: frame.id,
                payload: frame.payload,
                version: frame.version,
            })
            .await
            .map_err(|_err| TransportError::Closed);
    }

    async fn recv(&self) -> Result<Option<InboundFrame>, TransportError> {
        Ok(self.inbound.lock().await.recv().await)
    }

    async fn close(&self) -> Result<(), TransportError> {
        let mut outbound = self.outbound.lock().await;
        outbound.take();
        Ok(())
    }
}

pub struct JsonLineTransport<R, W> {
    info: TransportInfo,
    reader: Mutex<BufReader<R>>,
    writer: Mutex<W>,
    closed: AtomicBool,
}

impl<R, W> JsonLineTransport<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    pub fn new(info: TransportInfo, reader: R, writer: W) -> Self {
        Self {
            info,
            reader: Mutex::new(BufReader::new(reader)),
            writer: Mutex::new(writer),
            closed: AtomicBool::new(false),
        }
    }
}

impl JsonLineTransport<tokio::io::Stdin, tokio::io::Stdout> {
    pub fn stdio(info: TransportInfo) -> Self {
        Self::new(info, stdin(), stdout())
    }
}

#[async_trait]
impl<R, W> Transport for JsonLineTransport<R, W>
where
    R: AsyncRead + Unpin + Send + Sync + 'static,
    W: AsyncWrite + Unpin + Send + Sync + 'static,
{
    fn info(&self) -> TransportInfo {
        self.info.clone()
    }

    async fn send(&self, frame: OutboundFrame) -> Result<(), TransportError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(TransportError::Closed);
        }

        let mut payload = serde_json::to_vec(&frame).map_err(|error| {
            TransportError::Failure(format!("failed to encode outbound frame: {error}"))
        })?;
        payload.push(b'\n');

        let mut writer = self.writer.lock().await;
        writer
            .write_all(&payload)
            .await
            .map_err(|error| TransportError::Failure(format!("failed to write frame: {error}")))?;
        writer
            .flush()
            .await
            .map_err(|error| TransportError::Failure(format!("failed to flush frame: {error}")))?;
        Ok(())
    }

    async fn recv(&self) -> Result<Option<InboundFrame>, TransportError> {
        let mut reader = self.reader.lock().await;
        let mut line = String::new();
        loop {
            line.clear();
            let read = reader.read_line(&mut line).await.map_err(|error| {
                TransportError::Failure(format!("failed to read frame: {error}"))
            })?;
            if read == 0 {
                return Ok(None);
            }
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if trimmed.is_empty() {
                continue;
            }
            let frame = serde_json::from_str::<InboundFrame>(trimmed).map_err(|error| {
                TransportError::Failure(format!("failed to decode inbound frame: {error}"))
            })?;
            if frame.version > PROTOCOL_VERSION {
                return Err(TransportError::Protocol(format!(
                    "unsupported frame version {} (max supported: {})",
                    frame.version, PROTOCOL_VERSION
                )));
            }
            return Ok(Some(frame));
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let mut writer = self.writer.lock().await;
        writer
            .shutdown()
            .await
            .map_err(|error| TransportError::Failure(format!("failed to shutdown writer: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncWriteExt, duplex, split};
    use tokio::time::{sleep, timeout};

    fn test_transport_info(name: &str) -> TransportInfo {
        TransportInfo {
            name: name.to_owned(),
            version: "0.1.0-test".to_owned(),
            secure: false,
        }
    }

    #[test]
    fn route_parser_covers_standard_methods() {
        assert_eq!(
            ProtocolRoute::from_method("tools/call"),
            ProtocolRoute::ToolsCall
        );
        assert_eq!(
            ProtocolRoute::from_method("custom/x"),
            ProtocolRoute::Custom("custom/x".to_owned())
        );
        // Previously-standard routes now map to Custom
        assert_eq!(
            ProtocolRoute::from_method("initialize"),
            ProtocolRoute::Custom("initialize".to_owned())
        );
        assert_eq!(
            ProtocolRoute::from_method("ping"),
            ProtocolRoute::Custom("ping".to_owned())
        );
    }

    #[test]
    fn strict_router_rejects_unknown_custom_methods() {
        let router = ProtocolRouter::strict();
        let error = router
            .resolve("internal/unsafe")
            .expect_err("strict mode should block unknown methods");
        assert!(matches!(error, RouterError::UnknownMethod(method) if method == "internal/unsafe"));
    }

    #[test]
    fn custom_route_policy_is_applied() {
        let mut router = ProtocolRouter::strict();
        router
            .register_custom_route(
                "channel/publish",
                RoutePolicy {
                    allow_anonymous: false,
                    required_capability: Some("channel.publish".to_owned()),
                },
            )
            .expect("custom route registration should succeed");

        let resolved = router
            .resolve("channel/publish")
            .expect("registered custom route should resolve");
        assert_eq!(
            resolved.route,
            ProtocolRoute::Custom("channel/publish".to_owned())
        );
        assert!(!resolved.policy.allow_anonymous);
        assert_eq!(
            resolved.policy.required_capability.as_deref(),
            Some("channel.publish")
        );
    }

    #[test]
    fn resolve_rejects_invalid_method_name() {
        let router = ProtocolRouter::default();
        let error = router
            .resolve("Tools/Call")
            .expect_err("invalid characters should be rejected");
        assert!(matches!(error, RouterError::InvalidMethod(_)));
    }

    #[test]
    fn authorize_denies_when_capability_is_missing() {
        let router = ProtocolRouter::default();
        let resolved = router
            .resolve("tools/call")
            .expect("standard route should resolve");
        let error = router
            .authorize(
                &resolved,
                &RouteAuthorizationRequest {
                    capabilities: BTreeSet::from(["discover".to_owned()]),
                },
            )
            .expect_err("tools/call should require invoke");
        assert!(matches!(
            error,
            RouteAuthorizationError::MissingCapability {
                method,
                required_capability
            } if method == "tools/call" && required_capability == "invoke"
        ));
    }

    #[test]
    fn authorize_allows_when_capability_matches() {
        let router = ProtocolRouter::default();
        let resolved = router
            .resolve("tools/call")
            .expect("standard route should resolve");
        let decision = router
            .authorize(
                &resolved,
                &RouteAuthorizationRequest {
                    capabilities: BTreeSet::from([" invoke ".to_owned()]),
                },
            )
            .expect("matching capability should authorize");
        assert_eq!(decision, RouteAuthorizationDecision::Allow);
    }

    #[test]
    fn authorize_supports_wildcard_capability() {
        let router = ProtocolRouter::default();
        let resolved = router
            .resolve("tools/call")
            .expect("standard route should resolve");
        let decision = router
            .authorize(
                &resolved,
                &RouteAuthorizationRequest {
                    capabilities: BTreeSet::from(["*".to_owned()]),
                },
            )
            .expect("wildcard capability should authorize");
        assert_eq!(decision, RouteAuthorizationDecision::Allow);
    }

    #[tokio::test]
    async fn channel_transport_roundtrip_delivers_frame() {
        let (left, right) =
            ChannelTransport::linked(8, test_transport_info("left"), test_transport_info("right"))
                .expect("linked transport should initialize");

        left.send(OutboundFrame {
            method: "tools/call".to_owned(),
            id: Some("req-1".to_owned()),
            payload: serde_json::json!({"tool":"search"}),
            version: PROTOCOL_VERSION,
        })
        .await
        .expect("send should succeed");

        let received = right
            .recv()
            .await
            .expect("recv should succeed")
            .expect("peer frame should be available");
        assert_eq!(received.method, "tools/call");
        assert_eq!(received.id.as_deref(), Some("req-1"));
        assert_eq!(received.payload["tool"], "search");
    }

    #[tokio::test]
    async fn channel_transport_close_stops_future_sends() {
        let (left, _right) =
            ChannelTransport::linked(4, test_transport_info("left"), test_transport_info("right"))
                .expect("linked transport should initialize");

        left.close().await.expect("close should succeed");
        let error = left
            .send(OutboundFrame {
                method: "ping".to_owned(),
                id: None,
                payload: serde_json::json!({}),
                version: PROTOCOL_VERSION,
            })
            .await
            .expect_err("send after close should fail");
        assert!(matches!(error, TransportError::Closed));
    }

    #[tokio::test]
    async fn channel_transport_peer_close_produces_recv_none() {
        let (left, right) =
            ChannelTransport::linked(4, test_transport_info("left"), test_transport_info("right"))
                .expect("linked transport should initialize");

        left.close().await.expect("close should succeed");
        let received = right.recv().await.expect("recv should succeed");
        assert!(received.is_none(), "peer close should end receiver stream");
    }

    #[tokio::test]
    async fn channel_transport_applies_bounded_backpressure() {
        let (left, right) =
            ChannelTransport::linked(1, test_transport_info("left"), test_transport_info("right"))
                .expect("linked transport should initialize");

        left.send(OutboundFrame {
            method: "tools/call".to_owned(),
            id: Some("req-1".to_owned()),
            payload: serde_json::json!({"seq":1}),
            version: PROTOCOL_VERSION,
        })
        .await
        .expect("first send should fill queue");

        let blocked_send = tokio::spawn(async move {
            left.send(OutboundFrame {
                method: "tools/call".to_owned(),
                id: Some("req-2".to_owned()),
                payload: serde_json::json!({"seq":2}),
                version: PROTOCOL_VERSION,
            })
            .await
        });

        sleep(Duration::from_millis(25)).await;
        assert!(
            !blocked_send.is_finished(),
            "second send should remain blocked while queue is full"
        );

        let first = right
            .recv()
            .await
            .expect("recv should succeed")
            .expect("first frame should be present");
        assert_eq!(first.payload["seq"], 1);

        timeout(Duration::from_secs(1), blocked_send)
            .await
            .expect("blocked send should finish once queue drains")
            .expect("join should succeed")
            .expect("send should succeed after drain");

        let second = right
            .recv()
            .await
            .expect("recv should succeed")
            .expect("second frame should be present");
        assert_eq!(second.payload["seq"], 2);
    }

    #[test]
    fn channel_transport_rejects_zero_capacity() {
        let error =
            ChannelTransport::linked(0, test_transport_info("left"), test_transport_info("right"))
                .expect_err("zero capacity must fail");
        assert_eq!(error, TransportBuildError::InvalidCapacity(0));
    }

    #[tokio::test]
    async fn json_line_transport_roundtrip_is_bidirectional() {
        let (left_stream, right_stream) = duplex(4 * 1024);
        let (left_read, left_write) = split(left_stream);
        let (right_read, right_write) = split(right_stream);

        let left = JsonLineTransport::new(test_transport_info("json-left"), left_read, left_write);
        let right =
            JsonLineTransport::new(test_transport_info("json-right"), right_read, right_write);

        left.send(OutboundFrame {
            method: "tools/call".to_owned(),
            id: Some("left-1".to_owned()),
            payload: serde_json::json!({"side":"left"}),
            version: PROTOCOL_VERSION,
        })
        .await
        .expect("left send should succeed");
        let from_left = right
            .recv()
            .await
            .expect("right recv should succeed")
            .expect("right should receive frame");
        assert_eq!(from_left.method, "tools/call");
        assert_eq!(from_left.id.as_deref(), Some("left-1"));
        assert_eq!(from_left.payload["side"], "left");

        right
            .send(OutboundFrame {
                method: "resources/read".to_owned(),
                id: Some("right-1".to_owned()),
                payload: serde_json::json!({"side":"right"}),
                version: PROTOCOL_VERSION,
            })
            .await
            .expect("right send should succeed");
        let from_right = left
            .recv()
            .await
            .expect("left recv should succeed")
            .expect("left should receive frame");
        assert_eq!(from_right.method, "resources/read");
        assert_eq!(from_right.id.as_deref(), Some("right-1"));
        assert_eq!(from_right.payload["side"], "right");
    }

    #[tokio::test]
    async fn json_line_transport_rejects_invalid_json_frame() {
        let (transport_stream, mut peer_stream) = duplex(1024);
        let (reader, writer) = split(transport_stream);
        let transport = JsonLineTransport::new(test_transport_info("json-parse"), reader, writer);

        peer_stream
            .write_all(b"{\"method\":123,\"id\":null,\"payload\":{}}\n")
            .await
            .expect("peer write should succeed");

        let error = transport
            .recv()
            .await
            .expect_err("invalid frame should fail decode");
        assert!(
            matches!(error, TransportError::Failure(ref message) if message.contains("failed to decode inbound frame")),
            "unexpected decode error: {error}"
        );
    }

    #[tokio::test]
    async fn json_line_transport_skips_empty_lines() {
        let (transport_stream, mut peer_stream) = duplex(1024);
        let (reader, writer) = split(transport_stream);
        let transport = JsonLineTransport::new(test_transport_info("json-empty"), reader, writer);

        peer_stream
            .write_all(b"\n\n{\"method\":\"ping\",\"id\":null,\"payload\":{}}\n")
            .await
            .expect("peer write should succeed");

        let received = transport
            .recv()
            .await
            .expect("recv should succeed")
            .expect("frame should be returned");
        assert_eq!(received.method, "ping");
    }

    #[tokio::test]
    async fn json_line_transport_close_blocks_future_sends() {
        let (left_stream, _right_stream) = duplex(1024);
        let (left_read, left_write) = split(left_stream);
        let left = JsonLineTransport::new(test_transport_info("json-close"), left_read, left_write);

        left.close().await.expect("close should succeed");
        let error = left
            .send(OutboundFrame {
                method: "ping".to_owned(),
                id: None,
                payload: serde_json::json!({}),
                version: PROTOCOL_VERSION,
            })
            .await
            .expect_err("send after close should fail");
        assert!(matches!(error, TransportError::Closed));
    }

    #[test]
    fn frame_without_version_deserializes_with_default() {
        let json = r#"{"method":"ping","id":null,"payload":{}}"#;
        let frame: InboundFrame = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(frame.version, 1);
    }

    #[test]
    fn frame_with_explicit_version_is_preserved() {
        let json = r#"{"method":"ping","id":null,"payload":{},"version":1}"#;
        let frame: InboundFrame = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(frame.version, 1);
    }

    #[test]
    fn outbound_frame_serializes_version() {
        let frame = OutboundFrame {
            method: "ping".to_owned(),
            id: None,
            payload: serde_json::json!({}),
            version: PROTOCOL_VERSION,
        };
        let serialized = serde_json::to_value(&frame).expect("should serialize");
        assert_eq!(serialized["version"], PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn json_line_transport_rejects_unsupported_version() {
        let (transport_stream, mut peer_stream) = duplex(1024);
        let (reader, writer) = split(transport_stream);
        let transport = JsonLineTransport::new(test_transport_info("json-version"), reader, writer);

        let future_frame = format!(
            r#"{{"method":"ping","id":null,"payload":{{}},"version":{}}}"#,
            PROTOCOL_VERSION + 1
        );
        peer_stream
            .write_all(format!("{future_frame}\n").as_bytes())
            .await
            .expect("peer write should succeed");

        let error = transport
            .recv()
            .await
            .expect_err("future version should be rejected");
        assert!(
            matches!(error, TransportError::Protocol(ref msg) if msg.contains("unsupported frame version")),
            "unexpected error: {error}"
        );
    }
}
