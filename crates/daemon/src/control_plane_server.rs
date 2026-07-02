use std::collections::VecDeque;
use std::convert::Infallible;
#[cfg(test)]
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::Router;
use axum::extract::Query;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::stream::{self, Stream};
use kernel::Capability;
use loong_protocol::{
    CONTROL_PLANE_PROTOCOL_VERSION, ControlPlaneAcpBindingScope, ControlPlaneAcpRoutingOrigin,
    ControlPlaneAcpSessionListResponse, ControlPlaneAcpSessionMetadata, ControlPlaneAcpSessionMode,
    ControlPlaneAcpSessionReadResponse, ControlPlaneAcpSessionState, ControlPlaneAcpSessionStatus,
    ControlPlaneApprovalDecision, ControlPlaneApprovalListResponse,
    ControlPlaneApprovalRequestStatus, ControlPlaneApprovalSummary, ControlPlaneChallengeResponse,
    ControlPlaneConnectErrorCode, ControlPlaneConnectErrorResponse, ControlPlaneConnectRequest,
    ControlPlaneConnectResponse, ControlPlaneEventEnvelope, ControlPlaneEventName,
    ControlPlanePairingListResponse, ControlPlanePairingRequestSummary,
    ControlPlanePairingResolveRequest, ControlPlanePairingResolveResponse, ControlPlanePolicy,
    ControlPlanePrincipal, ControlPlaneRecentEventsResponse, ControlPlaneScope,
    ControlPlaneSessionEvent, ControlPlaneSessionKind, ControlPlaneSessionListResponse,
    ControlPlaneSessionObservation, ControlPlaneSessionReadResponse, ControlPlaneSessionState,
    ControlPlaneSessionSummary, ControlPlaneSessionTerminalOutcome, ControlPlaneSessionWorkflow,
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
mod mapping_acp;
use self::mapping_acp::*;
mod mapping_pairing;
use self::mapping_pairing::*;
mod mapping_snapshot;
use self::mapping_snapshot::*;
mod mapping_task_approval;
use self::mapping_task_approval::*;
mod mapping_session;
use self::mapping_session::*;
mod connect;
pub(crate) mod pairing_projection;
use self::connect::*;
mod connect_auth;
use self::connect_auth::*;
mod control;
use self::control::*;
mod events;
use self::events::*;
mod resources;
mod resources_pairing_acp;
use self::resources_pairing_acp::*;
mod resources_session_task_approval;
use self::resources_session_task_approval::*;
mod serve;
pub use self::serve::{build_control_plane_router, run_control_plane_serve_cli};
mod support;
use self::support::*;
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

#[cfg(test)]
mod tests;
