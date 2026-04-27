use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::Router;
use axum::extract::Query;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::stream::{self, Stream};
use kernel::{
    Capability, CapabilityToken, ExecutionPlane, InMemoryAuditSink, LoongKernel, PlaneTier,
    StaticPolicyEngine, VerticalPackManifest,
};
use loong_protocol::{
    CONTROL_PLANE_PROTOCOL_VERSION, ControlPlaneAcpBindingScope, ControlPlaneAcpRoutingOrigin,
    ControlPlaneAcpSessionListResponse, ControlPlaneAcpSessionMetadata, ControlPlaneAcpSessionMode,
    ControlPlaneAcpSessionReadResponse, ControlPlaneAcpSessionState, ControlPlaneAcpSessionStatus,
    ControlPlaneApprovalDecision, ControlPlaneApprovalListResponse,
    ControlPlaneApprovalRequestStatus, ControlPlaneApprovalSummary, ControlPlaneChallengeResponse,
    ControlPlaneConnectErrorCode, ControlPlaneConnectErrorResponse, ControlPlaneConnectRequest,
    ControlPlaneConnectResponse, ControlPlaneEventEnvelope, ControlPlaneEventName,
    ControlPlanePairingListResponse, ControlPlanePairingRequestSummary,
    ControlPlanePairingResolveRequest, ControlPlanePairingResolveResponse,
    ControlPlanePairingStatus, ControlPlanePolicy, ControlPlanePrincipal,
    ControlPlaneRecentEventsResponse, ControlPlaneScope, ControlPlaneSessionEvent,
    ControlPlaneSessionKind, ControlPlaneSessionListResponse, ControlPlaneSessionObservation,
    ControlPlaneSessionReadResponse, ControlPlaneSessionState, ControlPlaneSessionSummary,
    ControlPlaneSessionTerminalOutcome, ControlPlaneSessionWorkflow,
    ControlPlaneSessionWorkflowBinding, ControlPlaneSessionWorkflowBindingWorktree,
    ControlPlaneSessionWorkflowContinuity, ControlPlaneSnapshot, ControlPlaneSnapshotResponse,
    ControlPlaneStateVersion, ControlPlaneTaskListResponse, ControlPlaneTaskReadResponse,
    ControlPlaneTaskSummary, ControlPlaneTurnEventEnvelope, ControlPlaneTurnResultResponse,
    ControlPlaneTurnStatus, ControlPlaneTurnSubmitRequest, ControlPlaneTurnSubmitResponse,
    ControlPlaneTurnSummary, ProtocolRouter,
};
use serde::Deserialize;

use crate::{CliResult, mvp};

mod mapping;
use self::mapping::*;
mod connect;
use self::connect::*;
mod control;
use self::control::*;
mod events;
use self::events::*;
mod resources;
use self::resources::*;
mod serve;
pub use self::serve::{build_control_plane_router, run_control_plane_serve_cli};
mod turn;
use self::turn::*;

#[cfg(test)]
use axum::body::{Body, to_bytes};
#[cfg(test)]
use axum::http::Request;
#[cfg(test)]
use ed25519_dalek::{Signer, SigningKey};
#[cfg(test)]
use loong_protocol::{ControlPlaneClientIdentity, ControlPlaneRole};
#[cfg(test)]
use tower::ServiceExt;

const CONTROL_PLANE_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const CONTROL_PLANE_MAX_BUFFERED_BYTES: usize = 256 * 1024;
const CONTROL_PLANE_TICK_INTERVAL_MS: u64 = 15_000;
const CONTROL_PLANE_DEFAULT_EVENT_LIMIT: usize = 50;
const CONTROL_PLANE_DEFAULT_LIST_LIMIT: usize = 50;
const CONTROL_PLANE_DEFAULT_SESSION_RECENT_LIMIT: usize = 20;
const CONTROL_PLANE_DEFAULT_SESSION_TAIL_LIMIT: usize = 50;
const CONTROL_PLANE_CHALLENGE_MAX_FUTURE_SKEW_MS: u64 = 10_000;
const CONTROL_PLANE_PACK_ID: &str = "control-plane";
const CONTROL_PLANE_PACK_DOMAIN: &str = "control";
const CONTROL_PLANE_PACK_VERSION: &str = "1.0.0";
const CONTROL_PLANE_PRIMARY_ADAPTER: &str = "control-plane";
const CONTROL_PLANE_KEEPALIVE_TEXT: &str = "keep-alive";
const CONTROL_PLANE_REMOTE_BOOTSTRAP_SCOPES: [ControlPlaneScope; 2] = [
    ControlPlaneScope::OperatorRead,
    ControlPlaneScope::OperatorPairing,
];

#[derive(Debug, Clone)]
struct ControlPlaneExposurePolicy {
    bind_addr: SocketAddr,
    shared_token: Option<String>,
}

impl ControlPlaneExposurePolicy {
    fn requires_remote_auth(&self) -> bool {
        !self.bind_addr.ip().is_loopback()
    }
}

