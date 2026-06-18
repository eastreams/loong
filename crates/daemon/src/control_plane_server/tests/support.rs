use super::*;
use crate::control_plane_device_auth::{control_plane_device_signature_message, current_time_ms};
use base64::Engine as _;
use futures_util::StreamExt;
use loong_contracts::SecretRef;
use loong_protocol::ControlPlanePairingStatus;

fn build_control_plane_router(manager: Arc<mvp::control_plane::ControlPlaneManager>) -> Router {
    super::build_control_plane_router(manager).expect("router")
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_router_with_views(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
) -> Router {
    super::serve::build_control_plane_router_with_views(manager, repository_view, acp_view)
        .expect("router")
}

fn build_control_plane_router_with_turn_runtime(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    turn_runtime: Arc<ControlPlaneTurnRuntime>,
) -> Router {
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    let exposure_policy = default_loopback_exposure_policy();
    super::serve::build_control_plane_router_with_runtime(
        manager,
        None,
        None,
        Some(turn_runtime),
        pairing_registry,
        exposure_policy,
    )
    .expect("router")
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_router_with_turn_runtime_and_views(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Arc<mvp::control_plane::ControlPlaneRepositoryView>,
    acp_view: Arc<mvp::control_plane::ControlPlaneAcpView>,
    turn_runtime: Arc<ControlPlaneTurnRuntime>,
) -> Router {
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    let exposure_policy = default_loopback_exposure_policy();
    super::serve::build_control_plane_router_with_runtime(
        manager,
        Some(repository_view),
        Some(acp_view),
        Some(turn_runtime),
        pairing_registry,
        exposure_policy,
    )
    .expect("router")
}

#[derive(Default)]
struct TestTurnBackendState {
    sink_calls: std::sync::atomic::AtomicUsize,
}

struct TestTurnBackend {
    id: &'static str,
    state: Arc<TestTurnBackendState>,
}

impl mvp::acp::AcpRuntimeBackend for TestTurnBackend {
    fn id(&self) -> &'static str {
        self.id
    }

    fn metadata(&self) -> mvp::acp::AcpBackendMetadata {
        mvp::acp::AcpBackendMetadata::new(
            self.id(),
            [
                mvp::acp::AcpCapability::SessionLifecycle,
                mvp::acp::AcpCapability::TurnExecution,
                mvp::acp::AcpCapability::TurnEventStreaming,
            ],
            "Control-plane turn backend for daemon tests",
        )
    }

    fn ensure_session<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        request: &'life2 mvp::acp::AcpSessionBootstrap,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = CliResult<mvp::acp::AcpSessionHandle>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            Ok(mvp::acp::AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("test-runtime-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("backend-{}", request.session_key)),
                agent_session_id: Some(format!("agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        })
    }

    fn run_turn<'life0, 'life1, 'life2, 'life3, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
        request: &'life3 mvp::acp::AcpTurnRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = CliResult<mvp::acp::AcpTurnResult>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        'life3: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            Ok(mvp::acp::AcpTurnResult {
                output_text: format!("echo: {}", request.input),
                state: mvp::acp::AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(mvp::acp::AcpTurnStopReason::Completed),
            })
        })
    }

    fn run_turn_with_sink<'life0, 'life1, 'life2, 'life3, 'life5, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
        request: &'life3 mvp::acp::AcpTurnRequest,
        _abort: Option<mvp::acp::AcpAbortSignal>,
        sink: Option<&'life5 dyn mvp::acp::AcpTurnEventSink>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = CliResult<mvp::acp::AcpTurnResult>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        'life3: 'async_trait,
        'life5: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            if let Some(sink) = sink {
                sink.on_event(&serde_json::json!({
                    "type": "text",
                    "content": format!("chunk:{}", request.input),
                }))?;
                sink.on_event(&serde_json::json!({
                    "type": "done",
                    "stopReason": "completed",
                }))?;
            }
            self.state
                .sink_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(mvp::acp::AcpTurnResult {
                output_text: format!("streamed: {}", request.input),
                state: mvp::acp::AcpSessionState::Ready,
                usage: Some(serde_json::json!({
                    "total_tokens": 7
                })),
                events: Vec::new(),
                stop_reason: Some(mvp::acp::AcpTurnStopReason::Completed),
            })
        })
    }

    fn cancel<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { Ok(()) })
    }

    fn close<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { Ok(()) })
    }
}

fn turn_runtime_test_config(backend_id: &str) -> mvp::config::LoongConfig {
    let mut config = mvp::config::LoongConfig::default();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.audit.mode = mvp::config::AuditMode::InMemory;
    config
}

