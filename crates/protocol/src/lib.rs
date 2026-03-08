use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{mpsc, Mutex};

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboundFrame {
    pub method: String,
    pub id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolRoute {
    Initialize,
    Ping,
    ToolsList,
    ToolsCall,
    ResourcesList,
    ResourcesRead,
    Custom(String),
}

impl ProtocolRoute {
    pub fn from_method(method: &str) -> Self {
        match method {
            "initialize" => Self::Initialize,
            "ping" => Self::Ping,
            "tools/list" => Self::ToolsList,
            "tools/call" => Self::ToolsCall,
            "resources/list" => Self::ResourcesList,
            "resources/read" => Self::ResourcesRead,
            other => Self::Custom(other.to_owned()),
        }
    }

    pub fn method(&self) -> &str {
        match self {
            Self::Initialize => "initialize",
            Self::Ping => "ping",
            Self::ToolsList => "tools/list",
            Self::ToolsCall => "tools/call",
            Self::ResourcesList => "resources/list",
            Self::ResourcesRead => "resources/read",
            Self::Custom(method) => method,
        }
    }

    pub fn is_standard(&self) -> bool {
        !matches!(self, Self::Custom(_))
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
        if method.trim().is_empty() {
            return Err(RouterError::InvalidMethod(
                "custom route method cannot be empty".to_owned(),
            ));
        }
        if ProtocolRoute::from_method(&method).is_standard() {
            return Err(RouterError::InvalidMethod(format!(
                "standard route cannot be registered as custom: {method}"
            )));
        }
        self.custom_routes.insert(method, policy);
        Ok(())
    }

    pub fn resolve(&self, method: &str) -> Result<ResolvedRoute, RouterError> {
        let route = ProtocolRoute::from_method(method);
        match route {
            ProtocolRoute::Initialize | ProtocolRoute::Ping => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: true,
                    required_capability: None,
                },
            }),
            ProtocolRoute::ToolsList | ProtocolRoute::ResourcesList => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: true,
                    required_capability: Some("discover".to_owned()),
                },
            }),
            ProtocolRoute::ToolsCall | ProtocolRoute::ResourcesRead => Ok(ResolvedRoute {
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
}

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("unknown protocol method: {0}")]
    UnknownMethod(String),
    #[error("invalid protocol method: {0}")]
    InvalidMethod(String),
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport closed")]
    Closed,
    #[error("transport failure: {0}")]
    Failure(String),
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

        sender
            .send(InboundFrame {
                method: frame.method,
                id: frame.id,
                payload: frame.payload,
            })
            .await
            .map_err(|_| TransportError::Closed)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
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
            ProtocolRoute::from_method("resources/read"),
            ProtocolRoute::ResourcesRead
        );
        assert_eq!(
            ProtocolRoute::from_method("custom/x"),
            ProtocolRoute::Custom("custom/x".to_owned())
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

    #[tokio::test]
    async fn channel_transport_roundtrip_delivers_frame() {
        let (left, right) =
            ChannelTransport::linked(8, test_transport_info("left"), test_transport_info("right"))
                .expect("linked transport should initialize");

        left.send(OutboundFrame {
            method: "tools/call".to_owned(),
            id: Some("req-1".to_owned()),
            payload: serde_json::json!({"tool":"search"}),
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
        })
        .await
        .expect("first send should fill queue");

        let blocked_send = tokio::spawn(async move {
            left.send(OutboundFrame {
                method: "tools/call".to_owned(),
                id: Some("req-2".to_owned()),
                payload: serde_json::json!({"seq":2}),
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
}