fn default_loopback_exposure_policy() -> ControlPlaneExposurePolicy {
    ControlPlaneExposurePolicy {
        bind_addr: default_control_plane_bind_addr(0),
        shared_token: None,
    }
}

struct ControlPlaneKernelAuthority {
    kernel: LoongKernel<StaticPolicyEngine>,
    _audit: Arc<InMemoryAuditSink>,
    token_bindings: std::sync::RwLock<std::collections::BTreeMap<String, CapabilityToken>>,
}

#[derive(Clone)]
struct ControlPlaneHttpState {
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    connection_counter: Arc<AtomicU64>,
    connection_registry: Arc<mvp::control_plane::ControlPlaneConnectionRegistry>,
    challenge_registry: Arc<mvp::control_plane::ControlPlaneChallengeRegistry>,
    pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    kernel_authority: Arc<ControlPlaneKernelAuthority>,
    exposure_policy: Arc<ControlPlaneExposurePolicy>,
    #[cfg(feature = "memory-sqlite")]
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    #[cfg(feature = "memory-sqlite")]
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    turn_runtime: Option<Arc<ControlPlaneTurnRuntime>>,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    include_targeted: bool,
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SessionListQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct SessionReadQuery {
    session_id: String,
    #[serde(default)]
    recent_event_limit: Option<usize>,
    #[serde(default)]
    tail_after_id: Option<i64>,
    #[serde(default)]
    tail_page_limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TaskListQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct TaskReadQuery {
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalListQuery {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AcpSessionListQuery {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AcpSessionReadQuery {
    session_key: String,
}

#[derive(Debug, Deserialize)]
struct PairingListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SubscribeQuery {
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default)]
    include_targeted: bool,
}

#[derive(Debug, Deserialize)]
struct TurnResultQuery {
    turn_id: String,
}

#[derive(Debug, Deserialize)]
struct TurnStreamQuery {
    turn_id: String,
    #[serde(default)]
    after_seq: Option<u64>,
}

struct ControlPlaneSubscribeStreamState {
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    pending_events: VecDeque<mvp::control_plane::ControlPlaneEventRecord>,
    receiver: tokio::sync::broadcast::Receiver<mvp::control_plane::ControlPlaneEventRecord>,
    last_seq: u64,
    include_targeted: bool,
}

struct ControlPlaneTurnStreamState {
    turn_id: String,
    registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    pending_events: VecDeque<mvp::control_plane::ControlPlaneTurnEventRecord>,
    receiver: tokio::sync::broadcast::Receiver<mvp::control_plane::ControlPlaneTurnEventRecord>,
    last_seq: u64,
}

/// Shared dependencies for ad-hoc turn execution launched from the control
/// plane HTTP surface.
///
/// This is intentionally narrower than the full control-plane router state: it
/// keeps just enough config, ACP ownership, and per-turn event registry state
/// to materialize `AgentRuntime` turns on demand.
struct ControlPlaneTurnRuntime {
    resolved_path: std::path::PathBuf,
    config: mvp::config::LoongConfig,
    acp_manager: Arc<mvp::acp::AcpSessionManager>,
    registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
}

struct ControlPlaneTurnEventForwarder {
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    turn_id: String,
}

impl ControlPlaneKernelAuthority {
    fn new() -> Result<Self, String> {
        let kernel_with_audit =
            LoongKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
        let mut kernel = kernel_with_audit.0;
        let audit = kernel_with_audit.1;
        let pack = control_plane_pack();
        let register_result = kernel.register_pack(pack);
        register_result
            .map_err(|error| format!("control-plane pack registration failed: {error}"))?;
        Ok(Self {
            kernel,
            _audit: audit,
            token_bindings: std::sync::RwLock::new(std::collections::BTreeMap::new()),
        })
    }

    fn issue_scoped_token(
        &self,
        connection_token: &str,
        agent_id: &str,
        capabilities: &std::collections::BTreeSet<Capability>,
    ) -> Result<(), String> {
        let token = self
            .kernel
            .issue_scoped_token(CONTROL_PLANE_PACK_ID, agent_id, capabilities, 15 * 60)
            .map_err(|error| format!("control-plane kernel token issuance failed: {error}"))?;
        let mut token_bindings = self
            .token_bindings
            .write()
            .unwrap_or_else(|error| error.into_inner());
        token_bindings.insert(connection_token.to_owned(), token);
        Ok(())
    }

    fn authorize(
        &self,
        connection_token: &str,
        operation: &str,
        capabilities: &std::collections::BTreeSet<Capability>,
    ) -> Result<(), String> {
        let token_bindings = self
            .token_bindings
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let token = token_bindings
            .get(connection_token)
            .ok_or_else(|| "missing control-plane kernel token binding".to_owned())?;
        self.kernel
            .authorize_operation(
                CONTROL_PLANE_PACK_ID,
                token,
                ExecutionPlane::Runtime,
                PlaneTier::Core,
                CONTROL_PLANE_PRIMARY_ADAPTER,
                None,
                operation,
                capabilities,
            )
            .map_err(|error| format!("control-plane kernel authorization failed: {error}"))
    }

    fn remove_binding(&self, connection_token: &str) {
        let mut token_bindings = self
            .token_bindings
            .write()
            .unwrap_or_else(|error| error.into_inner());
        token_bindings.remove(connection_token);
    }
}

fn control_plane_pack() -> VerticalPackManifest {
    let granted_capabilities = std::collections::BTreeSet::from([
        Capability::ControlRead,
        Capability::ControlWrite,
        Capability::ControlApprovals,
        Capability::ControlPairing,
        Capability::ControlAcp,
    ]);
    let default_route = kernel::ExecutionRoute {
        harness_kind: kernel::HarnessKind::EmbeddedPi,
        adapter: None,
    };
    let allowed_connectors = std::collections::BTreeSet::new();
    let metadata = std::collections::BTreeMap::new();
    VerticalPackManifest {
        pack_id: CONTROL_PLANE_PACK_ID.to_owned(),
        domain: CONTROL_PLANE_PACK_DOMAIN.to_owned(),
        version: CONTROL_PLANE_PACK_VERSION.to_owned(),
        default_route,
        allowed_connectors,
        granted_capabilities,
        metadata,
    }
}

fn default_control_plane_bind_addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

fn resolve_control_plane_bind_addr(
    bind_override: Option<&str>,
    port: u16,
) -> Result<SocketAddr, String> {
    let Some(raw_bind_addr) = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(default_control_plane_bind_addr(port));
    };
    raw_bind_addr.parse::<SocketAddr>().map_err(|error| {
        format!("parse control-plane bind address `{raw_bind_addr}` failed: {error}")
    })
}

fn build_control_plane_exposure_policy(
    bind_addr: SocketAddr,
    config: Option<&mvp::config::LoongConfig>,
) -> Result<ControlPlaneExposurePolicy, String> {
    let is_loopback = bind_addr.ip().is_loopback();
    if is_loopback {
        return Ok(ControlPlaneExposurePolicy {
            bind_addr,
            shared_token: None,
        });
    }

    let Some(config) = config else {
        return Err(
            "non-loopback control-plane bind requires --config with control_plane.allow_remote=true"
                .to_owned(),
        );
    };

    if !config.control_plane.allow_remote {
        return Err(
            "non-loopback control-plane bind requires control_plane.allow_remote=true".to_owned(),
        );
    }

    let shared_token = config.control_plane.resolved_shared_token()?;
    let Some(shared_token) = shared_token else {
        return Err(
            "non-loopback control-plane bind requires control_plane.shared_token".to_owned(),
        );
    };

    Ok(ControlPlaneExposurePolicy {
        bind_addr,
        shared_token: Some(shared_token),
    })
}

#[cfg(feature = "memory-sqlite")]
fn ensure_turn_session_visible(
    state: &ControlPlaneHttpState,
    session_id: &str,
) -> Option<Response> {
    let repository_view = state.repository_view.as_ref()?;
    match repository_view.ensure_visible_session_id(session_id) {
        Ok(()) => None,
        Err(error) if error == "control_plane_session_id_missing" => {
            Some(error_response(StatusCode::BAD_REQUEST, error))
        }
        Err(error) if error.starts_with("visibility_denied:") => {
            Some(error_response(StatusCode::FORBIDDEN, error))
        }
        Err(error) => Some(error_response(StatusCode::INTERNAL_SERVER_ERROR, error)),
    }
}

#[cfg(not(feature = "memory-sqlite"))]
fn ensure_turn_session_visible(
    _state: &ControlPlaneHttpState,
    _session_id: &str,
) -> Option<Response> {
    None
}

impl ControlPlaneTurnRuntime {
    /// Build a control-plane turn runtime from a config snapshot and the shared
    /// ACP manager that should back all HTTP-triggered turns for that process.
    fn new(
        resolved_path: std::path::PathBuf,
        config: mvp::config::LoongConfig,
    ) -> Result<Self, String> {
        let acp_manager = mvp::acp::shared_acp_session_manager(&config)?;
        Ok(Self::with_manager(resolved_path, config, acp_manager))
    }

    /// Test/advanced constructor that reuses an already prepared ACP manager
    /// while still allocating a fresh turn registry for this runtime shell.
    fn with_manager(
        resolved_path: std::path::PathBuf,
        config: mvp::config::LoongConfig,
        acp_manager: Arc<mvp::acp::AcpSessionManager>,
    ) -> Self {
        Self {
            resolved_path,
            config,
            acp_manager,
            registry: Arc::new(mvp::control_plane::ControlPlaneTurnRegistry::new()),
        }
    }
}

impl mvp::acp::AcpTurnEventSink for ControlPlaneTurnEventForwarder {
    fn on_event(&self, event: &serde_json::Value) -> CliResult<()> {
        let recorded_event = self
            .registry
            .record_runtime_event(self.turn_id.as_str(), event.clone())?;
        let payload = map_turn_event_payload(&recorded_event);
        let _ = self.manager.record_acp_turn_event(payload, true);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