fn seeded_turn_runtime(
    backend_id: &'static str,
    state: Arc<TestTurnBackendState>,
) -> Arc<ControlPlaneTurnRuntime> {
    mvp::acp::register_acp_backend(backend_id, {
        move || {
            Box::new(TestTurnBackend {
                id: backend_id,
                state: state.clone(),
            })
        }
    })
    .expect("register control-plane turn backend");
    let mut config = turn_runtime_test_config(backend_id);
    let temp_root = std::env::temp_dir().join(format!(
        "loong-control-plane-turn-runtime-{}-{}",
        backend_id,
        current_time_ms()
    ));
    std::fs::create_dir_all(&temp_root).expect("create control-plane turn runtime temp root");
    config.memory.sqlite_path = temp_root.join("memory.sqlite3").display().to_string();
    let resolved_path = temp_root.join("config.toml");
    mvp::config::write(
        Some(resolved_path.to_str().expect("utf8 config path")),
        &config,
        true,
    )
    .expect("write control-plane turn runtime config");
    let acp_manager = Arc::new(mvp::acp::AcpSessionManager::default());
    Arc::new(ControlPlaneTurnRuntime::with_manager(
        resolved_path,
        config,
        acp_manager,
    ))
}

fn remote_control_plane_config(shared_token: &str) -> mvp::config::LoongConfig {
    let mut config = mvp::config::LoongConfig::default();
    config.control_plane.allow_remote = true;
    config.control_plane.shared_token = Some(SecretRef::Inline(shared_token.to_owned()));
    config
}

fn non_loopback_bind_addr() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 4317))
}

fn build_remote_control_plane_router(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    shared_token: &str,
) -> Router {
    let config = remote_control_plane_config(shared_token);
    let bind_addr = non_loopback_bind_addr();
    let exposure_policy =
        build_control_plane_exposure_policy(bind_addr, Some(&config)).expect("policy");
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    super::serve::build_control_plane_router_with_runtime(
        manager,
        None,
        None,
        None,
        pairing_registry,
        exposure_policy,
    )
    .expect("router")
}

async fn connect_token(
    router: &Router,
    scopes: std::collections::BTreeSet<ControlPlaneScope>,
) -> String {
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: None,
    };

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let connect: ControlPlaneConnectResponse = serde_json::from_slice(&body).expect("connect json");
    connect.connection_token
}

fn bearer_request(method: &str, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method(method)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request")
}

async fn issue_challenge(router: &Router) -> ControlPlaneChallengeResponse {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/challenge")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("challenge response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&body).expect("challenge json")
}

fn signed_device_for_request(
    client_id: &str,
    role: ControlPlaneRole,
    scopes: std::collections::BTreeSet<ControlPlaneScope>,
    challenge: &ControlPlaneChallengeResponse,
) -> loong_protocol::ControlPlaneDeviceIdentity {
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let device_template = loong_protocol::ControlPlaneDeviceIdentity {
        device_id: "device-1".to_owned(),
        public_key: String::new(),
        signature: String::new(),
        signed_at_ms: challenge.issued_at_ms,
        nonce: challenge.nonce.clone(),
    };
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: client_id.to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device_template.clone()),
    };
    let message = control_plane_device_signature_message(&request, &device_template);
    let signature = signing_key.sign(&message);
    loong_protocol::ControlPlaneDeviceIdentity {
        device_id: "device-1".to_owned(),
        public_key: base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes()),
        signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
        signed_at_ms: challenge.issued_at_ms,
        nonce: challenge.nonce.clone(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn isolated_memory_config(test_name: &str) -> mvp::session::store::SessionStoreConfig {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ISOLATED_MEMORY_CONFIG_ID: AtomicU64 = AtomicU64::new(1);
    let nonce = NEXT_ISOLATED_MEMORY_CONFIG_ID.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "loong-control-plane-server-{test_name}-{}-{nonce}",
        std::process::id(),
    ));
    let _ = std::fs::create_dir_all(&base);
    let db_path = base.join("memory.sqlite3");
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(base.join("memory.sqlite3-wal"));
    let _ = std::fs::remove_file(base.join("memory.sqlite3-shm"));
    mvp::session::store::SessionStoreConfig {
        sqlite_path: Some(db_path),
        runtime_config: None,
    }
}

