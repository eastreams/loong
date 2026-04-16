use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, stdin, stdout};
use tokio::sync::{Mutex, mpsc};

mod control_plane;

pub const PROTOCOL_VERSION: u32 = 1;
const CONTROL_READ_CAPABILITY: &str = "control_read";
const CONTROL_WRITE_CAPABILITY: &str = "control_write";
const CONTROL_APPROVALS_CAPABILITY: &str = "control_approvals";
const CONTROL_PAIRING_CAPABILITY: &str = "control_pairing";
const CONTROL_ACP_CAPABILITY: &str = "control_acp";

pub use control_plane::{
    CONTROL_PLANE_PROTOCOL_VERSION, ControlPlaneAcpBindingScope, ControlPlaneAcpRoutingOrigin,
    ControlPlaneAcpSessionListResponse, ControlPlaneAcpSessionMetadata, ControlPlaneAcpSessionMode,
    ControlPlaneAcpSessionReadResponse, ControlPlaneAcpSessionState, ControlPlaneAcpSessionStatus,
    ControlPlaneApprovalDecision, ControlPlaneApprovalListResponse,
    ControlPlaneApprovalRequestStatus, ControlPlaneApprovalSummary, ControlPlaneAuthClaims,
    ControlPlaneChallengeResponse, ControlPlaneClientIdentity, ControlPlaneConnectErrorCode,
    ControlPlaneConnectErrorResponse, ControlPlaneConnectRequest, ControlPlaneConnectResponse,
    ControlPlaneDeviceIdentity, ControlPlaneEventEnvelope, ControlPlaneEventName,
    ControlPlanePairingListResponse, ControlPlanePairingRequestSummary,
    ControlPlanePairingResolveRequest, ControlPlanePairingResolveResponse,
    ControlPlanePairingStatus, ControlPlanePolicy, ControlPlanePrincipal,
    ControlPlaneRecentEventsResponse, ControlPlaneRole, ControlPlaneScope,
    ControlPlaneSessionEvent, ControlPlaneSessionKind, ControlPlaneSessionListResponse,
    ControlPlaneSessionObservation, ControlPlaneSessionReadResponse, ControlPlaneSessionState,
    ControlPlaneSessionSummary, ControlPlaneSessionTerminalOutcome, ControlPlaneSessionWorkflow,
    ControlPlaneSessionWorkflowBinding, ControlPlaneSessionWorkflowBindingWorktree,
    ControlPlaneSessionWorkflowContinuity, ControlPlaneSnapshot, ControlPlaneSnapshotResponse,
    ControlPlaneStateVersion, ControlPlaneTaskListResponse, ControlPlaneTaskReadResponse,
    ControlPlaneTaskSummary, ControlPlaneTurnEventEnvelope, ControlPlaneTurnResultResponse,
    ControlPlaneTurnStatus, ControlPlaneTurnSubmitRequest, ControlPlaneTurnSubmitResponse,
    ControlPlaneTurnSummary,
};

fn default_frame_version() -> u32 {
    PROTOCOL_VERSION
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
    ControlChallenge,
    ControlConnect,
    ControlPing,
    ControlSubscribe,
    ControlSnapshot,
    ControlEvents,
    PresenceRead,
    HealthRead,
    SessionList,
    SessionRead,
    TaskList,
    TaskRead,
    TurnSubmit,
    TurnResult,
    TurnStream,
    ApprovalList,
    ApprovalResolve,
    PairingList,
    PairingResolve,
    AcpSessionList,
    AcpSessionRead,
    Custom(String),
}

impl ProtocolRoute {
    pub fn from_method(method: &str) -> Self {
        match method {
            "tools/call" => Self::ToolsCall,
            "control/challenge" => Self::ControlChallenge,
            "control/connect" => Self::ControlConnect,
            "control/ping" => Self::ControlPing,
            "control/subscribe" => Self::ControlSubscribe,
            "control/snapshot" => Self::ControlSnapshot,
            "control/events" => Self::ControlEvents,
            "presence/read" => Self::PresenceRead,
            "health/read" => Self::HealthRead,
            "session/list" => Self::SessionList,
            "session/read" => Self::SessionRead,
            "task/list" => Self::TaskList,
            "task/read" => Self::TaskRead,
            "turn/submit" => Self::TurnSubmit,
            "turn/result" => Self::TurnResult,
            "turn/stream" => Self::TurnStream,
            "approval/list" => Self::ApprovalList,
            "approval/resolve" => Self::ApprovalResolve,
            "pairing/list" => Self::PairingList,
            "pairing/resolve" => Self::PairingResolve,
            "acp/session/list" => Self::AcpSessionList,
            "acp/session/read" => Self::AcpSessionRead,
            other => Self::Custom(other.to_owned()),
        }
    }

