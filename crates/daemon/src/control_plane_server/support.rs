use super::*;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::AtomicU64;

use kernel::{
    CapabilityToken, ExecutionPlane, InMemoryAuditSink, Kernel, PlaneTier, VerticalPackManifest,
};
use loong_spec::{SpecContextFactory, SpecExecutionContext};

#[derive(Debug, Clone)]
pub(super) struct ControlPlaneExposurePolicy {
    pub(super) bind_addr: SocketAddr,
    pub(super) shared_token: Option<String>,
}

impl ControlPlaneExposurePolicy {
    pub(super) fn requires_remote_auth(&self) -> bool {
        !self.bind_addr.ip().is_loopback()
    }
}

pub(super) fn default_loopback_exposure_policy() -> ControlPlaneExposurePolicy {
    ControlPlaneExposurePolicy {
        bind_addr: default_control_plane_bind_addr(0),
        shared_token: None,
    }
}

pub(super) struct ControlPlaneKernelAuthority {
    kernel: Kernel<SpecContextFactory>,
    pack: VerticalPackManifest,
    _audit: Arc<InMemoryAuditSink>,
    token_bindings: std::sync::RwLock<std::collections::BTreeMap<String, CapabilityToken>>,
}

#[derive(Clone)]
pub(super) struct ControlPlaneHttpState {
    pub(super) manager: Arc<mvp::control_plane::ControlPlaneManager>,
    pub(super) connection_counter: Arc<AtomicU64>,
    pub(super) connection_registry: Arc<mvp::control_plane::ControlPlaneConnectionRegistry>,
    pub(super) challenge_registry: Arc<mvp::control_plane::ControlPlaneChallengeRegistry>,
    pub(super) pairing_registry: Arc<mvp::control_plane::ControlPlanePairingRegistry>,
    pub(super) kernel_authority: Arc<ControlPlaneKernelAuthority>,
    pub(super) exposure_policy: Arc<ControlPlaneExposurePolicy>,
    #[cfg(feature = "memory-sqlite")]
    pub(super) repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    #[cfg(feature = "memory-sqlite")]
    pub(super) acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
    pub(super) turn_runtime: Option<Arc<ControlPlaneTurnRuntime>>,
}

pub(super) struct ControlPlaneSubscribeStreamState {
    pub(super) manager: Arc<mvp::control_plane::ControlPlaneManager>,
    pub(super) pending_events: VecDeque<mvp::control_plane::ControlPlaneEventRecord>,
    pub(super) receiver:
        tokio::sync::broadcast::Receiver<mvp::control_plane::ControlPlaneEventRecord>,
    pub(super) last_seq: u64,
    pub(super) include_targeted: bool,
}

pub(super) struct ControlPlaneTurnStreamState {
    pub(super) turn_id: String,
    pub(super) registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    pub(super) pending_events: VecDeque<mvp::control_plane::ControlPlaneTurnEventRecord>,
    pub(super) receiver:
        tokio::sync::broadcast::Receiver<mvp::control_plane::ControlPlaneTurnEventRecord>,
    pub(super) last_seq: u64,
}

/// Shared dependencies for ad-hoc turn execution launched from the control
/// plane HTTP surface.
///
/// This is intentionally narrower than the full control-plane router state: it
/// keeps just enough config, ACP ownership, and per-turn event registry state
/// to materialize `AgentRuntime` turns on demand.
pub(super) struct ControlPlaneTurnRuntime {
    pub(super) resolved_path: std::path::PathBuf,
    pub(super) config: mvp::config::LoongConfig,
    pub(super) acp_manager: Arc<mvp::acp::AcpSessionManager>,
    pub(super) registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
}

pub(super) struct ControlPlaneTurnEventForwarder {
    pub(super) manager: Arc<mvp::control_plane::ControlPlaneManager>,
    pub(super) registry: Arc<mvp::control_plane::ControlPlaneTurnRegistry>,
    pub(super) turn_id: String,
}

impl ControlPlaneKernelAuthority {
    pub(super) fn new() -> Result<Self, String> {
        let kernel_with_audit = Kernel::new_with_in_memory_audit();
        let mut kernel = kernel_with_audit.0;
        let audit = kernel_with_audit.1;
        let pack = control_plane_pack();
        let register_result = kernel.register_pack(pack.clone());
        register_result
            .map_err(|error| format!("control-plane pack registration failed: {error}"))?;
        Ok(Self {
            kernel,
            pack,
            _audit: audit,
            token_bindings: std::sync::RwLock::new(std::collections::BTreeMap::new()),
        })
    }

    pub(super) fn issue_scoped_token(
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

    pub(super) async fn authorize(
        &self,
        connection_token: &str,
        operation: &str,
        capabilities: &std::collections::BTreeSet<Capability>,
    ) -> Result<(), String> {
        let token = {
            let token_bindings = self
                .token_bindings
                .read()
                .unwrap_or_else(|error| error.into_inner());
            token_bindings
                .get(connection_token)
                .cloned()
                .ok_or_else(|| "missing control-plane kernel token binding".to_owned())?
        };
        let policy_context =
            SpecExecutionContext::new(&self.pack, &token, self.kernel.now_epoch_s(), None);
        self.kernel
            .authorize_operation(
                CONTROL_PLANE_PACK_ID,
                &token,
                ExecutionPlane::Runtime,
                PlaneTier::Core,
                CONTROL_PLANE_PRIMARY_ADAPTER,
                None,
                operation,
                capabilities,
                &policy_context,
            )
            .await
            .map_err(|error| format!("control-plane kernel authorization failed: {error}"))
    }

    pub(super) fn remove_binding(&self, connection_token: &str) {
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

pub(super) fn default_control_plane_bind_addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

pub(super) fn resolve_control_plane_bind_addr(
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

pub(super) fn build_control_plane_exposure_policy(
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
pub(super) fn ensure_turn_session_visible(
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
pub(super) fn ensure_turn_session_visible(
    _state: &ControlPlaneHttpState,
    _session_id: &str,
) -> Option<Response> {
    None
}

impl ControlPlaneTurnRuntime {
    /// Build a control-plane turn runtime from a config snapshot and the shared
    /// ACP manager that should back all HTTP-triggered turns for that process.
    pub(super) fn new(
        resolved_path: std::path::PathBuf,
        config: mvp::config::LoongConfig,
    ) -> Result<Self, String> {
        let acp_manager = mvp::acp::acquire_shared_acp_session_manager(&config)?;
        Ok(Self::with_manager(resolved_path, config, acp_manager))
    }

    /// Test/advanced constructor that reuses an already prepared ACP manager
    /// while still allocating a fresh turn registry for this runtime shell.
    pub(super) fn with_manager(
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