#[cfg(feature = "memory-sqlite")]
fn seeded_repository_view(test_name: &str) -> Arc<mvp::control_plane::ControlPlaneRepositoryView> {
    let config = isolated_memory_config(test_name);
    let repo = mvp::session::repository::SessionRepository::new(&config).expect("repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: serde_json::json!({
            "task": "research control plane parity",
            "label": "Child",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 90,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["read"],
                "workspace_root": "/tmp/loong/control-plane/child-session",
                "kernel_bound": false,
                "runtime_narrowing": {}
            },
            "runtime_self_continuity": {
                "runtime_self": {
                    "standing_instructions": ["Stay concise."],
                    "tool_usage_policy": ["Prefer visible evidence."],
                    "soul_guidance": ["Keep continuity explicit."],
                    "identity_context": ["# Identity\n- Name: Child"],
                    "user_context": ["Operator prefers concise technical summaries."]
                },
                "resolved_identity": {
                    "source": "workspace_self",
                    "content": "# Identity\n- Name: Child"
                },
                "session_profile_projection": "## Session Profile\nOperator prefers concise technical summaries."
            }
        }),
    })
    .expect("append child event");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-visible".to_owned(),
        session_id: "child-session".to_owned(),
        turn_id: "turn-visible".to_owned(),
        tool_call_id: "call-visible".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-visible",
        }),
    })
    .expect("create visible approval");
    repo.upsert_session_tool_policy(mvp::session::repository::NewSessionToolPolicyRecord {
        session_id: "child-session".to_owned(),
        requested_tool_ids: vec!["read".to_owned()],
        runtime_narrowing: mvp::tools::runtime_config::ToolRuntimeNarrowing::default(),
    })
    .expect("create visible tool policy");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create hidden root");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-hidden".to_owned(),
        session_id: "hidden-root".to_owned(),
        turn_id: "turn-hidden".to_owned(),
        tool_call_id: "call-hidden".to_owned(),
        tool_name: "delegate_async".to_owned(),
        approval_key: "tool:delegate_async".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate_async",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-hidden",
        }),
    })
    .expect("create hidden approval");

    Arc::new(mvp::control_plane::ControlPlaneRepositoryView::new(
        config,
        mvp::config::ToolConfig::default(),
        "root-session",
    ))
}

#[cfg(feature = "memory-sqlite")]
fn seeded_control_plane_views(
    test_name: &str,
) -> (
    Arc<mvp::control_plane::ControlPlaneRepositoryView>,
    Arc<mvp::control_plane::ControlPlaneAcpView>,
) {
    let memory_config = isolated_memory_config(test_name);
    let repo =
        mvp::session::repository::SessionRepository::new(&memory_config).expect("repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: serde_json::json!({
            "status": "started",
        }),
    })
    .expect("append child event");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-visible".to_owned(),
        session_id: "child-session".to_owned(),
        turn_id: "turn-visible".to_owned(),
        tool_call_id: "call-visible".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-visible",
        }),
    })
    .expect("create visible approval");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create hidden root");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-hidden".to_owned(),
        session_id: "hidden-root".to_owned(),
        turn_id: "turn-hidden".to_owned(),
        tool_call_id: "call-hidden".to_owned(),
        tool_name: "delegate_async".to_owned(),
        approval_key: "tool:delegate_async".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate_async",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-hidden",
        }),
    })
    .expect("create hidden approval");

    let mut config = mvp::config::LoongConfig::default();
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    config.acp.enabled = true;

    let store = mvp::acp::AcpSqliteSessionStore::new(Some(config.memory.resolved_sqlite_path()));
    mvp::acp::AcpSessionStore::upsert(
        &store,
        mvp::acp::AcpSessionMetadata {
            session_key: "agent:codex:child-session".to_owned(),
            conversation_id: Some("conversation-visible".to_owned()),
            binding: Some(mvp::acp::AcpSessionBindingScope {
                route_session_id: "child-session".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc-visible".to_owned()),
                participant_id: None,
                thread_id: Some("thread-visible".to_owned()),
            }),
            activation_origin: Some(mvp::acp::AcpRoutingOrigin::ExplicitRequest),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-visible".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-visible".to_owned()),
            agent_session_id: Some("agent-visible".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Interactive),
            state: mvp::acp::AcpSessionState::Ready,
            last_activity_ms: 100,
            last_error: None,
        },
    )
    .expect("seed visible ACP session");
    mvp::acp::AcpSessionStore::upsert(
        &store,
        mvp::acp::AcpSessionMetadata {
            session_key: "agent:codex:hidden-root".to_owned(),
            conversation_id: Some("conversation-hidden".to_owned()),
            binding: Some(mvp::acp::AcpSessionBindingScope {
                route_session_id: "hidden-root".to_owned(),
                channel_id: Some("telegram".to_owned()),
                account_id: None,
                conversation_id: Some("hidden".to_owned()),
                participant_id: None,
                thread_id: None,
            }),
            activation_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticDispatch),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-hidden".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-hidden".to_owned()),
            agent_session_id: Some("agent-hidden".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Review),
            state: mvp::acp::AcpSessionState::Busy,
            last_activity_ms: 200,
            last_error: Some("hidden".to_owned()),
        },
    )
    .expect("seed hidden ACP session");

    (
        Arc::new(mvp::control_plane::ControlPlaneRepositoryView::new(
            memory_config,
            mvp::config::ToolConfig::default(),
            "root-session",
        )),
        Arc::new(mvp::control_plane::ControlPlaneAcpView::new(
            config,
            "root-session",
        )),
    )
}