    pub fn method(&self) -> &str {
        match self {
            Self::ToolsCall => "tools/call",
            Self::ControlChallenge => "control/challenge",
            Self::ControlConnect => "control/connect",
            Self::ControlPing => "control/ping",
            Self::ControlSubscribe => "control/subscribe",
            Self::ControlSnapshot => "control/snapshot",
            Self::ControlEvents => "control/events",
            Self::PresenceRead => "presence/read",
            Self::HealthRead => "health/read",
            Self::SessionList => "session/list",
            Self::SessionRead => "session/read",
            Self::TaskList => "task/list",
            Self::TaskRead => "task/read",
            Self::TurnSubmit => "turn/submit",
            Self::TurnResult => "turn/result",
            Self::TurnStream => "turn/stream",
            Self::ApprovalList => "approval/list",
            Self::ApprovalResolve => "approval/resolve",
            Self::PairingList => "pairing/list",
            Self::PairingResolve => "pairing/resolve",
            Self::AcpSessionList => "acp/session/list",
            Self::AcpSessionRead => "acp/session/read",
            Self::Custom(method) => method,
        }
    }

    pub fn is_standard(&self) -> bool {
        matches!(
            self,
            Self::ToolsCall
                | Self::ControlChallenge
                | Self::ControlConnect
                | Self::ControlPing
                | Self::ControlSubscribe
                | Self::ControlSnapshot
                | Self::ControlEvents
                | Self::PresenceRead
                | Self::HealthRead
                | Self::SessionList
                | Self::SessionRead
                | Self::TaskList
                | Self::TaskRead
                | Self::TurnSubmit
                | Self::TurnResult
                | Self::TurnStream
                | Self::ApprovalList
                | Self::ApprovalResolve
                | Self::PairingList
                | Self::PairingResolve
                | Self::AcpSessionList
                | Self::AcpSessionRead
        )
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
            ProtocolRoute::ControlChallenge | ProtocolRoute::ControlConnect => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: true,
                    required_capability: None,
                },
            }),
            ProtocolRoute::ControlPing
            | ProtocolRoute::ControlSubscribe
            | ProtocolRoute::ControlSnapshot
            | ProtocolRoute::ControlEvents
            | ProtocolRoute::PresenceRead
            | ProtocolRoute::HealthRead
            | ProtocolRoute::SessionList
            | ProtocolRoute::SessionRead
            | ProtocolRoute::TaskList
            | ProtocolRoute::TaskRead
            | ProtocolRoute::TurnResult
            | ProtocolRoute::TurnStream => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: false,
                    required_capability: Some(CONTROL_READ_CAPABILITY.to_owned()),
                },
            }),
            ProtocolRoute::TurnSubmit => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: false,
                    required_capability: Some(CONTROL_WRITE_CAPABILITY.to_owned()),
                },
            }),
            ProtocolRoute::ApprovalList | ProtocolRoute::ApprovalResolve => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: false,
                    required_capability: Some(CONTROL_APPROVALS_CAPABILITY.to_owned()),
                },
            }),
            ProtocolRoute::PairingList | ProtocolRoute::PairingResolve => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: false,
                    required_capability: Some(CONTROL_PAIRING_CAPABILITY.to_owned()),
                },
            }),
            ProtocolRoute::AcpSessionList | ProtocolRoute::AcpSessionRead => Ok(ResolvedRoute {
                route,
                policy: RoutePolicy {
                    allow_anonymous: false,
                    required_capability: Some(CONTROL_ACP_CAPABILITY.to_owned()),
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
        if !resolved.policy.allow_anonymous && !request.authenticated {
            return Err(RouteAuthorizationError::Unauthenticated {
                method: resolved.method().to_owned(),
            });
        }

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
    pub authenticated: bool,
    pub capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteAuthorizationDecision {
    Allow,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RouteAuthorizationError {
    #[error("unauthenticated request for method: {method}")]
    Unauthenticated { method: String },
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
    raw.trim().to_ascii_lowercase().replace(['.', '-'], "_")
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

#[doc(hidden)]
pub mod test_support;
